# コントリビュートについて

IssueとPull Requestを歓迎します。

## 開発環境

- Windows x64、MSVC C++ Build Tools
- Rust 1.95以降
- SteamVRとHMD（VR動作を確認する場合）

## 提出前の確認

- `cargo fmt --check` でRustコードの整形を確認します。
- `cargo clippy --locked --all-targets -- -D warnings` でLintを実行します。
- `cargo test --locked` で入力合成、OSC送信、オーバーレイの座標判定を検証します。OSCテストではループバックUDPを使用します。
- `cargo run --release --locked` でWindows GUIを確認します。VR連携を変更した場合は、HMD上で開閉、コントローラー操作、送信も確認します。

## バグ報告

次の情報があると調査しやすくなります。

- ViMEのバージョンまたはコミット、WindowsとSteamVRのバージョン、HMDとコントローラーの機種
- 再現手順、送信先、使用した入力言語、Windows画面またはVR画面に出たエラー
- OSC送信の場合は、VRChatのOSC有効状態と設定ポート

## セキュリティ上の問題

セキュリティに関わる問題は公開Issueではなく、GitHubのSecurity Advisory(Report a vulnerability)から非公開で報告してください。

## ライセンス

コントリビュートされた内容は、本プロジェクトと同じApache License 2.0で提供されるものとします。
