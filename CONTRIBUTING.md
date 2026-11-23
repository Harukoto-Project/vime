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

## 誤字・誤変換の報告 API

配布用ビルドでは、GitHub Actions のリポジトリ変数 `VIME_TYPO_REPORT_URL` に運営者の HTTPS エンドポイントを設定します。手元でビルドする場合は、PowerShell で `$env:VIME_TYPO_REPORT_URL = 'https://reports.example.com/v1/typos'` を実行してから Cargo を実行します。URL が未設定、無効、または HTTP のビルドは報告を送信しません。

ViME は、誤字修正の判定が有効で、利用者が報告を許可している場合に、誤字または誤変換の修正と判定された打ち直しを JSON で POST します。本文は次の形式です。

```json
{
  "schema_version": 1,
  "app_version": "0.1.0",
  "language": "ja",
  "original": "異義",
  "replacement": "異議"
}
```

入力言語は `ja`、`en`、`ko` のいずれかです。報告の本文には前後の文脈や利用者 ID を含めません。修正前後の語には個人情報や機密情報が含まれる可能性があります。報告の受信データはアクセスを制限し、変換精度の分析と改善に使用してください。

受信側でスキーマと値を検証し、集計用ストレージに受信時刻を付けて 1 報告を 1 レコードとして保存できます。JSON Lines で出力する場合は、JSON オブジェクト 1 件を 1 行にしてください。保存に成功したら HTTP 204 などの 2xx を返します。クライアントは応答本文を使用しません。送信はバックグラウンドで行い、失敗した報告は端末に保存したり再送したりしません。以前のバージョンが保存した `%LOCALAPPDATA%\ViME\typos.json` は新しいバージョンで読み込みも送信もされません。

## セキュリティ上の問題

セキュリティに関わる問題は公開Issueではなく、GitHubのSecurity Advisory(Report a vulnerability)から非公開で報告してください。

## ライセンス

コントリビュートされた内容は、本プロジェクトと同じApache License 2.0で提供されるものとします。
