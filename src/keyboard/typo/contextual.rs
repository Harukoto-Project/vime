use std::{
    io::{self, BufRead, BufReader, Write},
    ops::Range,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::AtomicU8,
        mpsc::{Receiver, RecvTimeoutError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{Value, json};
use vime::settings::JapaneseTypoSettings;

use super::{Snapshot, Suggestion, TypoSettings, Worker};

const SCRIPT: &str = include_str!("../../../tools/japanese_typo_worker.py");
const CREATE_NO_WINDOW: u32 = 0x08000000;
const START_TIMEOUT: Duration = Duration::from_secs(45);
const DETECT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const TARGET: Duration = Duration::from_millis(125);
pub(super) const BUDGET: Duration = Duration::from_millis(750);

#[derive(Deserialize)]
struct Hit {
    start: usize,
    end: usize,
    alternatives: Vec<String>,
}

pub(super) struct Detector {
    child: Child,
    input: ChildStdin,
    replies: Receiver<io::Result<String>>,
    ready: bool,
    start_deadline: Instant,
}

impl Detector {
    pub(super) fn start(app_directory: &Path) -> io::Result<Option<Self>> {
        let Some(directory) = vime::japanese_typo::model_directory(app_directory) else {
            return Ok(None);
        };
        let adapter = std::env::var("VIME_JAPANESE_TYPO_ADAPTER").unwrap_or_else(|_| "0".into());
        let mut child = Command::new(directory.join("venv/Scripts/python.exe"))
            .args(["-I", "-X", "utf8", "-c", SCRIPT])
            .arg(&directory)
            .arg(adapter)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                io::Error::new(error.kind(), format!("Japanese typo worker: {error}"))
            })?;
        let input = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, replies) = sync_channel(1);
        let worker = Self {
            child,
            input,
            replies,
            ready: false,
            start_deadline: Instant::now() + START_TIMEOUT,
        };
        thread::Builder::new()
            .name("vime-japanese-typo".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Some(worker))
    }

    fn receive(&self, timeout: Duration) -> io::Result<Value> {
        let line = match self.replies.recv_timeout(timeout) {
            Ok(line) => line?,
            Err(RecvTimeoutError::Timeout) => return Err(io::ErrorKind::TimedOut.into()),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other(
                    "Japanese typo worker exited; check its Python dependencies",
                ));
            }
        };
        let value: Value = serde_json::from_str(&line)?;
        if let Some(error) = value["error"].as_str() {
            return Err(io::Error::other(format!("Japanese typo worker: {error}")));
        }
        Ok(value)
    }

    pub(super) fn ready(&mut self) -> io::Result<()> {
        if !self.ready {
            let response = self.receive(
                self.start_deadline
                    .saturating_duration_since(Instant::now()),
            )?;
            if response["ready"] != true {
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.ready = true;
        }
        Ok(())
    }

    fn detect(
        &mut self,
        text: &str,
        caret: usize,
        settings: &JapaneseTypoSettings,
    ) -> io::Result<Vec<Hit>> {
        serde_json::to_writer(
            &mut self.input,
            &json!({
                "text": text,
                "caret": caret,
                "threshold": settings.probability_threshold,
                "count": settings.candidates,
                "max_positions": settings.max_positions,
            }),
        )?;
        self.input.write_all(b"\n")?;
        let mut response = self.receive(DETECT_TIMEOUT)?;
        Ok(serde_json::from_value(response["hits"].take())?)
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn window(snapshot: &Snapshot, radius: usize) -> Range<usize> {
    let start = snapshot.text[..snapshot.caret]
        .char_indices()
        .rev()
        .nth(radius - 1)
        .map_or(0, |(index, _)| index);
    let end = snapshot.text[snapshot.caret..]
        .char_indices()
        .nth(radius)
        .map_or(snapshot.text.len(), |(index, _)| snapshot.caret + index);
    start..end
}

fn batch(text: &str, hits: &[Hit]) -> Value {
    let mut questions = serde_json::Map::new();
    let mut state = text.to_owned();
    let mut spans: Vec<_> = hits.iter().enumerate().collect();
    spans.sort_unstable_by_key(|(_, hit)| std::cmp::Reverse(hit.start));
    for (index, hit) in spans {
        state.insert_str(hit.end, &format!("[/{index}]"));
        state.insert_str(hit.start, &format!("[{index}]"));
    }
    for (index, hit) in hits.iter().enumerate() {
        let original = &text[hit.start..hit.end];
        let mut criteria = json!({"keep": original});
        for (choice, replacement) in hit.alternatives.iter().enumerate() {
            criteria[format!("fix_{choice}")] = json!(replacement);
        }
        questions.insert(format!("position_{index}"), json!({
            "type": "choice",
            "instructions": format!(
                "Which option makes the Japanese sentence correct in [{index}]...[/{index}]? Keep valid or unfinished text.",
            ),
            "criteria": criteria,
        }));
    }
    json!({"state": state, "questions": questions})
}

fn ranked(
    response: &Value,
    hits: Vec<Hit>,
    offset: usize,
    count: usize,
) -> io::Result<Vec<Suggestion>> {
    let mut ranked = Vec::new();
    for (index, hit) in hits.into_iter().enumerate() {
        let answer = &response["answers"][format!("position_{index}")];
        let probability = |choice: &str| {
            answer["probabilities"][choice]
                .as_f64()
                .filter(|p| (0.0..=1.0).contains(p))
                .ok_or(io::ErrorKind::InvalidData)
        };
        let keep = probability("keep")?;
        for (choice, word) in hit.alternatives.into_iter().enumerate() {
            let score = probability(&format!("fix_{choice}"))?;
            if score > keep {
                ranked.push((
                    score,
                    Suggestion {
                        word,
                        range: offset + hit.start..offset + hit.end,
                    },
                ));
            }
        }
    }
    ranked.sort_by(|(a, _), (b, _)| b.total_cmp(a));
    let mut suggestions = Vec::<Suggestion>::new();
    for (_, suggestion) in ranked {
        if !suggestions
            .iter()
            .any(|existing| existing.word == suggestion.word)
        {
            suggestions.push(suggestion);
            if suggestions.len() == count {
                break;
            }
        }
    }
    Ok(suggestions)
}

#[derive(Default)]
pub(super) struct Timings {
    detector: Duration,
    reranker: Option<Duration>,
}

impl Worker {
    pub(super) fn contextual(
        &mut self,
        snapshot: &Snapshot,
        settings: &TypoSettings,
        active: &AtomicU8,
        cancelled: impl Fn() -> bool + Sync,
        budget: Duration,
        timings: &mut Timings,
    ) -> io::Result<Vec<Suggestion>> {
        let detector = match &mut self.detector {
            Ok(Some(detector)) => detector,
            Ok(None) => return Ok(Vec::new()),
            Err(error) => return Err(io::Error::new(error.kind(), error.to_string())),
        };
        detector.ready()?;
        let start = Instant::now();
        let deadline = start + budget;
        let window = window(snapshot, settings.japanese.context_chars);
        let text = &snapshot.text[window.clone()];
        let caret = snapshot.caret - window.start;
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let hits = detector.detect(text, caret, &settings.japanese);
        timings.detector = start.elapsed();
        if hits
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::TimedOut)
        {
            self.detector = Err(io::Error::other("Japanese typo worker timed out"));
        }
        let mut hits = hits?;
        hits.retain(|hit| {
            hit.start < hit.end
                && text.get(hit.start..hit.end).is_some()
                && snapshot.preedit_start.is_none_or(|preedit| {
                    window.start + hit.end <= preedit
                        || (window.start + hit.start >= preedit
                            && window.start + hit.end <= snapshot.caret)
                        || window.start + hit.start >= snapshot.caret
                })
        });
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let start = Instant::now();
        let result = (|| {
            self.local.prepare(
                settings.d1_device,
                active,
                settings.limits().concurrent,
                deadline,
                &cancelled,
            )?;
            let (url, key) = self.local.connection()?;
            let body = batch(text, &hits);
            let client = match &self.loopback {
                Some(client) => client,
                None => self
                    .loopback
                    .insert(vime::system_one::CancellableClient::new(true)?),
            };
            client.post(url, key, &body, deadline, &cancelled)
        })();
        timings.reranker = Some(start.elapsed());
        match result {
            Ok(response) => ranked(&response, hits, window.start, settings.japanese.candidates),
            // The next input can proceed as soon as the interactive deadline expires.
            Err(error) if error.kind() == io::ErrorKind::TimedOut && budget == BUDGET => {
                Ok(Vec::new())
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, path::PathBuf};
    use vime::{ime::Language, settings::Settings};

    #[test]
    fn ranks_each_position_against_keep_and_preserves_utf8_ranges() {
        let hits = vec![
            Hit {
                start: 3,
                end: 6,
                alternatives: vec!["猫".into(), "鳥".into()],
            },
            Hit {
                start: 6,
                end: 9,
                alternatives: vec!["は".into()],
            },
        ];
        let request = batch("私犬が", &hits);
        assert_eq!(request["questions"]["position_0"]["criteria"]["keep"], "犬");
        assert_eq!(request["state"], "私[0]犬[/0][1]が[/1]");
        assert_eq!(request["questions"].as_object().unwrap().len(), 2);
        let response = json!({"answers": {
            "position_0": {"probabilities": {"fix_0": 0.30, "fix_1": 0.60, "keep": 0.10}},
            "position_1": {"probabilities": {"fix_0": 0.12, "keep": 0.88}}
        }});
        let suggestions = ranked(&response, hits, 6, 5).unwrap();
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.word.as_str())
                .collect::<Vec<_>>(),
            ["鳥", "猫"]
        );
        assert_eq!(suggestions[0].range, 9..12);
    }

    #[test]
    #[ignore = "requires the local d1-2B engine and DirectML model"]
    fn japanese_typo_benchmark() -> io::Result<()> {
        let directory = env::var_os("VIME_TYPO_BENCH_DIR")
            .map_or_else(Settings::directory, |path| Ok(PathBuf::from(path)))?;
        let model_directory = env::var_os("VIME_TYPO_BENCH_MODEL_DIR")
            .map_or_else(|| directory.clone(), PathBuf::from);
        let mut settings = Settings::load(&directory)?.typo;
        if settings.provider != vime::settings::TypoProvider::D1Local {
            settings = TypoSettings::recommended();
        }
        let active = AtomicU8::new(settings.d1_device as u8);
        let mut worker = Worker::new(directory);
        if matches!(worker.detector, Ok(None)) {
            worker.detector = Detector::start(&model_directory);
        }
        let detector = worker
            .detector
            .as_mut()
            .map_err(|error| io::Error::other(error.to_string()))?
            .as_mut()
            .ok_or_else(|| io::Error::other("Prepare the LINE ONNX model first"))?;
        detector.ready()?;
        worker.local.prepare(
            settings.d1_device,
            &active,
            settings.limits().concurrent,
            Instant::now() + START_TIMEOUT,
            || false,
        )?;
        worker.local.connection()?;

        let samples = [
            ("今日の天気は晴れです。", "午後は散歩します。"),
            ("明日は大事な会議がありまふ。", ""),
            ("ねこがソファーで眠っでいます。", ""),
            (
                "来月の会議では、これまでに集めた資料をもとに、新しい企画の内容を説明します。",
                "気になる点があれば、その場で質問してください。次に進む前に確認します。",
            ),
        ];
        let snapshots: Vec<_> = samples
            .into_iter()
            .map(|(left, right)| Snapshot {
                text: format!("{left}{right}"),
                caret: left.len(),
                language: Language::Japanese,
                preedit_start: None,
            })
            .collect();
        let mut detector = Vec::new();
        let mut reranker = Vec::new();
        let mut total = Vec::new();
        for iteration in 0..6 {
            for (sample, snapshot) in snapshots.iter().enumerate() {
                let mut times = Timings::default();
                let start = Instant::now();
                let suggestions = worker.contextual(
                    snapshot,
                    &settings,
                    &active,
                    || false,
                    Duration::from_secs(5),
                    &mut times,
                )?;
                if iteration == 0 {
                    println!(
                        "sample {}: {:?}",
                        sample + 1,
                        suggestions
                            .iter()
                            .map(|s| s.word.as_str())
                            .collect::<Vec<_>>()
                    );
                    continue;
                }
                total.push(start.elapsed());
                detector.push(times.detector);
                reranker.extend(times.reranker);
                thread::sleep(TARGET.saturating_sub(start.elapsed()));
            }
        }
        for (name, data) in [
            ("DistilBERT", detector),
            ("d1-2B (when called)", reranker),
            ("total", total),
        ] {
            let mut data = data;
            data.sort_unstable();
            if let Some(max) = data.last() {
                let ms = |time: Duration| time.as_secs_f64() * 1000.0;
                println!(
                    "{name}: n={}, p50={:.1} ms, p95={:.1} ms, max={:.1} ms, >125 ms={}",
                    data.len(),
                    ms(data[data.len().div_ceil(2) - 1]),
                    ms(data[(data.len() * 95).div_ceil(100) - 1]),
                    ms(*max),
                    data.iter().filter(|&&value| value > TARGET).count()
                );
            } else {
                println!("{name}: no samples");
            }
        }
        Ok(())
    }
}
