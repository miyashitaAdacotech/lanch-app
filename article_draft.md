---
title: "車輪の再開発とわかっていたものの、ランチャーソフトを作った話"
emoji: "🔧"
type: "tech"
topics: ["Rust", "ClaudeCode", "Windows", "egui", "個人開発"]
published: false
---

## はじめに

みんなこう言うだろう。**時間の無駄、車輪の再開発**だと。

でも個人的にはこういう結論に至った。「かゆいところまで手が届く、自分用にカスタマイズされたアプリのほうが整備もしやすいし、Claude Codeで実装するんだからそこまで時間かからないでしょ」と。

実際、first commitから5日で25コミット、約5,700行のRustコードと110のテストが揃った。自分一人でゼロから書いたわけじゃない。**Claude Codeと二人三脚で作った**。

この記事は、MacからWindowsに移行したベテランエンジニアが、Alfred相当のランチャーをRust + eguiで自作するまでの記録です。

---

## 第1章：Alfredが使えない世界

自分はMacで[Alfred](https://www.alfredapp.com/)（パワーパック）のヘビーユーザーだった。

新しい会社でWindowsで開発することになり、色々悪戦苦闘した。OS違い、ショートカット違い、ターミナル違い──でも一番困ったのは、**いつも使ってたランチャーソフトがMac専用**だったこと。

Alfredで毎日使っていた機能はこの3つ:

| 機能 | 用途 |
|------|------|
| **Clipboard History** | コピー履歴を検索・再利用（画像含む） |
| **翻訳** | 選択テキストをホットキーで即翻訳 |
| **テキスト整形** | 雑なテキストをMarkdown形式に整形 |

これがないと仕事にならない。代替を探すしかなかった。

---

## 第2章：Windows代替ツールの選定と挫折

似たようなツールを片っ端から試した。

| ツール | 良かった点 | ダメだった点 |
|--------|-----------|-------------|
| [Raycast](https://www.raycast.com/) | 求めてたものに一番近い | 挙動がおかしくなることがあり、β版ということもあり断念 |
| [Flow Launcher](https://www.flowlauncher.com/) | UIはきれい | Clipboard Historyが求めてたものと違った |
| [PowerToys](https://github.com/microsoft/PowerToys) | 安定している | Advanced Pasteは25件まで、検索なし、再起動で消える |

📸 **【画像】PowerToysのAdvanced Paste画面（Alt+Hで表示される画面）**

最終的に**全部断念**した。すべて一長一短。

「妙に重かったり落ちたりしない、要件通りの機能がほしい」──シンプルな要求のはずなのに、ぴったりハマるものがなかった。

---

## 第3章：ほしいもの定義

断念した末に、「じゃあ自分で作ろう」となった。ほしいものを整理するとこうなった:

### クリップボード履歴
- 画像も覚えておく（テキストだけじゃ足りない）
- 日をまたいで保持（最大1週間）
- キーワードと日付で検索できる
- 100件ずつページング、選択したら即ペースト

### 翻訳
- 選択テキストをホットキー一発で英語⇔日本語翻訳
- 日本語→英語のときはポップアップから入力

### Markdown整形
- 選択したテキスト（崩れたテーブル、雑なリスト）をMarkdown形式にきれいに整形
- Claude APIで整形するので精度が高い

---

## 第4章：技術選定

### なぜRustか

| 選択肢 | 却下理由 |
|--------|---------|
| Electron | 重い。クリップボード監視でメモリ食いすぎ |
| Python + tkinter | 配布が面倒。pyinstallerのexeはデカい |
| C# / WPF | 書けるけど、Rust書きたかった（正直） |
| **Rust + egui** | 軽い、シングルバイナリ、Win32 APIとの親和性が高い |

正直なところ、**Rustを書きたかった**のが一番の動機。でも結果的に、6.6MBのシングルバイナリで起動時メモリ12MB程度という軽さは正解だった。

### 主な依存クレート

```toml
[dependencies]
eframe = "0.31"          # GUI（イミディエイトモードGUI）
arboard = "3"            # クリップボード操作
windows-sys = "0.59"     # Win32 API（ホットキー、クリップボード監視）
reqwest = { version = "0.12", features = ["blocking", "json"] }  # 翻訳API
chrono = "0.4"           # タイムスタンプ
serde = "1"              # 設定ファイルのJSON読み書き
```

---

## 第5章：アーキテクチャ

```
lanch-app/
├── src/
│   ├── main.rs              # エントリポイント・CLI引数処理
│   ├── tray.rs              # システムトレイ常駐・ホットキー登録
│   ├── config.rs            # ~/.lanch-app/config.json の読み書き
│   ├── clipboard.rs         # クリップボード操作（コピー・ペースト）
│   ├── clipboard_history.rs # Win32 AddClipboardFormatListener で監視
│   ├── clipboard_store.rs   # JSON index + blobファイルでストレージ
│   ├── clipboard_ui.rs      # egui 検索UI（Alfred風ポップアップ）
│   ├── translator.rs        # Google Translate / DeepL API
│   ├── formatter.rs         # Claude API / CLI でMarkdown整形
│   ├── popup.rs             # 翻訳入力ポップアップ
│   ├── spinner.rs           # 処理中スピナー（36x36 最前面表示）
│   ├── notification.rs      # Windows通知
│   └── lang.rs              # 日本語判定
└── ~/.lanch-app/
    ├── config.json           # ユーザー設定
    └── clipboard-history/
        ├── index.json        # 履歴インデックス
        └── blobs/            # 画像バイナリ
```

設計原則は**1モジュール1責務**。Claude Codeに「SOLID, KISS, YAGNI, DRYで」と伝えてあるので、自然とこの構成になった。

---

## 第6章：Clipboard History──一番こだわった機能

### 監視の仕組み

Win32の`AddClipboardFormatListener`で、クリップボードの変更をリアルタイムに検知する。隠しメッセージウィンドウを作って、`WM_CLIPBOARDUPDATE`を受け取る方式。

```rust
// clipboard_history.rs（簡略化）
unsafe {
    let hwnd = CreateWindowExW(/* HWND_MESSAGE */);
    AddClipboardFormatListener(hwnd);

    // メッセージループ
    while GetMessageW(&mut msg, ...) > 0 {
        DispatchMessageW(&msg);
    }
}
```

変更を検知したら`arboard`でテキスト or 画像を読み取り、ストアに保存する。画像は自前のミニマルPNGエンコーダでバイナリ化してblobフォルダに保存。

### ストレージ設計

```json
// ~/.lanch-app/clipboard-history/index.json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "timestamp": "2026-03-14T15:30:00+09:00",
    "entry_type": "Text",
    "text_content": "コピーしたテキスト",
    "preview": "コピーしたテキスト",
    "size_bytes": 42
  },
  {
    "id": "...",
    "entry_type": "Image",
    "blob_file": "Image 2026-03-14 15-30-00.png",
    "preview": "Image 2026-03-14 15:30:00 (8.4 MB)",
    "size_bytes": 8847360
  }
]
```

- インデックスはメモリに保持、画像バイナリはディスクから遅延読み込み
- 1時間ごとにローテーション、7日超のエントリを自動削除

### 検索UI

📸 **【画像】Clipboard History のUI全体（Alt+Hで表示。左にリスト、右に詳細パネル）**

eguiで作ったAlfred風のポップアップ。左45%がエントリリスト、右55%が詳細パネル。

- キーワード検索: テキスト内容でフィルタ
- 日付検索: `2026-03-14` と打てばその日のエントリだけ表示
- 上下キーでナビゲーション、Enterでコピー＆自動ペースト
- 画像エントリはサムネイル表示、選択するとクリップボードに画像データとしてコピー

📸 **【画像】画像エントリを選択した状態（右パネルにサムネイルが表示される）**

---

## 第7章：翻訳機能

### 選択テキスト翻訳（Alt+Z）

テキストを選択した状態で`Alt+Z`を押すと、裏でCtrl+Cしてクリップボードからテキストを取得、言語を自動判定して翻訳、結果をWindows通知で表示する。

```
英語テキストを選択 → Alt+Z → 日本語に翻訳 → 通知で表示
日本語テキストを選択 → Alt+Z → 英語に翻訳 → 通知で表示
```

📸 **【画像】翻訳結果がWindows通知で表示される様子**

### 翻訳ポップアップ（Ctrl+Shift+T）

日本語を入力して英語にしたいときは、ポップアップを開いて入力する。

📸 **【画像】翻訳ポップアップ（テキスト入力→翻訳結果表示）**

---

## 第8章：Markdown整形

選択テキストを`Ctrl+Shift+F`で整形する。バックエンドはClaude。

### ハイブリッド方式

| 方式 | 速度 | コスト |
|------|------|--------|
| **ANTHROPIC_API_KEY あり** | 2-3秒 | API従量課金 |
| **Claude CLI（Max Plan）** | 10-30秒 | Max Plan枠内 |

環境変数`ANTHROPIC_API_KEY`があれば高速API直接呼び出し、なければClaude CLIにフォールバック。Max Planユーザーなら追加コストゼロで使える。

### 処理中スピナー

整形は時間がかかるので、カーソル付近に36x36の小さなスピナーを最前面表示する。完了したら自動で消える。

📸 **【画像】整形中にカーソル付近に表示される小さなスピナー**

---

## 第9章：Claude Codeとの開発フロー

### 開発のタイムライン

| 日付 | 内容 |
|------|------|
| 3/9 | first commit。翻訳 + Markdown整形の基盤 |
| 3/10 | エラーハンドリング改善、ハイブリッド方式に変更 |
| 3/11 | Issue設計、ClipboardHistory仕様検討 |
| 3/12 | ClipboardHistory実装（監視・ストレージ・UI） |
| 3/13-14 | テスト追加、画像対応、自動ペースト、スピナー、バグ修正 |

**5日間で5,748行、110テスト**。一人では絶対にこのペースは無理。

### Claude Codeに渡した指示

CLAUDE.mdにはこれだけ書いてある:

```markdown
実装するときはSOLID, KISS, YAGNI, DRY原則を入れてほしい
```

シンプルだけど、これだけで「過度な抽象化をしない」「不要な機能を作らない」というガードレールになる。あとは会話の中で「これ作って」「テスト書いて」「バグ直して」を繰り返すだけ。

### ハマったポイント

#### egui の Negative child size パニック

画像サムネイルの表示で、eguiが「Negative child size: [-4.7 -2.5]」でパニックしてアプリが落ちる問題があった。`ui.available_size()`がレイアウト完了前に小さな値を返し、パディングを引くと負になるケースが原因。

修正は単純:

```rust
// 表示領域が小さすぎる場合はスキップ（負サイズパニック防止）
if available.x < 10.0 || available.y < 10.0 {
    ui.colored_label(hint_color, "(表示領域が小さすぎます)");
} else {
    // 通常の画像表示
    ui.image(egui::load::SizedTexture::new(
        texture.id(),
        egui::vec2(dw.max(1.0), dh.max(1.0)),
    ));
}
```

加えて、クリップボード監視スレッドのパニックがアプリ全体を道連れにしないよう`catch_unwind`で保護した。

#### ScrollAreaが高さを確保しない

Clipboard Historyのリストが3件しか表示されない問題。`ui.horizontal`内のScrollAreaが高さゼロ扱いになっていた。`allocate_ui_at_rect`で残り領域を明示的に確保して解決。

---

## 第10章：設定ファイル

```json
// ~/.lanch-app/config.json
{
  "engine": "google",
  "deepl_api_key": "",
  "source_lang": "auto",
  "target_lang_ja": "en",
  "target_lang_en": "ja",
  "claude_model": "claude-sonnet-4-20250514",
  "font_size": 16.0,
  "opacity": 0.95,
  "hotkey_popup": "ctrl+shift+t",
  "hotkey_selected": "alt+z",
  "hotkey_format": "ctrl+shift+f",
  "hotkey_clipboard_history": "alt+h"
}
```

ホットキーは全部カスタマイズ可能。他のアプリと競合したら変えればいい。

---

## まとめ──車輪の再開発は悪じゃない

> 車輪の再開発は、自分のケツにフィットする椅子を作ることに近い。

既製品で十分な人は既製品を使えばいい。でも自分にとっては:

- **PowerToysのClipboard History** → 25件まで、検索なし、再起動で消える → ダメ
- **FlowLauncherのClipboard** → 求めてたものと違う → ダメ
- **自作** → 要件通り、軽い（メモリ12MB）、壊れたら自分で直せる → **これ**

Claude Codeのおかげで「自作」のハードルが劇的に下がった。5日で5,700行。しかもRustで。これは2年前なら考えられなかった開発速度。

「車輪の再開発」と言われても、自分の車輪は自分が一番うまく回せる。

---

### 🔗 リンク

- **GitHub**: https://github.com/miyashita337/lanch-app
- **Zenn**: https://zenn.dev/harieshokunin
- **X**: https://x.com/harieshokunin
- **Alfred**: https://www.alfredapp.com/

### 📸 画像チェックリスト

記事公開前に以下の画像を用意してください:

| 箇所 | 内容 | 撮り方 |
|------|------|--------|
| 第2章 | PowerToysのAdvanced Paste画面 | Alt+H（旧）で表示してスクショ |
| 第6章-1 | Clipboard History UI全体 | Alt+Hで開いてテキストエントリが並んだ状態 |
| 第6章-2 | 画像エントリ選択時 | 画像エントリを↓キーで選択、右パネルにサムネが出た状態 |
| 第7章-1 | 翻訳通知 | 英語テキスト選択→Alt+Z→通知表示 |
| 第7章-2 | 翻訳ポップアップ | Ctrl+Shift+T→日本語入力→翻訳結果 |
| 第8章 | 整形中スピナー | テキスト選択→Ctrl+Shift+F→スピナー表示中 |
