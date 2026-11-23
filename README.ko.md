# ViME

SteamVR용 가상 IME 키보드입니다. VRChat에서 일본어, 영어, 한국어를 입력할 수 있습니다.

[기능](#기능) · [설치](#설치) · [사용법](#사용법) · [라이선스](#라이선스)  
[日本語](README.md) · [English](README.en.md) · [한국어](README.ko.md)

## 기능

- **일본어**는 로마자를 히라가나로 바꾸고 가타카나와 한자 후보를 보여 줍니다. Windows에 Microsoft IME가 설치되어 있으면 더 많은 후보를 사용할 수 있습니다.
- **영어**는 QWERTY, **한국어**는 두벌식으로 입력합니다. 목록에서 이모지, 얼굴 문자, 기호를 고르거나, 일본어·한국어 입력과 영어 단축 이름(`:cat:` 등)으로 후보를 찾을 수 있습니다.
- **Windows 설정**에서 전송 방식, OSC 포트와 알림음, 열기 단축키, VR 키보드 너비를 바꿀 수 있습니다. SteamVR에 연결하기 전에도 미리보기에서 키보드를 사용해 볼 수 있습니다.

## 설치

Windows x64, Rust 1.95 이상, MSVC C++ Build Tools가 필요합니다. VR에서 사용하려면 SteamVR을 설치하세요.

```powershell
cargo run --release --locked
```

실행 파일은 `target/release/vime.exe`에 생성됩니다. SteamVR을 다른 위치에 설치했다면 `STEAMVR_PATH`에 설치 폴더를 지정할 수 있습니다.

## 사용법

1. SteamVR을 실행하고 ViME Windows 앱에서 **SteamVR に接続**을 선택합니다.
2. **Ctrl+Shift+F8**, Index·Touch의 오른쪽 **B**, Vive Wand의 왼쪽 **Menu**로 키보드를 열고 닫습니다. 키를 가리킨 뒤 컨트롤러 트리거를 눌러 입력합니다. **コントローラーバインドを編集**에서 SteamVR의 컨트롤러 할당 설정을 열 수 있습니다.
3. Windows 설정에서 전송 방식을 선택하고 저장합니다.
   - **VRChat OSC**(기본값)는 VRChat에서 OSC를 활성화한 뒤 **送信**을 누르면 Chatbox에 바로 전송합니다. 기본 로컬 UDP 포트는 9000입니다.
   - **SteamVR アプリ入力**은 실행 중인 VR 앱에 유니코드 키보드 입력을 보냅니다. 앱의 입력란을 열고 Windows에서 해당 앱을 전면에 둡니다. 앱이 Windows 문자 입력 이벤트를 처리해야 합니다.
4. VR에서 사용하는 동안 ViME Windows 앱을 열어 두세요. 설정은 `%LOCALAPPDATA%/ViME/settings.json`에 저장됩니다.

이 버전은 프로토타입입니다. 헤드셋 화면, 기기별 컨트롤러 할당, VRChat 수신은 실제 기기에서 확인해야 합니다.

## 라이선스

[Apache License 2.0](LICENSE) · [기여 안내](CONTRIBUTING.md)

SteamVR 연동에는 Valve의 공개 [OpenVR API](https://github.com/ValveSoftware/openvr)를 사용합니다. Windows 글꼴은 기기에 설치된 파일을 불러옵니다.
