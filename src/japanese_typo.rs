use std::{
    env, fs,
    io::{self, Read, Seek, SeekFrom},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering::Relaxed},
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::{i18n::tr, job_process::Process};

const PREPARE: &str = include_str!("../tools/prepare_japanese_typo.py");
const PYTHON: &str = "venv/Scripts/python.exe";
const DEADLINE: Duration = Duration::from_secs(1200);

pub fn model_directory(directory: &Path) -> Option<PathBuf> {
    let directory = directory.join("japanese-typo");
    [directory.clone(), directory.join("managed")]
        .into_iter()
        .find(|directory| {
            [PYTHON, "masked-lm-fp16.onnx", "spiece.model"]
                .into_iter()
                .all(|file| directory.join(file).is_file())
        })
}

#[derive(Default)]
struct Progress {
    cancelled: AtomicBool,
    stage: AtomicU8,
}

pub struct Download {
    progress: Arc<Progress>,
    complete: Receiver<io::Result<()>>,
}

impl Download {
    pub fn start(directory: PathBuf, wake: impl FnOnce() + Send + 'static) -> io::Result<Self> {
        let progress = Arc::new(Progress::default());
        let worker = progress.clone();
        let (sender, complete) = channel();
        thread::Builder::new()
            .name("vime-japanese-model-download".into())
            .spawn(move || {
                let _ = sender.send(install(&directory, &worker));
                wake();
            })?;
        Ok(Self { progress, complete })
    }

    pub fn status(&self) -> &'static str {
        match self.progress.stage.load(Relaxed) {
            1 => tr("日本語モデルの実行環境を準備しています"),
            2 => tr("日本語モデルをダウンロードして FP16 に変換しています"),
            _ => tr("Python 3.12 を確認しています"),
        }
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

fn check(progress: &Progress, deadline: Instant) -> io::Result<()> {
    if progress.cancelled.load(Relaxed) {
        Err(io::ErrorKind::Interrupted.into())
    } else if Instant::now() >= deadline {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(())
    }
}

fn run(command: &mut Command, progress: &Progress, deadline: Instant) -> io::Result<()> {
    check(progress, deadline)?;
    let mut log = tempfile::tempfile()?;
    command
        .creation_flags(CREATE_NO_WINDOW)
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?);
    let mut child = Process::spawn(command)?;
    loop {
        check(progress, deadline)?;
        if let Some(status) = child.child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let len = log.metadata()?.len();
            log.seek(SeekFrom::Start(len.saturating_sub(2048)))?;
            let mut bytes = Vec::new();
            log.read_to_end(&mut bytes)?;
            let message = String::from_utf8_lossy(&bytes);
            return Err(io::Error::other(format!("{status}\n{}", message.trim())));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn python(progress: &Progress, deadline: Instant) -> io::Result<Command> {
    let paths = env::var_os("PATH").unwrap_or_default();
    let probe = "import struct, sys; sys.exit(not (sys.version_info[:2] == (3, 12) and struct.calcsize('P') == 8))";
    let available = env::split_paths(&paths)
        .filter(|path| path.is_absolute() && !path.ends_with("WindowsApps"))
        .flat_map(|path| {
            [
                (path.join("python.exe"), &[][..]),
                (path.join("py.exe"), &["-3.12"][..]),
            ]
        })
        .chain(env::var_os("LOCALAPPDATA").map(|root| {
            (
                PathBuf::from(root).join("Programs/Python/Python312/python.exe"),
                &[][..],
            )
        }))
        .chain(
            env::var_os("SystemRoot")
                .map(|root| (PathBuf::from(root).join("py.exe"), &["-3.12"][..])),
        );
    for (path, args) in available {
        if path.is_file() {
            let mut command = Command::new(&path);
            match run(
                command.args(args).args(["-I", "-c", probe]),
                progress,
                deadline,
            ) {
                Ok(()) => {
                    let mut python = Command::new(path);
                    python.args(args);
                    return Ok(python);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        tr("Windows x64 版の Python 3.12 をインストールしてください"),
    ))
}

fn pip(python: &Path) -> Command {
    let mut pip = Command::new(python);
    pip.args([
        "-I",
        "-m",
        "pip",
        "install",
        "--no-input",
        "--timeout",
        "30",
    ]);
    pip
}

fn install(directory: &Path, progress: &Progress) -> io::Result<()> {
    if model_directory(directory).is_some() {
        return Ok(());
    }
    let deadline = Instant::now() + DEADLINE;
    let mut python = python(progress, deadline)?;
    let root = directory.join("japanese-typo");
    fs::create_dir_all(&root)?;
    let staging = tempfile::tempdir_in(&root)?;
    let runtime = staging.path().join("runtime");
    let build = staging.path().join("build");
    progress.stage.store(1, Relaxed);
    // The build environment is temporary. The runtime only needs inference packages.
    run(
        python
            .args(["-I", "-m", "venv"])
            .arg(runtime.join("venv"))
            .arg(&build),
        progress,
        deadline,
    )?;
    run(
        pip(&runtime.join(PYTHON)).args([
            "onnxruntime-directml==1.24.4",
            "sentencepiece==0.2.1",
            "fugashi==1.5.2",
            "unidic-lite==1.0.8",
            "regex",
        ]),
        progress,
        deadline,
    )?;
    let builder = build.join("Scripts/python.exe");
    run(
        pip(&builder).args(["onnx", "onnxconverter-common", "transformers==4.57.6"]),
        progress,
        deadline,
    )?;
    run(
        pip(&builder).args([
            "--extra-index-url",
            "https://download.pytorch.org/whl/cpu",
            "torch==2.7.1+cpu",
        ]),
        progress,
        deadline,
    )?;
    progress.stage.store(2, Relaxed);
    run(
        Command::new(builder)
            .args(["-I", "-W", "ignore", "-c", PREPARE, "--directory"])
            .arg(&runtime)
            .env("TMP", staging.path())
            .env("TEMP", staging.path()),
        progress,
        deadline,
    )?;
    check(progress, deadline)?;
    if model_directory(directory).is_none()
        && let Err(error) = fs::rename(runtime, root.join("managed"))
        && model_directory(directory).is_none()
    {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "downloads and converts the official LINE model in a new Python environment"]
    fn installs_japanese_typo_model() -> io::Result<()> {
        let temporary = tempfile::tempdir()?;
        let directory = env::var_os("VIME_JAPANESE_INSTALL_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| temporary.path().to_owned());
        let progress = Progress::default();
        install(&directory, &progress)?;
        let runtime = model_directory(&directory).ok_or(io::ErrorKind::NotFound)?;
        run(
            Command::new(runtime.join(PYTHON)).args([
                "-I", "-c",
                "import fugashi, onnxruntime, regex, sentencepiece, unidic_lite; assert 'DmlExecutionProvider' in onnxruntime.get_available_providers()",
            ]),
            &progress, Instant::now() + Duration::from_secs(30),
        )
    }
}
