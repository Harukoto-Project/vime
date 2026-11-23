# ViME

A virtual keyboard with IME for SteamVR. Type in Japanese, English, and Korean in VRChat.

[Features](#features) · [Setup](#setup) · [Usage](#usage) · [License](#license)

[日本語](README.md) · [English](README.en.md) · [한국어](README.ko.md)

## Features

- **Japanese** converts romaji to hiragana and offers katakana and kanji candidates. More candidates are available when Microsoft IME is installed.
- **English** uses QWERTY. **Korean** composes Hangul with the Dubeolsik layout. Pick emoji, kaomoji, and symbols from the palette, or look them up with readings in Japanese and Korean and shortcodes such as `:cat:` in English.
- **Windows settings** cover the output route, OSC port and sound, toggle shortcut, and keyboard width in VR. The keyboard preview works before connecting to SteamVR.

## Setup

Requires Windows x64, Rust 1.95 or later, and MSVC C++ Build Tools. Install SteamVR to use the keyboard in VR.

```powershell
cargo run --release --locked
```

The executable is built at `target/release/vime.exe`. Set `STEAMVR_PATH` to the SteamVR installation directory if it is installed in a custom location.

## Usage

1. Start SteamVR and select **SteamVR に接続** in the ViME Windows app.
2. Toggle the keyboard with **Ctrl+Shift+F8**, right **B** on Index or Touch, or left **Menu** on Vive Wand. Aim at a key and press the controller trigger. Select **コントローラーバインドを編集** to edit controller bindings in SteamVR.
3. Choose an output route in Windows settings and save.
   - **VRChat OSC** (default) posts to the VRChat Chatbox with **送信** after OSC is enabled in VRChat. The default local UDP port is 9000.
   - **SteamVR アプリ入力** sends Unicode keyboard events to the active VR application. Open a text field and keep that application in the Windows foreground. The application must accept Windows text input events.
4. Keep the ViME Windows app open during a VR session. Settings are saved at `%LOCALAPPDATA%/ViME/settings.json`.

This is a prototype. Headset rendering, controller bindings on each device, and receipt in VRChat still need live verification.

## License

[Apache License 2.0](LICENSE) · [Contributing](CONTRIBUTING.md)

The SteamVR integration uses Valve’s public [OpenVR API](https://github.com/ValveSoftware/openvr). The application loads Windows fonts from the device.
