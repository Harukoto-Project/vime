use crate::{
    calibration::{Limits, Profile},
    i18n::{Lang, tr},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_updates_and_leaves_the_last_valid_settings_on_validation_error() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.save(directory.path()).unwrap();
        settings.destination = Destination::SteamVrInput;
        settings.keyboard_theme = KeyboardTheme::Light;
        settings.hotkey = "Alt+F8".into();
        settings.save(directory.path()).unwrap();
        let mut invalid = settings.clone();
        invalid.osc_port = 0;
        assert!(invalid.save(directory.path()).is_err());
        assert_eq!(Settings::load(directory.path()).unwrap(), settings);
    }

    #[test]
    fn typo_reporting_defaults_to_on_and_preserves_opt_out() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("settings.json"), "{}").unwrap();
        assert!(Settings::default().report_typos);
        let mut settings = Settings::load(directory.path()).unwrap();
        assert!(settings.report_typos);
        settings.report_typos = false;
        settings.save(directory.path()).unwrap();
        assert!(!Settings::load(directory.path()).unwrap().report_typos);
    }

    #[test]
    fn oscquery_defaults_on_for_new_users_and_preserves_existing_manual_routing() {
        assert!(Settings::default().osc_query);
        let previous: Settings = serde_json::from_str(r#"{"osc_port":9100}"#).unwrap();
        assert!(!previous.osc_query);
        assert_eq!(previous.osc_port, 9100);
    }

    #[test]
    fn is_set_up_once_settings_are_saved_and_proposes_local_cuda_d1() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!Settings::saved(directory.path()));
        let mut settings = Settings::default();
        // The typo checker stays off until the welcome page chooses a provider.
        assert_eq!(settings.typo.provider, TypoProvider::Off);
        settings.typo = TypoSettings::recommended();
        assert_eq!(settings.typo.provider, TypoProvider::D1Local);
        assert_eq!(settings.typo.d1_device, D1Device::Cuda);
        assert_eq!(settings.typo.calibration_key(), Some("local:d1-2b:cuda"));
        settings.save(directory.path()).unwrap();
        assert!(Settings::saved(directory.path()));
    }

    #[test]
    fn preserves_the_remote_d1_setting_and_saves_the_local_mode_without_a_url() {
        let prior: Settings = serde_json::from_str(r#"{"typo":{"provider":"d1"}}"#).unwrap();
        assert_eq!(prior.typo.provider, TypoProvider::D1);
        assert_eq!(prior.typo.d1_device, D1Device::Cpu);
        assert!(prior.typo.endpoint().is_empty());
        assert!(prior.typo.calibration_key().is_none());
        assert!(prior.typo.api_key_scope().is_none());
        let directory = tempfile::tempdir().unwrap();
        prior.save(directory.path()).unwrap();
        assert_eq!(Settings::load(directory.path()).unwrap(), prior);
        let mut local = prior;
        local.typo.provider = TypoProvider::D1Local;
        local.typo.d1_device = D1Device::Vulkan;
        local.save(directory.path()).unwrap();
        assert_eq!(
            Settings::load(directory.path()).unwrap().typo.provider,
            TypoProvider::D1Local
        );
        assert_eq!(
            Settings::load(directory.path()).unwrap().typo.d1_device,
            D1Device::Vulkan
        );
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn saves_only_the_protected_key_for_the_configured_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.typo.provider = TypoProvider::D1;
        settings.typo.url = "http://127.0.0.1:8080/v1/systemone".into();
        let key = "synthetic-settings-key";
        let scope = settings.typo.api_key_scope().unwrap();
        settings.typo.protected_api_keys.insert(
            scope.clone(),
            crate::credentials::protect(key, &scope).unwrap(),
        );
        settings.save(directory.path()).unwrap();
        assert!(
            !fs::read_to_string(directory.path().join("settings.json"))
                .unwrap()
                .contains(key)
        );
        let mut loaded = Settings::load(directory.path()).unwrap();
        assert_eq!(loaded.typo.api_key().unwrap(), key);
        loaded.typo.url = "http://127.0.0.1:8081/v1/systemone".into();
        assert!(
            !loaded
                .typo
                .protected_api_keys
                .contains_key(&loaded.typo.api_key_scope().unwrap())
        );
    }

    #[test]
    fn persists_distinct_server_cpu_and_gpu_limits() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.typo.provider = TypoProvider::D1;
        settings.typo.url = "http://127.0.0.1:8080/v1/systemone".into();
        let remote = settings.typo.calibration_key().unwrap().to_owned();
        settings
            .typo
            .calibration
            .entry(remote)
            .or_default()
            .overrides
            .concurrent = Some(8);
        settings.typo.provider = TypoProvider::D1Local;
        let cpu = settings.typo.calibration_key().unwrap().to_owned();
        settings
            .typo
            .calibration
            .entry(cpu)
            .or_default()
            .overrides
            .concurrent = Some(2);
        settings.typo.d1_device = D1Device::Vulkan;
        let gpu = settings.typo.calibration_key().unwrap().to_owned();
        settings
            .typo
            .calibration
            .entry(gpu)
            .or_default()
            .overrides
            .concurrent = Some(4);
        settings.typo.d1_device = D1Device::Cuda;
        let cuda = settings.typo.calibration_key().unwrap().to_owned();
        settings
            .typo
            .calibration
            .entry(cuda)
            .or_default()
            .overrides
            .concurrent = Some(3);
        settings.save(directory.path()).unwrap();
        let mut loaded = Settings::load(directory.path()).unwrap();
        assert_eq!(loaded.typo.d1_device, D1Device::Cuda);
        assert_eq!(loaded.typo.limits().concurrent, 3);
        loaded.typo.d1_device = D1Device::Vulkan;
        assert_eq!(loaded.typo.limits().concurrent, 4);
        loaded.typo.d1_device = D1Device::Cpu;
        assert_eq!(loaded.typo.limits().concurrent, 2);
        loaded.typo.provider = TypoProvider::D1;
        assert_eq!(loaded.typo.limits().concurrent, 8);
    }
}

