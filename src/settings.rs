use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    #[default]
    Osc,
    SteamVrInput,
}

impl Destination {
    pub fn label(self) -> &'static str {
        match self {
            Self::Osc => "VRChat OSC",
            Self::SteamVrInput => "SteamVR アプリ入力",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub destination: Destination,
    pub osc_port: u16,
    pub osc_notify: bool,
    pub hotkey: String,
    pub overlay_width: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            destination: Destination::Osc,
            osc_port: 9000,
            osc_notify: false,
            hotkey: "Ctrl+Shift+F8".into(),
            overlay_width: 1.08,
        }
    }
}

impl Settings {
    pub fn directory() -> io::Result<PathBuf> {
        env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("ViME"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA が見つかりません"))
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let config = match fs::read(path.join("settings.json")) {
            Ok(data) => serde_json::from_slice::<Self>(&data)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.validate()?;
        fs::create_dir_all(path)?;
        let mut temporary = tempfile::NamedTempFile::new_in(path)?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), self)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path.join("settings.json"))?;
        Ok(())
    }

    fn validate(&self) -> io::Result<()> {
        if self.osc_port == 0 || !(0.65..=1.65).contains(&self.overlay_width) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "OSC ポートまたはキーボード幅が範囲外です",
            ));
        }
        Ok(())
    }
}
