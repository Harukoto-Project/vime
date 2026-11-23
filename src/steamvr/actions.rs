use serde_json::{Value, json};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub const APP_KEY: &std::ffi::CStr = c"org.harukoto.vime";
pub const DASHBOARD_KEY: &std::ffi::CStr = c"org.harukoto.vime.dashboard";
pub const TOGGLE_SET: &std::ffi::CStr = c"/actions/vime";
pub const POINTER_SET: &std::ffi::CStr = c"/actions/keyboard";
pub const TOGGLE: &std::ffi::CStr = c"/actions/vime/in/toggle";
pub const AIM: &std::ffi::CStr = c"/actions/keyboard/in/aim";
pub const CLICK: &std::ffi::CStr = c"/actions/keyboard/in/click";

pub fn runtime_dll() -> Option<PathBuf> {
    let registered = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .and_then(|path| fs::read(path.join("openvr/openvrpaths.vrpath")).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let registered = registered
        .as_ref()
        .and_then(|value| value["runtime"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from);
    let standard = env::var_os("ProgramFiles(x86)")
        .map(|root| PathBuf::from(root).join("Steam/steamapps/common/SteamVR"));
    env::var_os("STEAMVR_PATH")
        .map(PathBuf::from)
        .into_iter()
        .chain(registered)
        .chain(standard)
        .map(|path| path.join("bin/win64/openvr_api.dll"))
        .find(|path| path.is_absolute() && path.is_file())
}

pub fn write_manifests(directory: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let directory = directory.join("steamvr");
    fs::create_dir_all(&directory)?;

    let mut defaults = Vec::new();
    for (controller, control, mode) in [
        ("knuckles", "thumbstick", "joystick"),
        ("oculus_touch", "joystick", "joystick"),
        ("vive_controller", "trackpad", "button"),
    ] {
        let name = format!("binding_{controller}_v2.json");
        let inputs = |side: &str, control: &str, mode: &str, action: &str| {
            json!({
                "path": format!("/user/hand/{side}/input/{control}"),
                "mode": mode,
                "inputs": { "click": { "output": action } }
            })
        };
        let poses: Vec<_> = ["left", "right"]
            .into_iter()
            .map(|side| {
                json!({
                    "path": format!("/user/hand/{side}/pose/tip"),
                    "output": AIM.to_str().unwrap()
                })
            })
            .collect();
        write_json(
            &directory.join(&name),
            &json!({
                "app_key": APP_KEY.to_str().unwrap(),
                "controller_type": controller,
                "name": "ViME",
                "description": "Toggle ViME and select keyboard keys",
                "bindings": {
                    (TOGGLE_SET.to_str().unwrap()): {
                        "sources": [inputs("left", control, mode, TOGGLE.to_str().unwrap())]
                    },
                    (POINTER_SET.to_str().unwrap()): {
                        "poses": poses,
                        "sources": [
                            inputs("left", "trigger", "button", CLICK.to_str().unwrap()),
                            inputs("right", "trigger", "button", CLICK.to_str().unwrap())
                        ]
                    }
                }
            }),
        )?;
        defaults.push(json!({ "controller_type": controller, "binding_url": name }));
    }

    let actions = directory.join("actions.json");
    write_json(
        &actions,
        &json!({
            "action_sets": [
                { "name": TOGGLE_SET.to_str().unwrap(), "usage": "single" },
                { "name": POINTER_SET.to_str().unwrap(), "usage": "leftright" }
            ],
            "actions": [
                { "name": TOGGLE.to_str().unwrap(), "type": "boolean", "requirement": "optional" },
                { "name": AIM.to_str().unwrap(), "type": "pose", "requirement": "optional" },
                { "name": CLICK.to_str().unwrap(), "type": "boolean", "requirement": "optional" }
            ],
            "default_bindings": defaults,
            "localization": [{
                "language_tag": "en_us",
                (TOGGLE_SET.to_str().unwrap()): "ViME",
                (POINTER_SET.to_str().unwrap()): "Keyboard",
                (TOGGLE.to_str().unwrap()): "Toggle keyboard",
                (AIM.to_str().unwrap()): "Aim at keyboard",
                (CLICK.to_str().unwrap()): "Press key"
            }]
        }),
    )?;

    let application = directory.join("vime.vrmanifest");
    let executable = env::current_exe()?;
    write_json(
        &application,
        &json!({
            "source": "builtin",
            "applications": [{
                "app_key": APP_KEY.to_str().unwrap(),
                "launch_type": "binary",
                "binary_path_windows": executable,
                "action_manifest_path": "actions.json",
                "is_overlay": true,
                "strings": { "en_us": { "name": "ViME", "description": "Virtual keyboard with IME" } }
            }]
        }),
    )?;
    Ok((application, actions))
}

fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), value)?;
    temporary.persist(path)?;
    Ok(())
}
