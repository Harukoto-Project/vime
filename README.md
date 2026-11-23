# ViME

SteamVR向けの仮想IMEキーボード。VRChatで日本語・英語・韓国語を入力できます。

[機能](#機能) · [セットアップ](#セットアップ) · [使い方](#使い方) · [ライセンス](#ライセンス)

[日本語](README.md) · [English](README.en.md) · [한국어](README.ko.md)

## 機能

- **日本語**はJIS配列で、ローマ字からひらがな・カタカナ・漢字候補に変換します。**半角/全角**でJIS配列のまま半角英数字の直接入力に切り替えられます。WindowsのMicrosoft IMEが使える環境では追加の変換候補が出ます。
- **English**はQWERTY、**한국어**は두벌식（2ボル式）で入力できます。絵文字・顔文字・記号は一覧から選べます。日本語・韓国語は読み、英語は `:cat:` のような短縮名から候補を出せます。
- **Windows GUI**で送信先、OSCポートと通知音、開閉ショートカット、VR上のキーボード幅を設定できます。SteamVRを接続する前でもキーボードを試せます。
- **SteamVRダッシュボード**の **ViME** タブからも、キーボード、送信設定、コントローラー割り当てを操作できます。

## セットアップ

Windows x64、Rust 1.95以降、MSVC C++ Build Toolsが必要です。VRで使う場合はSteamVRをインストールしてください。

```powershell
cargo run --release --locked
```

ビルド済みの実行ファイルは `target/release/vime.exe` に生成されます。SteamVRが別の場所にある場合は、`STEAMVR_PATH` にSteamVRのルートフォルダを指定できます。

## 使い方

1. SteamVRが起動中なら、ViMEは起動時に接続します。SteamVRを後から起動した場合は、Windows画面で **SteamVR に接続** を選びます。
2. **右ジョイスティック押し込み**（Vive Wandは右トラックパッド押し込み）または **Ctrl+Shift+F8** で、キーボードを顔の正面に開閉します。左右どちらのコントローラーでもキーを指し、トリガーを押した時点で入力できます。割り当てはWindows画面やダッシュボードからSteamVRの編集画面を開いて変更できます。既存のカスタム割り当てで新しい既定値を使うには、SteamVRでViMEの割り当てを既定に戻します。
3. SteamVRダッシュボードの **ViME** タブ、またはWindows画面の **設定** で送信先を選び、保存します。
   - **VRChat OSC**（既定）はVRChatでOSCを有効にすると、**送信**でChatboxに直接投稿します。ローカルUDPポートは既定で9000です。
   - **SteamVR アプリ入力**は、現在のVRアプリをWindowsで前面にし、そのアプリの文字入力欄を開いた状態で使います。**送信**でUnicodeキーボード入力を渡します。アプリ側がWindowsの文字入力イベントを受け付ける必要があります。
4. VRで利用中は、ViMEのWindowsウィンドウを開いたままにしてください。設定は `%LOCALAPPDATA%/ViME/settings.json` に保存されます。

本バージョンは試作版です。HMD上の表示、コントローラーごとのバインド、VRChatでの受信は実機検証が必要です。

## ライセンス

[Apache License 2.0](LICENSE) · [コントリビュート](CONTRIBUTING.md)

SteamVR連携はValveの公開[OpenVR API](https://github.com/ValveSoftware/openvr)を利用します。Windowsのフォントは端末から読み込みます。
