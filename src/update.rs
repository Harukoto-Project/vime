use std::{
    env, fs, io,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{Receiver, TryRecvError, channel},
    thread,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use eframe::egui::{Context, RichText, Ui};
use semver::Version;
use serde::Deserialize;
use tempfile::TempDir;
use windows_sys::{
    Win32::{
        System::Threading::CREATE_NO_WINDOW,
        UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    },
    core::w,
};

use crate::{
    download::{Progress, fetch, file_sha256},
    i18n::{tr, tr_args},
    system_one::Client,
};

pub const VERSION_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"));
const REPOSITORY: &str = "https://github.com/Harukoto-Project/vime";
const LATEST: &str = "https://api.github.com/repos/Harukoto-Project/vime/releases/latest";
const ARCHIVE_LIMIT: u64 = 1024 * 1024 * 1024;
const EXECUTABLE: &str = "vime.exe";

#[derive(Default)]
pub struct Update {
    state: State,
    worker: Option<Receiver<io::Result<State>>>,
    restart: bool,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Checking,
    Preparing(Version),
    Current,
    NoRelease,
    Ready(Staged),
    Failed(io::Error),
}

pub enum Action {
    Check,
    Restart,
}

impl Update {
    pub fn check(&mut self, context: &Context) {
        if self.worker.is_some() || matches!(self.state, State::Ready(_)) {
            return;
        }
        let context = context.clone();
        let (sender, worker) = channel();
        let started = thread::Builder::new()
            .name("vime-update".into())
            .spawn(move || {
                let result = prepare_update(|version| {
                    let _ = sender.send(Ok(State::Preparing(version)));
                    context.request_repaint();
                });
                let _ = sender.send(result);
                context.request_repaint();
            });
        match started {
            Ok(_) => {
                self.worker = Some(worker);
                self.state = State::Checking;
            }
            Err(error) => self.state = State::Failed(error),
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(worker) = &self.worker {
            let state = match worker.try_recv() {
                Ok(result) => result.unwrap_or_else(State::Failed),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => State::Failed(io::ErrorKind::Interrupted.into()),
            };
            if !matches!(state, State::Preparing(_)) {
                self.worker = None;
            }
            self.state = state;
            changed = true;
        }
        changed
    }

    pub fn restart(&mut self, context: &Context) {
        if matches!(self.state, State::Ready(_)) {
            self.restart = true;
            context.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
    }

    pub fn apply_on_exit(&mut self) {
        self.poll();
        if let State::Ready(staged) = std::mem::take(&mut self.state)
            && let Err(error) = staged.launch(self.restart)
        {
            let message = tr_args(
                "アップデートを適用できませんでした ({0})",
                &[&error.to_string()],
            );
            let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    wide.as_ptr(),
                    w!("ViME"),
                    MB_OK | MB_ICONERROR,
                )
            };
        }
    }

    pub fn show(&self, ui: &mut Ui) -> Option<Action> {
        ui.label(RichText::new(tr("アップデート")).strong());
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if matches!(self.state, State::Ready(_)) {
                if ui.button(tr("再起動して更新")).clicked() {
                    action = Some(Action::Restart);
                }
            } else if ui
                .add_enabled(
                    self.worker.is_none(),
                    eframe::egui::Button::new(tr("アップデートを確認")),
                )
                .clicked()
            {
                action = Some(Action::Check);
            }
            let status = match &self.state {
                State::Idle => return,
                State::Checking => tr("アップデートを確認中…").into(),
                State::Preparing(version) => {
                    tr_args("v{0} をダウンロード中…", &[&version.to_string()])
                }
                State::Current => tr("最新バージョンです").into(),
                State::NoRelease => tr("公開済みの安定版はありません").into(),
                State::Ready(staged) => {
                    tr_args("終了時に v{0} に更新します", &[&staged.version.to_string()])
                }
                State::Failed(error) => tr_args(
                    "アップデートを準備できませんでした ({0})",
                    &[&error.to_string()],
                ),
            };
            let label = RichText::new(status);
            ui.label(if matches!(self.state, State::Failed(_)) {
                label.color(ui.visuals().error_fg_color)
            } else {
                label.weak()
            });
        });
        action
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}

