# <img src="assets/icon.png" width="36" height="36" alt=""> ViME

ViME는 Windows에서 실행되는 SteamVR용 오픈 소스 IME 키보드입니다. VR 컨트롤러로 일본어, 영어, 한국어를 입력하고 VRChat Chatbox나 호환되는 SteamVR 앱으로 보낼 수 있습니다.

**[Windows 버전 다운로드(GitHub Releases)](https://github.com/Harukoto-Project/vime/releases)** · [日本語](README.md) · [English](README.en.md) · 한국어

![일본어 인사말을 입력한 ViME VR 키보드. 위쪽에는 언어 선택 탭, 왼쪽 아래에는 이모지 버튼이 있다.](assets/vr-keyboard.png)

## 설치

1. [Releases](https://github.com/Harukoto-Project/vime/releases)에서 `ViME-<버전>-windows-x64.zip`을 다운로드합니다.
2. 압축을 풀고 `vime.exe`를 실행합니다. 배포 버전은 Windows x64용이며 Rust를 설치할 필요가 없습니다.
3. VR에서 사용하려면 SteamVR을 실행합니다. ViME를 먼저 실행했다면 Windows 앱에서 **SteamVR에 연결**을 누르세요. 연결하기 전에도 Windows 앱의 키보드 미리보기에서 입력을 시험해 볼 수 있습니다.

배포 ZIP이 없다면 [소스에서 실행](#소스에서-실행)할 수 있습니다.

## VRChat에 메시지 보내기

1. VRChat의 Action Menu에서 **OSC → Enabled**를 켭니다. ViME의 전송 대상은 기본값인 **VRChat OSC**를 사용합니다. VRChat의 수신 포트를 바꿨다면 ViME에도 같은 포트를 설정하세요. 기본값은 `9000`입니다. [VRChat OSC 공식 가이드](https://docs.vrchat.com/docs/osc-overview)
2. **오른쪽 스틱**(Vive Wand는 **오른쪽 트랙패드**)을 누르거나 PC에서 **Ctrl+Shift+F8**을 눌러 키보드를 엽니다. 어느 쪽 컨트롤러로든 키를 가리키고 트리거를 누르면 입력됩니다.
3. 메시지를 입력한 뒤 VR 키보드의 **Enter**를 눌러 Chatbox로 보냅니다. 변환 중에는 첫 번째 **Enter**로 글자를 확정하고, 한 번 더 누르면 전송합니다.

VR에서 사용하는 동안 ViME를 계속 실행해 두세요. SteamVR 대시보드의 **ViME** 탭에서도 키보드와 전송 설정을 열고 컨트롤러 할당을 편집할 수 있습니다.

Windows 앱의 **설정**에서 OSC 알림음, 키보드 열기 및 닫기 단축키, VR 키보드 너비를 바꿀 수 있습니다. 설정은 `%LOCALAPPDATA%\ViME\settings.json`에 저장됩니다.

## 언어 전환 및 문자 변환

- **일본어**는 JIS 배열에서 로마자로 입력합니다. 스페이스 키를 누르면 한자와 가타카나 변환 후보가 표시됩니다. 숫자와 영문은 반각을 유지하고, 기호는 기본적으로 전각을 사용합니다. 쉼표와 마침표는 각각 `、`와 `。`로 입력됩니다. 변환 후보에서 반각 기호와 가타카나도 선택할 수 있습니다. **半/全** 키는 반각 직접 입력으로 전환합니다.
- **영어**는 QWERTY, **한국어**는 두벌식을 사용합니다. 한국어 모드에서 **한/영**을 누르면 영문과 숫자를 바로 입력할 수 있습니다.
- 왼쪽 아래의 **😃**를 누르면 이모지, 얼굴 문자, 기호 목록이 열립니다. 영어 모드에서는 `:cat:` 같은 단축 이름으로 이모지를 찾을 수도 있습니다.

![일본어 변환 후보에 전각 パーティー와 반각 ﾊﾟｰﾃｨｰ가 함께 표시된다.](assets/vr-conversion.png)

## 다른 SteamVR 앱에서 사용하기

ViME의 **설정**에서 전송 대상을 **SteamVR 앱 입력**으로 바꾸고 저장합니다. 대상 앱의 입력란을 열고 Windows에서 해당 앱을 전면에 둔 상태로 VR 키보드의 **Enter**를 누르세요. 대상 앱이 Windows 문자 입력 이벤트를 처리할 수 있어야 합니다.

## 문제 해결

- **컨트롤러로 키보드가 열리지 않으면** 먼저 **Ctrl+Shift+F8**로 열어 보세요. ViME에서 SteamVR 할당 편집기를 열어 컨트롤러 단축키를 바꾸거나 기본 할당을 복원할 수 있습니다.
- **Chatbox에 메시지가 나타나지 않으면** VRChat의 OSC 활성화 상태, ViME의 전송 대상 **VRChat OSC**, 수신 포트가 일치하는지 확인하세요.
- **SteamVR을 찾지 못하면** SteamVR을 실행한 뒤 다시 연결하세요. 다른 위치에 설치했다면 `STEAMVR_PATH` 환경 변수에 설치 폴더를 지정할 수 있습니다.

ViME는 프로토타입입니다. 헤드셋 화면, 컨트롤러별 할당, VRChat 수신은 실제 기기에서 검증이 필요합니다. 문제를 보고할 때 ViME 버전, 기기 모델, 재현 방법을 함께 알려 주세요. [GitHub Issues](https://github.com/Harukoto-Project/vime/issues)

## 소스에서 실행

Windows x64, Rust 1.95 이상, MSVC C++ Build Tools가 필요합니다. 저장소의 루트 디렉터리에서 실행하세요.

```powershell
cargo run --release --locked
```

실행 파일은 `target/release/vime.exe`에 생성됩니다. 개발과 변경 제출은 [기여 안내](CONTRIBUTING.md)를 참고하세요.

## 라이선스

[Apache License 2.0](LICENSE)
