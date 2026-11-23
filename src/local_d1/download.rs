use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::Ordering::Relaxed,
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use super::{
    Archive, MODEL_NAME, MODEL_REVISION, Runtime, engine_path, manifest, model_path, runtime,
    runtime_cache,
};
use crate::{
    download::{Progress, fetch},
    i18n::tr,
    settings::D1Device,
};
use std::os::windows::process::CommandExt;

pub const MODEL_BYTES: u64 = 1_099_629_888;
const MODEL_HASH: &str = "adca5789ad30ae6027e8e936749bb9f19673097492ab26fa1c5b69643ca50b4b";
const MODEL_BASE: &str = "https://huggingface.co/LiquidAI/d1-2B-GGUF/resolve";
pub const MODEL_LICENSE: &str = "https://huggingface.co/LiquidAI/d1-2B-GGUF/blob/4a181094d1b6addfaa3a0956f78e7ab2aa406a0c/LICENSE";

pub fn model_available(directory: &Path) -> bool {
    fs::metadata(model_path(directory))
        .is_ok_and(|file| file.is_file() && file.len() == MODEL_BYTES)
}

pub fn installed(directory: &Path, device: D1Device) -> bool {
    model_available(directory) && engine_path(directory, runtime(device)).is_some()
}

pub fn required_bytes(directory: &Path, device: D1Device) -> u64 {
    let runtime = runtime(device);
    let engine = if engine_path(directory, runtime).is_none() {
        runtime.archives().map(|archive| archive.bytes).sum()
    } else {
        0
    };
    engine
        + if model_available(directory) {
            0
        } else {
            MODEL_BYTES
        }
}

pub struct Download {
    total: u64,
    progress: Arc<Progress>,
    complete: Receiver<io::Result<()>>,
}

impl Download {
    pub fn start(
        directory: PathBuf,
        device: D1Device,
        wake: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let runtime = runtime(device);
        let download_engine = engine_path(&directory, runtime).is_none();
        let download_model = !model_available(&directory);
        let total = required_bytes(&directory, device);
        let progress = Arc::new(Progress::default());
        let worker = progress.clone();
        let (sender, complete) = channel();
        thread::Builder::new()
            .name("vime-d1-download".into())
            .spawn(move || {
                let result = (|| {
                    if download_engine {
                        install_engine(&directory, runtime, &worker)?;
                    }
                    if download_model && !model_available(&directory) {
                        install_model(&directory, &worker)?;
                    }
                    Ok(())
                })();
                let _ = sender.send(result);
                wake();
            })?;
        Ok(Self {
            total,
            progress,
            complete,
        })
    }

    pub fn received(&self) -> u64 {
        self.progress.received.load(Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total
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

fn install_model(directory: &Path, progress: &Progress) -> io::Result<()> {
    let path = model_path(directory);
    let parent = path.parent().expect("model has a parent directory");
    fs::create_dir_all(parent)?;
    let url = format!("{MODEL_BASE}/{MODEL_REVISION}/{MODEL_NAME}");
    let file = fetch(&url, MODEL_BYTES, MODEL_HASH, parent, progress)?;
    progress.check()?;
    match file.persist(path) {
        Ok(_) => Ok(()),
        Err(_) if model_available(directory) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn install_engine(directory: &Path, runtime: &Runtime, progress: &Progress) -> io::Result<()> {
    let install = runtime_cache(directory, runtime);
    let parent = install.parent().expect("engine has a parent directory");
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    // Only the checked upstream archive reaches the Windows archive tool.
    let tools = staging.path().join("runtime");
    fs::create_dir(&tools)?;
    let tar = PathBuf::from(env::var_os("SystemRoot").ok_or(io::ErrorKind::NotFound)?)
        .join("System32")
        .join("tar.exe");
    for asset in runtime.archives() {
        let archive = fetch(
            &asset.url,
            asset.bytes,
            &asset.sha256,
            staging.path(),
            progress,
        )?;
        extract_archive(&tar, archive.path(), &tools, asset)?;
    }
    fs::write(
        tools.join("LICENSE-llama.cpp"),
        include_str!("LLAMA_LICENSE"),
    )?;
    progress.check()?;
    // Publish the engine directory atomically; another ViME can install the
    // same pinned runtime concurrently.
    match fs::rename(&tools, &install) {
        Ok(()) => Ok(()),
        Err(_) if engine_path(directory, runtime).is_some() => Ok(()),
        Err(error) => Err(error),
    }
}

fn extract_archive(tar: &Path, archive: &Path, tools: &Path, asset: &Archive) -> io::Result<()> {
    let mut command = Command::new(tar);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let extraction_failed = || io::Error::other(tr("推論エンジンを展開できませんでした"));
    let mut files = if asset.files.is_empty() {
        manifest().files.clone()
    } else {
        asset.files.clone()
    };
    if !asset.optional_files.is_empty() {
        let listing = command.args(["-tf"]).arg(archive).output()?;
        if !listing.status.success() {
            return Err(extraction_failed());
        }
        let names = String::from_utf8_lossy(&listing.stdout);
        files.extend(
            asset
                .optional_files
                .iter()
                .filter(|name| names.lines().any(|line| line == name.as_str()))
                .cloned(),
        );
        command = Command::new(tar);
        command
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
    }
    if !command
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(tools)
        .args(files)
        .stdout(Stdio::null())
        .status()?
        .success()
    {
        return Err(extraction_failed());
    }
    Ok(())
}
