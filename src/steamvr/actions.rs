use super::LAUNCH_ARGUMENT;
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
pub const CLOSE: &std::ffi::CStr = c"/actions/keyboard/in/close";
const CAPTURE: &str = "/actions/keyboard/in/capture";

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
        let name = format!("binding_{controller}_v4.json");
        let trigger = |hand: &str| {
            let mut binding = button(hand, "trigger", "button", "click", CLICK.to_str().unwrap());
            binding["parameters"] = json!({
                "click_activate_threshold": "0.45",
                "click_deactivate_threshold": "0.30",
            });
            binding
        };
        let mut keyboard_sources = vec![trigger("left"), trigger("right")];
        for hand in ["left", "right"] {
            let controls: &[&str] = match (controller, hand) {
                ("knuckles", _) => &["a", "b", "grip", "trackpad", "system"],
                ("oculus_touch", "left") => &["x", "y", "grip", "system"],
                ("oculus_touch", _) => &["a", "b", "grip"],
                ("vive_controller", _) => &["grip", "application_menu", "trackpad", "system"],
                _ => unreachable!(),
            };
            for &control in controls {
                let action =
                    if (controller, hand, control) == ("vive_controller", "right", "trackpad") {
                        CLOSE.to_str().unwrap()
                    } else {
                        CAPTURE
                    };
                let mut binding = button(hand, control, "button", "click", action);
                if controller == "knuckles" && matches!(control, "grip" | "trackpad") {
                    binding["parameters"] = json!({ "force_input": "force" });
                }
                keyboard_sources.push(binding);
            }
            if controller == "oculus_touch" {
                keyboard_sources.push(button(hand, "thumbrest", "button", "touch", CAPTURE));
            }
        }
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
                "description": "Toggle ViME, select keys, and reserve non-joystick controls while typing",
                "bindings": {
                    (TOGGLE_SET.to_str().unwrap()): {
                        "sources": [button("right", control, mode, "click", TOGGLE.to_str().unwrap())]
                    },
                    (POINTER_SET.to_str().unwrap()): {
                        "poses": poses,
                        "sources": keyboard_sources
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
                { "name": CLICK.to_str().unwrap(), "type": "boolean", "requirement": "optional" },
                { "name": CLOSE.to_str().unwrap(), "type": "boolean", "requirement": "optional" },
                { "name": CAPTURE, "type": "boolean", "requirement": "optional" }
            ],
            "default_bindings": defaults,
            "localization": [{
                "language_tag": "en_us",
                (TOGGLE_SET.to_str().unwrap()): "ViME",
                (POINTER_SET.to_str().unwrap()): "Keyboard",
                (TOGGLE.to_str().unwrap()): "Toggle keyboard",
                (AIM.to_str().unwrap()): "Aim at keyboard",
                (CLICK.to_str().unwrap()): "Press key",
                (CLOSE.to_str().unwrap()): "Close keyboard",
                (CAPTURE): "Block game input while typing"
            }]
        }),
    )?;

    let application = directory.join("vime.vrmanifest");
    write_application_manifest(&application)?;
    Ok((application, actions))
}

fn button(hand: &str, control: &str, mode: &str, input: &str, action: &str) -> Value {
    json!({
        "path": format!("/user/hand/{hand}/input/{control}"),
        "mode": mode,
        "inputs": { (input): { "output": action } }
    })
}

pub(super) fn write_application_manifest(application: &Path) -> io::Result<()> {
    let executable = env::current_exe()?;
    write_json(
        application,
        &json!({
            "source": "builtin",
            "applications": [{
                "app_key": APP_KEY.to_str().unwrap(),
                "launch_type": "binary",
                "binary_path_windows": executable,
                "arguments": LAUNCH_ARGUMENT,
                "action_manifest_path": "actions.json",
                "is_overlay": true,
                "is_dashboard_overlay": true,
                "strings": { "en_us": { "name": "ViME", "description": "Virtual keyboard with IME" } }
            }]
        }),
    )
}

fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), value)?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_the_current_executable_as_a_startup_overlay() {
        let directory = tempfile::tempdir().unwrap();
        let (application, actions) = write_manifests(directory.path()).unwrap();
        let manifest: Value = serde_json::from_slice(&fs::read(application).unwrap()).unwrap();
        let application = &manifest["applications"][0];
        assert_eq!(application["app_key"], APP_KEY.to_str().unwrap());
        assert_eq!(application["is_dashboard_overlay"], true);
        assert_eq!(
            application["binary_path_windows"],
            json!(env::current_exe().unwrap())
        );
        assert_eq!(application["arguments"], LAUNCH_ARGUMENT);
        assert_eq!(
            application["action_manifest_path"],
            actions.file_name().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn keyboard_bindings_leave_joysticks_available_to_the_game() {
        let directory = tempfile::tempdir().unwrap();
        let (_, manifest_path) = write_manifests(directory.path()).unwrap();
        let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let defaults = manifest["default_bindings"].as_array().unwrap();
        assert_eq!(defaults.len(), 3);
        let keyboard = POINTER_SET.to_str().unwrap();
        for default in defaults {
            let path = manifest_path.with_file_name(default["binding_url"].as_str().unwrap());
            let binding: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            let sources = binding["bindings"][keyboard]["sources"].as_array().unwrap();
            let input = |hand, control| {
                let path = format!("/user/hand/{hand}/input/{control}");
                sources.iter().find(|source| source["path"] == path)
            };
            for hand in ["left", "right"] {
                assert!(input(hand, "thumbstick").is_none());
                assert!(input(hand, "joystick").is_none());
                assert_eq!(
                    input(hand, "trigger").unwrap()["inputs"]["click"]["output"],
                    CLICK.to_str().unwrap()
                );
                assert_eq!(
                    input(hand, "grip").unwrap()["inputs"]["click"]["output"],
                    CAPTURE
                );
            }
            if default["controller_type"] == "vive_controller" {
                assert_eq!(
                    input("right", "trackpad").unwrap()["inputs"]["click"]["output"],
                    CLOSE.to_str().unwrap()
                );
            }
        }
    }
}