impl Destination {
    pub fn label(self) -> &'static str {
        match self {
            Self::Osc => "VRChat OSC",
            Self::SteamVrInput => tr("SteamVR アプリ入力"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypoProvider {
    #[default]
    Off,
    D1Local,
    D1,
    Jev,
    Custom,
}

impl TypoProvider {
    pub const ALL: [Self; 5] = [Self::Off, Self::D1Local, Self::D1, Self::Jev, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => tr("無効"),
            Self::D1Local => tr("d1-2B（ローカル）"),
            Self::D1 => tr("d1-2B（API）"),
            Self::Jev => "TypeSafe Jev",
            Self::Custom => tr("System One 互換"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum D1Device {
    #[default]
    Cpu = 1,
    Vulkan = 2,
    Cuda = 3,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct TypoSettings {
    pub provider: TypoProvider,
    pub d1_device: D1Device,
    pub url: String,
    pub model: String,
    pub calibration: BTreeMap<String, Profile>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub protected_api_keys: BTreeMap<String, String>,
}

impl TypoSettings {
    /// What the welcome page proposes: the built-in d1-2B on an NVIDIA GPU.
    pub fn recommended() -> Self {
        Self {
            provider: TypoProvider::D1Local,
            d1_device: D1Device::Cuda,
            ..Self::default()
        }
    }

    pub fn calibration_key(&self) -> Option<&str> {
        match self.provider {
            TypoProvider::D1Local => Some(match self.d1_device {
                D1Device::Cpu => "local:d1-2b:cpu",
                D1Device::Vulkan => "local:d1-2b:vulkan",
                D1Device::Cuda => "local:d1-2b:cuda",
            }),
            TypoProvider::D1 | TypoProvider::Custom if !self.endpoint().is_empty() => {
                Some(self.endpoint())
            }
            _ => None,
        }
    }

    pub fn limits(&self) -> Limits {
        self.calibration_key()
            .and_then(|key| self.calibration.get(key))
            .map_or_else(Limits::default, Profile::limits)
    }
    pub fn endpoint(&self) -> &str {
        match self.provider {
            TypoProvider::Off | TypoProvider::D1Local => "",
            TypoProvider::D1 | TypoProvider::Custom => self.url.trim(),
            TypoProvider::Jev => "https://api.typesafe.ai/v1/systemone",
        }
    }

    pub fn model(&self) -> Option<&str> {
        match self.provider {
            TypoProvider::Jev => Some("jev-latest"),
            TypoProvider::Custom if !self.model.trim().is_empty() => Some(self.model.trim()),
            _ => None,
        }
    }

    pub fn key_variable(&self) -> &'static str {
        match self.provider {
            TypoProvider::Jev => "TYPESAFE_API_KEY",
            TypoProvider::D1 => "VIME_D1_API_KEY",
            _ => "VIME_SYSTEM_ONE_API_KEY",
        }
    }

    pub fn api_key_scope(&self) -> Option<String> {
        (!self.endpoint().is_empty())
            .then(|| format!("{}:{}", self.key_variable(), self.endpoint()))
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    pub fn api_key(&self) -> io::Result<String> {
        if let Some(scope) = self.api_key_scope()
            && let Some(protected) = self.protected_api_keys.get(&scope)
        {
            return crate::credentials::unprotect(protected, &scope);
        }
        Ok(env::var(self.key_variable()).unwrap_or_default())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardTheme {
    Light,
    #[default]
    Dark,
}

impl KeyboardTheme {
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => tr("ライト"),
            Self::Dark => tr("ダーク"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub destination: Destination,
    #[serde(default)]
    pub osc_query: bool,
    pub osc_port: u16,
    pub osc_notify: bool,
    pub hotkey: String,
    pub overlay_width: f32,
    pub keyboard_theme: KeyboardTheme,
    pub language: Lang,
    pub typo: TypoSettings,
    pub report_typos: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Lang::system(),
            destination: Destination::Osc,
            osc_query: true,
            osc_port: 9000,
            osc_notify: false,
            hotkey: "Ctrl+Shift+F8".into(),
            overlay_width: 1.08,
            keyboard_theme: KeyboardTheme::default(),
            typo: TypoSettings::default(),
            report_typos: true,
        }
    }
}

impl Settings {
    pub fn directory() -> io::Result<PathBuf> {
        env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("ViME"))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, tr("LOCALAPPDATA が見つかりません"))
            })
    }

    /// Whether settings were ever saved here. Without them ViME has not been set up.
    pub fn saved(path: &Path) -> bool {
        path.join("settings.json").exists()
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
                tr("OSC ポートまたはキーボード幅が範囲外です"),
            ));
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        if matches!(
            self.typo.provider,
            TypoProvider::D1 | TypoProvider::Jev | TypoProvider::Custom
        ) && !(self.typo.provider == TypoProvider::D1 && self.typo.endpoint().is_empty())
            && !crate::system_one::valid_url(self.typo.endpoint())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                tr("System One の HTTP または HTTPS URL を入力してください"),
            ));
        }
        Ok(())
    }
}
