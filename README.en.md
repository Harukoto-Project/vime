# ViME

A virtual keyboard with IME for SteamVR. Type in Japanese, English, and Korean in VRChat.

[Features](#features) · [Setup](#setup) · [Usage](#usage) · [License](#license)

[日本語](README.md) · [English](README.en.md) · [한국어](README.ko.md)

## Features

- **Japanese** converts romaji to hiragana and offers katakana and kanji candidates. More candidates are available when Microsoft IME is installed.
- **English** uses QWERTY. **Korean** composes Hangul with the Dubeolsik layout. Pick emoji, kaomoji, and symbols from the palette, or look them up with readings in Japanese and Korean and shortcodes such as `:cat:` in English.
- **Windows settings** cover the output route, OSC port and sound, toggle shortcut, and keyboard width in VR. The keyboard preview works before connecting to SteamVR.
- The **ViME** tab in the SteamVR dashboard offers the keyboard, output settings, and controller bindings.

## Setup

Requires Windows x64, Rust 1.95 or later, and MSVC C++ Build Tools. Install SteamVR to use the keyboard in VR.

```powershell
cargo run --release --locked
```

The executable is built at `target/release/vime.exe`. Set `STEAMVR_PATH` to the SteamVR installation directory if it is installed in a custom location.

## Usage

1. ViME connects on startup when SteamVR is running. If you start SteamVR later, select **SteamVR に接続** in the Windows app.
2. Click the **left thumbstick** (left trackpad on Vive Wand) or press **Ctrl+Shift+F8** to open the keyboard in front of you. Use the same shortcut to close it. Aim at a key and press the trigger. Open the SteamVR binding editor from the Windows app or dashboard. Restore ViME's default binding in SteamVR to adopt the new shortcut if you use a custom binding.
3. Choose an output route in the **ViME** dashboard tab or Windows **設定** tab and save.
   - **VRChat OSC** (default) posts to the VRChat Chatbox with **送信** after OSC is enabled in VRChat. The default local UDP port is 9000.
   - **SteamVR アプリ入力** sends Unicode keyboard events to the active VR application. Open a text field and keep that application in the Windows foreground. The application must accept Windows text input events.
4. Keep the ViME Windows app open during a VR session. Settings are saved at `%LOCALAPPDATA%/ViME/settings.json`.

This is a prototype. Headset rendering, controller bindings on each device, and receipt in VRChat still need live verification.

## License

[Apache License 2.0](LICENSE) · [Contributing](CONTRIBUTING.md)

The SteamVR integration uses Valve’s public [OpenVR API](https://github.com/ValveSoftware/openvr). The application loads Windows fonts from the device.