impl Release {
    fn package(&self, installed: &Version) -> io::Result<Option<(Version, u64)>> {
        if self.draft || self.prerelease {
            return Ok(None);
        }
        let version = self
            .tag_name
            .strip_prefix('v')
            .and_then(|tag| Version::parse(tag).ok())
            .ok_or(io::ErrorKind::InvalidData)?;
        if !version.pre.is_empty() || !version.cmp_precedence(installed).is_gt() {
            return Ok(None);
        }
        let archive = archive_name(&version);
        let checksum = format!("{archive}.sha256");
        let size = self.assets.iter().find(|asset| {
            asset.name == archive
                && asset.state == "uploaded"
                && (1..=ARCHIVE_LIMIT).contains(&asset.size)
        });
        match size {
            Some(asset)
                if self.assets.iter().any(|asset| {
                    asset.name == checksum
                        && asset.state == "uploaded"
                        && (1..=1024).contains(&asset.size)
                }) =>
            {
                Ok(Some((version, asset.size)))
            }
            _ => Err(io::Error::other(tr(
                "Windows x64 用の更新ファイルが見つかりません",
            ))),
        }
    }
}

fn archive_name(version: &Version) -> String {
    format!("ViME-{version}-windows-x64.zip")
}

fn read_response(url: &str, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    Client::download(url, |chunk| {
        if bytes.len() + chunk.len() > limit {
            return Err(io::ErrorKind::InvalidData.into());
        }
        bytes.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(bytes)
}

fn read_checksum(data: &[u8], archive: &str) -> io::Result<String> {
    let text = std::str::from_utf8(data).map_err(|_| io::ErrorKind::InvalidData)?;
    let (hash, filename) = text
        .trim()
        .split_once("  ")
        .ok_or(io::ErrorKind::InvalidData)?;
    if filename != archive || hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(hash.to_ascii_lowercase())
}

fn prepare_update(preparing: impl FnOnce(Version)) -> io::Result<State> {
    let response = match read_response(LATEST, 2 * 1024 * 1024) {
        Ok(response) => response,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(State::NoRelease),
        Err(error) => return Err(error),
    };
    let release: Release = serde_json::from_slice(&response)?;
    let installed = Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package version");
    let Some((version, bytes)) = release.package(&installed)? else {
        return Ok(State::Current);
    };
    preparing(version.clone());
    let executable = env::current_exe()?;
    let original_hash = file_sha256(&executable)?;
    let archive = archive_name(&version);
    let url = format!("{REPOSITORY}/releases/download/v{version}/{archive}").replace('+', "%2B");
    let checksum = read_checksum(&read_response(&format!("{url}.sha256"), 1024)?, &archive)?;
    let staging = tempfile::Builder::new()
        .prefix(".vime-update-")
        .tempdir_in(executable.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let file = fetch(&url, bytes, &checksum, staging.path(), &Progress::default())?;
    let package = staging.path().join("package");
    fs::create_dir(&package)?;
    if !Command::new(system_tool("tar.exe")?)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .arg("-xf")
        .arg(file.path())
        .arg("-C")
        .arg(&package)
        .status()?
        .success()
        || !package.join(EXECUTABLE).is_file()
    {
        return Err(io::Error::other(tr("更新ファイルを展開できませんでした")));
    }
    Ok(State::Ready(Staged {
        version,
        staging,
        executable,
        original_hash,
    }))
}

fn system_tool(name: &str) -> io::Result<PathBuf> {
    Ok(
        PathBuf::from(env::var_os("SystemRoot").ok_or(io::ErrorKind::NotFound)?)
            .join("System32")
            .join(name),
    )
}

struct Staged {
    version: Version,
    staging: TempDir,
    executable: PathBuf,
    original_hash: String,
}

impl Staged {
    fn launch(self, restart: bool) -> io::Result<()> {
        self.installer(restart, std::process::id())?
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let _ = self.staging.keep();
        Ok(())
    }

    fn installer(&self, restart: bool, pid: u32) -> io::Result<Command> {
        // The script is static. Pass local paths as environment values to preserve
        // Unicode and avoid interpreting any path characters as PowerShell code.
        let script: Vec<_> = include_str!("update/install.ps1")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut helper = Command::new(system_tool("WindowsPowerShell/v1.0/powershell.exe")?);
        helper
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
            .arg(STANDARD.encode(script))
            .env("VIME_UPDATE_STAGING", self.staging.path())
            .env("VIME_UPDATE_EXE", &self.executable)
            .env("VIME_UPDATE_ORIGINAL_HASH", &self.original_hash)
            .env("VIME_UPDATE_PID", pid.to_string())
            .env("VIME_UPDATE_RESTART", if restart { "1" } else { "0" })
            .env(
                "VIME_UPDATE_FAILED",
                tr("アップデートを適用できませんでした"),
            );
        Ok(helper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    #[test]
    fn accepts_only_newer_stable_windows_packages() {
        let mut release: Release = serde_json::from_str(
            r#"{
                "tag_name": "v0.10.0", "draft": false, "prerelease": false,
                "assets": [
                    {"name":"ViME-0.10.0-windows-x64.zip","state":"uploaded","size":150},
                    {"name":"ViME-0.10.0-windows-x64.zip.sha256","state":"uploaded","size":95}
                ]
            }"#,
        )
        .unwrap();
        let installed = Version::new(0, 9, 0);
        assert_eq!(
            release.package(&installed).unwrap(),
            Some((Version::new(0, 10, 0), 150))
        );
        assert!(release.package(&Version::new(0, 10, 0)).unwrap().is_none());
        assert!(release.package(&Version::new(1, 0, 0)).unwrap().is_none());
        release.prerelease = true;
        assert!(release.package(&installed).unwrap().is_none());
        release.prerelease = false;
        release.assets.pop();
        assert!(release.package(&installed).is_err());
    }

    #[test]
    fn verifies_checksum_filename_and_digest_format() {
        let archive = archive_name(&Version::new(0, 10, 0));
        let hash = "AB".repeat(32);
        let sidecar = format!("{hash}  {archive}\r\n");
        assert_eq!(
            read_checksum(sidecar.as_bytes(), &archive).unwrap(),
            hash.to_lowercase()
        );
        assert!(read_checksum(sidecar.as_bytes(), "other.zip").is_err());
        assert!(read_checksum(format!("xyz  {archive}").as_bytes(), &archive).is_err());
    }

    #[test]
    fn installs_windows_package_and_rolls_back_when_executable_is_locked() {
        let root = tempfile::Builder::new()
            .prefix("ViME [更新] ")
            .tempdir()
            .unwrap();
        let executable = root.path().join(EXECUTABLE);
        let notes = root.path().join("README.md");
        let user_file = root.path().join("local.txt");
        fs::write(&executable, "old executable").unwrap();
        fs::write(&notes, "old notes").unwrap();
        fs::write(&user_file, "user's file").unwrap();

        for locked in [true, false] {
            let staged = Staged {
                version: Version::new(0, 10, 0),
                staging: tempfile::Builder::new()
                    .prefix(".vime-update-")
                    .tempdir_in(root.path())
                    .unwrap(),
                original_hash: file_sha256(&executable).unwrap(),
                executable: executable.clone(),
            };
            let package = staged.staging.path().join("package");
            let asset = root.path().join("assets/guide.txt");
            fs::create_dir_all(package.join("assets")).unwrap();
            fs::write(package.join(EXECUTABLE), "new executable").unwrap();
            fs::write(package.join("README.md"), "new notes").unwrap();
            fs::write(package.join("assets/guide.txt"), "new asset").unwrap();
            // Permit hashing the running version while preventing replacement.
            let handle = locked.then(|| {
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .open(&executable)
                    .unwrap()
            });
            let output = staged
                .installer(false, i32::MAX as u32)
                .unwrap()
                .env_remove("VIME_UPDATE_FAILED")
                .output()
                .unwrap();
            drop(handle);
            assert_eq!(
                output.status.success(),
                !locked,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                !staged.staging.path().exists(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(&executable).unwrap(),
                if locked {
                    "old executable"
                } else {
                    "new executable"
                }
            );
            assert_eq!(
                fs::read_to_string(&notes).unwrap(),
                if locked { "old notes" } else { "new notes" }
            );
            assert_eq!(asset.exists(), !locked);
            assert_eq!(fs::read_to_string(&user_file).unwrap(), "user's file");
        }
    }
}
