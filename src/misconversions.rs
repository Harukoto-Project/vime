//! Fetches the Japanese Wikipedia Typo Dataset (JWTD v2) from Kyoto University
//! on the user's request and boils it down to the misconversion table on this
//! PC. ViME neither ships nor hosts the dataset (CC BY-SA 3.0) or the table.

use std::{
    env, fs,
    io::{self, BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::{
    download::{Progress, fetch},
    i18n::tr,
    ime::misconversion::{FILE, Table, Tally},
};

pub const DATA_BYTES: u64 = 98_567_960;
const DATA_HASH: &str = "2c6d864905adbac75a2c0181aa8796c81d3f86df41aa8bf79b37d47ee8ec5025";
const DATA_URL: &str = "https://nlp.ist.i.kyoto-u.ac.jp/nl-resource/JWTD/jwtd_v2.0.tar.gz";
const TRAIN: &str = "jwtd_v2.0/train.jsonl";
/// A table smaller than this means the archive was not what it should be.
const MIN_WORDS: usize = 1000;
pub const SOURCE_PAGE: &str = "https://nlp.ist.i.kyoto-u.ac.jp/EN/index.php?JWTD";
pub const LICENSE: &str = "https://creativecommons.org/licenses/by-sa/3.0/";

pub fn installed(directory: &Path) -> bool {
    directory.join(FILE).is_file()
}

pub fn remove(directory: &Path) -> io::Result<()> {
    match fs::remove_file(directory.join(FILE)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

pub struct Download {
    progress: Arc<Progress>,
    summarizing: Arc<AtomicBool>,
    complete: Receiver<io::Result<()>>,
}

impl Download {
    pub fn start(directory: PathBuf, wake: impl FnOnce() + Send + 'static) -> io::Result<Self> {
        let progress = Arc::new(Progress::default());
        let summarizing = Arc::new(AtomicBool::new(false));
        let (worker, stage) = (progress.clone(), summarizing.clone());
        let (sender, complete) = channel();
        thread::Builder::new()
            .name("vime-misconversion-download".into())
            .spawn(move || {
                let _ = sender.send(install(&directory, &worker, &stage));
                wake();
            })?;
        Ok(Self {
            progress,
            summarizing,
            complete,
        })
    }

    pub fn received(&self) -> u64 {
        self.progress.received.load(Relaxed)
    }

    /// Whether the archive is fully downloaded and being counted.
    pub fn summarizing(&self) -> bool {
        self.summarizing.load(Relaxed)
    }

    pub fn cancelling(&self) -> bool {
        self.progress.cancelled.load(Relaxed)
    }

    pub fn cancel(&self) {
        self.progress.cancelled.store(true, Relaxed);
    }

    pub fn result(&self) -> Option<io::Result<()>> {
        match self.complete.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(io::Error::other(tr("ダウンロードが中断されました"))))
            }
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn install(directory: &Path, progress: &Progress, summarizing: &AtomicBool) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    // The archive is only needed while counting; the table is all that stays.
    let staging = tempfile::tempdir_in(directory)?;
    let archive = fetch(DATA_URL, DATA_BYTES, DATA_HASH, staging.path(), progress)?;
    summarizing.store(true, Relaxed);
    let table = summarize(archive.path(), progress, MIN_WORDS)?;
    progress.check()?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(table.serialize().as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(directory.join(FILE))
        .map_err(|error| error.error)?;
    Ok(())
}

fn summarize(archive: &Path, progress: &Progress, minimum: usize) -> io::Result<Table> {
    let failed = || io::Error::other(tr("誤変換データを集計できませんでした"));
    let mut tally = Tally::default();
    read_train(archive, progress, |line| tally.count_corrections(line))?;
    if tally.corrected_words() < minimum {
        return Err(failed());
    }
    tally.start_second_pass();
    read_train(archive, progress, |line| tally.count_kept(line))?;
    let table = tally.table();
    if table.len() < minimum {
        return Err(failed());
    }
    Ok(table)
}

/// Streams `train.jsonl` out of the archive with the Windows archive tool.
fn read_train(archive: &Path, progress: &Progress, mut each: impl FnMut(&str)) -> io::Result<()> {
    let tar = PathBuf::from(env::var_os("SystemRoot").ok_or(io::ErrorKind::NotFound)?)
        .join("System32")
        .join("tar.exe");
    let mut child = Command::new(tar)
        .creation_flags(CREATE_NO_WINDOW)
        .arg("-xOf")
        .arg(archive)
        .arg(TRAIN)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let read: io::Result<u64> = (|| {
        let mut lines = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
        let mut line = Vec::new();
        let mut count = 0u64;
        loop {
            line.clear();
            if lines.read_until(b'\n', &mut line)? == 0 {
                return Ok(count);
            }
            count += 1;
            if count.is_multiple_of(1000) {
                progress.check()?;
            }
            if let Ok(line) = std::str::from_utf8(&line) {
                each(line);
            }
        }
    })();
    if read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let count = read?;
    if !status.success() || count == 0 {
        return Err(io::Error::other(tr("誤変換データを展開できませんでした")));
    }
    progress.check()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(directory: &Path, lines: &[String]) -> PathBuf {
        let inside = directory.join("jwtd_v2.0");
        fs::create_dir(&inside).unwrap();
        fs::write(inside.join("train.jsonl"), lines.join("\n")).unwrap();
        let archive = directory.join("jwtd.tar.gz");
        let tar = PathBuf::from(env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("tar.exe");
        assert!(
            Command::new(tar)
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(directory)
                .arg("jwtd_v2.0")
                .status()
                .unwrap()
                .success()
        );
        archive
    }

    fn line(post_text: &str, pre: &str, post: &str) -> String {
        format!(
            r#"{{"post_text":"{post_text}","diffs":[{{"pre_str":"{pre}","post_str":"{post}","category":"kanji-conversion_a"}}]}}"#
        )
    }

    #[test]
    fn counts_the_dataset_inside_an_archive() {
        let directory = tempfile::tempdir().unwrap();
        let archive = archive(
            directory.path(),
            &[
                line("意義がある。", "異義", "意義"),
                line("異義語を調べる。", "異義", "意義"),
                line("意外だった。", "以外", "意外"),
            ],
        );
        let progress = Progress::default();
        let table = summarize(&archive, &progress, 1).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table.find("それは異義").unwrap().1, ["意義".to_owned()]);
        // Too few words means the wrong archive.
        assert!(summarize(&archive, &progress, 3).is_err());
        progress.cancelled.store(true, Relaxed);
        let many: Vec<_> = (0..2000).map(|_| line("意義", "異義", "意義")).collect();
        let big = tempfile::tempdir().unwrap();
        assert_eq!(
            summarize(&self::archive(big.path(), &many), &progress, 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn rejects_a_file_that_is_not_the_dataset() {
        let directory = tempfile::tempdir().unwrap();
        let other = directory.path().join("other.tar.gz");
        fs::write(&other, b"not an archive").unwrap();
        assert!(summarize(&other, &Progress::default(), 1).is_err());
    }

    #[test]
    #[ignore = "downloads 98 MB from Kyoto University"]
    fn downloads_counts_and_saves_the_dataset() {
        let directory = tempfile::tempdir().unwrap();
        let download = Download::start(directory.path().to_owned(), || {}).unwrap();
        let result = loop {
            if let Some(result) = download.result() {
                break result;
            }
            thread::sleep(std::time::Duration::from_millis(200));
        };
        result.unwrap();
        assert_eq!(download.received(), DATA_BYTES);
        assert!(download.summarizing());
        assert!(Table::load(directory.path()).unwrap().len() > 20_000);
        // Only the table stays; the archive is gone.
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    /// `VIME_JWTD_ARCHIVE=<path to jwtd_v2.0.tar.gz> cargo test -- --ignored`
    #[test]
    #[ignore = "needs the 98 MB dataset"]
    fn summarizes_the_real_dataset() {
        let archive = PathBuf::from(env::var_os("VIME_JWTD_ARCHIVE").unwrap());
        let table = summarize(&archive, &Progress::default(), MIN_WORDS).unwrap();
        eprintln!("{} words", table.len());
        let (range, fixes) = table.find("それは入場").unwrap_or((0..0, &[]));
        eprintln!("{range:?} {fixes:?}");
        assert!(table.len() > 20_000);
    }
    #[test]
    fn installs_and_removes_the_saved_table() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!installed(directory.path()));
        remove(directory.path()).unwrap();
        fs::write(directory.path().join(FILE), "異義\t2\t0\t意義\n").unwrap();
        assert!(installed(directory.path()));
        assert_eq!(Table::load(directory.path()).unwrap().len(), 1);
        remove(directory.path()).unwrap();
        assert!(!installed(directory.path()));
        assert!(Table::load(directory.path()).is_none());
    }
}
