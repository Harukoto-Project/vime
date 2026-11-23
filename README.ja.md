# ViME

[English](README.md) · 日本語 · [한국어](README.ko.md)

ViMEは、VR内で日本語・英語・韓国語を入力できる、Windows向けのオープンソースIMEキーボードです。コントローラーでキーを押し、入力した文章をVRChatのChatboxや対応するSteamVRアプリに送れます。

**[Windows版のダウンロード（GitHub Releases）](https://github.com/Harukoto-Project/vime/releases)**

![ViMEのVRキーボード。「こんにちは、ViMEです。」と入力した状態で、上部に言語切り替え、左下に絵文字ボタンがある。](assets/vr-keyboard.png)

## インストール

1. [Releases](https://github.com/Harukoto-Project/vime/releases) から `ViME-<バージョン>-windows-x64.zip` をダウンロードします。
2. ZIPを展開し、`vime.exe` を起動します。配布版はWindows x64向けで、Rustのインストールは不要です。
3. VRで使うときはSteamVRを起動します。ViMEが先に起動していた場合は、Windows画面の **SteamVR に接続** を押してください。Windows画面のキーボードプレビューは、SteamVRに接続する前でも試せます。

配布ZIPが見つからない場合は、[ソースから起動する](#ソースから起動する)手順を使ってください。

## VRChatに送る

1. VRChatのAction Menuで **OSC → Enabled** をオンにします。ViMEの送信先は既定の **VRChat OSC** を使用します。VRChatの受信ポートを変更している場合は、ViMEでも同じポートを設定してください。既定は `9000` です。 [VRChatのOSC公式ガイド](https://docs.vrchat.com/docs/osc-overview)
2. **右スティック押し込み**（Vive Wandは**右トラックパッド押し込み**）か、PCの **Ctrl+Shift+F8** でキーボードを開きます。左右どちらのコントローラーでも、キーを指してトリガーを押すと入力できます。
3. 文章を入力し、キーボードの **Enter** でChatboxに送ります。変換中は1回目の **Enter** で文字を確定し、もう一度押すと送信します。

VRで使っている間はViMEを起動したままにしてください。SteamVRダッシュボードの **ViME** タブからも、キーボード、送信設定、コントローラーの割り当てを開けます。

Windows画面の **設定** では、OSCの通知音、開閉ショートカット、VRキーボードの幅を変更できます。設定は `%LOCALAPPDATA%\ViME\settings.json` に保存されます。

## 言語の切り替えと変換

- **日本語**はJIS配列でローマ字入力できます。スペースキーの **変換** で漢字やカタカナの候補が出ます。数字と英字は半角のまま、記号は全角で、`,` は `、`、`.` は `。` になります。記号とカタカナの半角表記も変換候補から選べます。**半/全** は半角の直接入力への切り替えです。
- **English** はQWERTY、**한국어** は두벌식（2ボル式）です。韓国語の **한/영** で英数字の直接入力に切り替えられます。
- **左下の 😃** で絵文字・顔文字・記号の一覧を開きます。英語入力では `:cat:` のような短縮名からも絵文字を選べます。

![日本語の変換欄に、全角の「パーティー」と半角の「ﾊﾟｰﾃｨｰ」が並ぶ。](assets/vr-conversion.png)

## ほかのSteamVRアプリで使う

ViMEの **設定** で、送信先を **SteamVR アプリ入力** に変更して保存します。対象のVRアプリで文字入力欄を開き、そのアプリをWindowsの前面にした状態で、VR側のキーボードから **Enter** を押します。対象アプリがWindowsの文字入力イベントを受け付ける必要があります。

## うまく動かないとき

- **コントローラーで開かない**場合は、まず **Ctrl+Shift+F8** で開けるか確認してください。コントローラーの割り当てはViMEからSteamVRの編集画面を開いて変更できます。カスタム割り当てを使っている場合は、SteamVRでViMEの既定の割り当てに戻せます。
- **Chatboxに届かない**場合は、VRChatのOSCが有効で、ViMEの送信先が **VRChat OSC** になっているか、受信ポートと一致しているかを確認してください。
- **SteamVRが見つからない**場合は、SteamVRを起動してから再接続してください。別の場所にインストールしている場合は、環境変数 `STEAMVR_PATH` にインストールフォルダを指定できます。

ViMEは試作版です。ヘッドセットやコントローラーごとの動作、VRChatでの受信は実機検証が必要です。不具合の報告には、ViMEのバージョン、機器名、再現手順を添えてください。 [GitHub Issues](https://github.com/Harukoto-Project/vime/issues)

## ソースから起動する

Windows x64、Rust 1.95以降、MSVC C++ Build Toolsが必要です。リポジトリのルートで実行します。

```powershell
cargo run --release --locked
```

実行ファイルは `target/release/vime.exe` に生成されます。開発・変更の提出については [コントリビュートガイド](CONTRIBUTING.md) を参照してください。

### 日本語の修正候補を設定する

Windows x64 の Python 3.12 と DirectML 対応 GPU を用意し、ViME の誤字修正に **d1-2B（ローカル）** を選びます。**d1-2B と日本語モデルをダウンロード**、またはウェルカムページの **おすすめ設定で始める** を選ぶと、両方の準備が同時に始まります。d1-2B を取得済みの場合は **日本語モデルをダウンロード** が表示されます。入力中の推論はローカルで実行します。

ソースから手動でモデルを準備する場合は、リポジトリのルートで次を実行し、ViME を再起動します。

```powershell
$vimeTypoDir = Join-Path $env:LOCALAPPDATA 'ViME\japanese-typo'
py -3.12 -m venv "$vimeTypoDir\venv"
$vimeTypoPython = Join-Path $vimeTypoDir 'venv\Scripts\python.exe'
& $vimeTypoPython -m pip install onnxruntime-directml==1.24.4 sentencepiece==0.2.1 fugashi unidic-lite regex onnx onnxconverter-common transformers==4.57.6
& $vimeTypoPython -m pip install --extra-index-url https://download.pytorch.org/whl/cpu 'torch==2.7.1+cpu'
& $vimeTypoPython tools/prepare_japanese_typo.py
```

ウェルカムページの **または、自分で選ぶ** と、通常の **設定** の両方で **日本語の修正候補を表示する** を切り替えられます。**日本語の修正候補の詳細設定** では、検出しきい値（既定 `0.01`）、候補数（既定 `5`）、カーソルの左右それぞれの文字数（既定 `40`）、d1-2B で判定する最大位置数（既定 `2`）を変更できます。これらの値は `settings.json` の `typo.japanese` に保存されます。GPU が複数ある場合は、起動前に `VIME_JAPANESE_TYPO_ADAPTER` で DirectML のアダプター番号を指定できます。

ウォームアップ後の DistilBERT、d1-2B、全体の処理時間を測るには、次を実行します。

```powershell
cargo test --locked japanese_typo_benchmark -- --ignored --nocapture
```

## ライセンス

[Apache License 2.0](LICENSE)

### 要求したときだけ取得するデータ

ViME には日本語Wikipedia入力誤りデータセットの複製は含まれません。設定の「日本語の誤変換データ」で **ダウンロード** を押すと、ViME が京都大学から [JWTD v2](https://nlp.ist.i.kyoto-u.ac.jp/EN/index.php?JWTD)（京都大学 言語メディア研究室、[CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/deed.ja)）を直接取得し、編集者が直した漢字の語を集計します。残るのはその語の表だけで、`%LOCALAPPDATA%\ViME\misconversions.tsv` に保存されます。この表は共有されず、CC BY-SA 3.0 のままです。出典: 田中佑, 村脇有吾, 河原大輔, 黒橋禎夫. 日本語Wikipediaの編集履歴に基づく入力誤りデータセットと訂正システムの改良. 言語処理学会第27回年次大会, 2021.
