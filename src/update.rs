use std::{
    env, io,
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
    i18n::{self, Lang, tr, tr_args},
    settings::Settings,
    system_one::Client,
};

pub const VERSION_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"));
const REPOSITORY: &str = "https://github.com/Harukoto-Project/vime";
const LATEST: &str = "https://api.github.com/repos/Harukoto-Project/vime/releases/latest";
const PACKAGE_LIMIT: u64 = 1024 * 1024 * 1024;
const MIGRATION_CHECKSUM: &str = "vime-migration.msi.sha256";
const MIGRATION_INSTALLER: &str = "vime-migration.msi";

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
            show_install_error(&error);
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
        let package = package_name(&version);
        let checksum = format!("{package}.sha256");
        let size = self.assets.iter().find(|asset| {
            asset.name == package
                && asset.state == "uploaded"
                && (1..=PACKAGE_LIMIT).contains(&asset.size)
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

fn package_name(version: &Version) -> String {
    format!("ViME-{version}-windows-x64.msi")
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

fn read_checksum(data: &[u8], package: &str) -> io::Result<String> {
    let text = std::str::from_utf8(data).map_err(|_| io::ErrorKind::InvalidData)?;
    let (hash, filename) = text
        .trim()
        .split_once("  ")
        .ok_or(io::ErrorKind::InvalidData)?;
    if filename != package || hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
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
    let package = package_name(&version);
    let url = format!("{REPOSITORY}/releases/download/v{version}/{package}").replace('+', "%2B");
    let checksum = read_checksum(&read_response(&format!("{url}.sha256"), 1024)?, &package)?;
    let staging = tempfile::Builder::new().prefix("vime-update-").tempdir()?;
    let file = fetch(&url, bytes, &checksum, staging.path(), &Progress::default())?;
    file.persist(staging.path().join(package))?;
    Ok(State::Ready(Staged {
        version,
        staging,
        checksum,
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
    checksum: String,
}

impl Staged {
    fn launch(self, restart: bool) -> io::Result<()> {
        installer(restart)?
            .env("VIME_UPDATE_STAGING", self.staging.path())
            .env(
                "VIME_UPDATE_MSI",
                self.staging.path().join(package_name(&self.version)),
            )
            .env("VIME_UPDATE_CHECKSUM", &self.checksum)
            .env("VIME_UPDATE_CURRENT_EXE", env::current_exe()?)
            .spawn()?;
        let _ = self.staging.keep();
        Ok(())
    }
}

pub fn launch_migration() -> bool {
    if env::args_os().any(|arg| arg == "--skip-msi-migration") {
        return false;
    }
    let Ok(executable) = env::current_exe() else {
        return false;
    };
    let Some(directory) = executable.parent() else {
        return false;
    };
    if !directory.join(MIGRATION_CHECKSUM).is_file() {
        return false;
    }
    i18n::set_language(
        Settings::directory()
            .and_then(|directory| Settings::load(&directory))
            .map_or_else(|_| Lang::system(), |settings| settings.language),
    );
    let launch = || -> io::Result<()> {
        if std::fs::symlink_metadata(&executable)?
            .file_type()
            .is_symlink()
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let checksum = file_sha256(&executable)?;
        let installed = Settings::directory()?
            .with_file_name("Programs")
            .join("ViME/vime.exe");
        // Compare resolved paths in case the ZIP was extracted into the MSI
        // destination and the user starts it through a linked directory.
        let resolved = executable.canonicalize()?;
        let in_place = installed.canonicalize().is_ok_and(|path| path == resolved);
        installer(true)?
            .env("VIME_UPDATE_LEGACY_EXE", &executable)
            .env("VIME_UPDATE_LEGACY_HASH", checksum)
            .env("VIME_UPDATE_IN_PLACE", if in_place { "1" } else { "0" })
            .env(
                "VIME_UPDATE_STEAMVR",
                if env::args_os().any(|arg| arg == crate::steamvr::LAUNCH_ARGUMENT) {
                    "1"
                } else {
                    "0"
                },
            )
            .env(
                "VIME_UPDATE_MSI",
                executable.with_file_name(MIGRATION_INSTALLER),
            )
            .env(
                "VIME_UPDATE_SIDECAR",
                executable.with_file_name(MIGRATION_CHECKSUM),
            )
            .spawn()?;
        Ok(())
    };
    match launch() {
        Ok(()) => true,
        Err(error) => {
            show_install_error(&error);
            false
        }
    }
}

fn installer(restart: bool) -> io::Result<Command> {
    // Paths travel through environment values to preserve Unicode and quoting.
    let script: Vec<_> = include_str!("update/install.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut helper = Command::new(system_tool("WindowsPowerShell/v1.0/powershell.exe")?);
    helper
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-NonInteractive", "-STA", "-EncodedCommand"])
        .arg(STANDARD.encode(script))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("VIME_UPDATE_PID", std::process::id().to_string())
        .env("VIME_UPDATE_RESTART", if restart { "1" } else { "0" })
        .env(
            "VIME_UPDATE_FAILED",
            tr("アップデートを適用できませんでした"),
        );
    for name in [
        "STAGING",
        "CHECKSUM",
        "LEGACY_EXE",
        "LEGACY_HASH",
        "STEAMVR",
        "SIDECAR",
        "IN_PLACE",
        "CURRENT_EXE",
    ] {
        helper.env_remove(format!("VIME_UPDATE_{name}"));
    }
    Ok(helper)
}

fn show_install_error(error: &io::Error) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_newer_stable_windows_packages() {
        let mut release: Release = serde_json::from_str(
            r#"{
                "tag_name": "v0.10.0", "draft": false, "prerelease": false,
                "assets": [
                    {"name":"ViME-0.10.0-windows-x64.zip","state":"uploaded","size":150},
                    {"name":"ViME-0.10.0-windows-x64.zip.sha256","state":"uploaded","size":95},
                    {"name":"ViME-0.10.0-windows-x64.msi","state":"uploaded","size":200},
                    {"name":"ViME-0.10.0-windows-x64.msi.sha256","state":"uploaded","size":95}
                ]
            }"#,
        )
        .unwrap();
        let installed = Version::new(0, 9, 0);
        assert_eq!(
            release.package(&installed).unwrap(),
            Some((Version::new(0, 10, 0), 200))
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
        let package = package_name(&Version::new(0, 10, 0));
        let hash = "AB".repeat(32);
        let sidecar = format!("{hash}  {package}\r\n");
        assert_eq!(
            read_checksum(sidecar.as_bytes(), &package).unwrap(),
            hash.to_lowercase()
        );
        assert!(read_checksum(sidecar.as_bytes(), "other.zip").is_err());
        assert!(read_checksum(format!("xyz  {package}").as_bytes(), &package).is_err());
    }
}
