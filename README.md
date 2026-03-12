# Lanch App - 統一ランチャー

Alfred / Raycast / PowerToys にインスパイアされた Windows 用ランチャーツール。
quick-translate をベースに、Claude によるMarkdown整形機能を統合。

## 機能

| ショートカット | 機能 | 説明 |
|---|---|---|
| `Ctrl+Shift+T` | 翻訳ポップアップ | テキスト入力 → リアルタイム翻訳 |
| `Ctrl+Shift+Y` | 選択テキスト翻訳 | 選択中のテキストを自動翻訳 → ポップアップ表示 |
| `Ctrl+Shift+F` | Markdown整形 | 選択中のテキストを Claude で整形 → クリップボードにコピー（サイレント） |

## セットアップ

### 1. Rust のインストール

```powershell
winget install Rustlang.Rustup
```

### 2. Markdown整形の準備（ハイブリッド方式）

Markdown整形は2つのバックエンドに対応しています。どちらか一方があれば動作します。

**方式A: Anthropic API 直接（高速: 2-3秒）**

```powershell
# ユーザー環境変数に設定（再起動不要・永続化される）
[System.Environment]::SetEnvironmentVariable('ANTHROPIC_API_KEY', 'sk-ant-api03-xxxxx', 'User')

# 現在のセッションにも即時反映
$env:ANTHROPIC_API_KEY = 'sk-ant-api03-xxxxx'
```

> API キーの取得: [Anthropic Console](https://console.anthropic.com/) → API Keys → Create Key
> ※ API 利用には別途クレジット購入が必要です（Claude.ai サブスクリプションとは別課金）

**方式B: Claude Code CLI 経由（低速: 20-30秒、Max Plan サブスクリプション枠）**

```powershell
npm install -g @anthropic-ai/claude-code
claude login
```

> Claude.ai Max Plan ($100/200) のサブスクリプション枠を使用するため追加費用なし。
> ただし Node.js 起動のオーバーヘッドがあるため API 直接より遅くなります。

**優先順位**: ANTHROPIC_API_KEY が設定されていれば API 直接（高速）を使用し、
未設定の場合は Claude CLI にフォールバックします。

### 3. ビルド

```powershell
cd lanch-app
cargo build --release
```

ビルド成功すると `target/release/lanch-app.exe` が生成されます。

### 4. 実行

```powershell
# トレイ常駐モード（デフォルト）
cargo run --release

# CLI翻訳
cargo run --release -- --translate "Hello World"

# CLI Markdown整形
cargo run --release -- --format "messy text here..."

# ヘルプ
cargo run --release -- --help
```

## 設定ファイル

初回起動時に `~/.lanch-app/config.json` が自動生成されます。

```json
{
  "engine": "google",
  "deepl_api_key": "",
  "source_lang": "auto",
  "target_lang_ja": "en",
  "target_lang_en": "ja",
  "claude_model": "claude-haiku-4-5-20251001",
  "font_size": 16.0,
  "opacity": 0.95,
  "log_enabled": true,
  "hotkey_popup": "ctrl+shift+t",
  "hotkey_selected": "ctrl+shift+y",
  "hotkey_format": "ctrl+shift+f"
}
```

| フィールド | 説明 |
|---|---|
| `claude_model` | 整形に使用するモデル（デフォルト: `claude-haiku-4-5-20251001`、高速＆安価） |
| `claude_api_key` | **非推奨**: 環境変数 `ANTHROPIC_API_KEY` を使用してください |

> `~/.quick-translate/config.json` が存在する場合、初回起動時に自動マイグレーションされます。

## Markdown整形の動作フロー

1. テキストを選択
2. `Ctrl+Shift+F` を押す
3. 自動で Ctrl+C → クリップボードにコピー
4. バックエンド自動選択（API直接 or Claude CLI）
5. Claude に送信 → Markdown形式に整形
6. 整形結果がクリップボードにコピーされる
7. トースト通知で完了を知らせる（サイレントモード）

### 整形できるもの

- 崩れたテーブル → Markdownテーブルに復元
- コードスニペット → 言語タグ付きコードブロック
- JSON / YAML / XML → 構造化データとして整形
- ログ出力 / スタックトレース → コードブロック
- 箇条書き / 番号リスト → 適切なリスト形式
- URL → Markdownリンク

## プロジェクト構成

```
src/
├── main.rs          # エントリポイント、CLI引数パース
├── config.rs        # 設定ファイル読み書き (serde)
├── lang.rs          # 言語自動判定 (Unicode範囲チェック)
├── translator.rs    # Google翻訳 / DeepL エンジン (reqwest)
├── formatter.rs     # Markdown整形 ハイブリッド (API直接 / Claude CLI)
├── notification.rs  # Windows トースト通知 (PowerShell)
├── popup.rs         # egui ポップアップUI (翻訳 / 整形結果)
├── clipboard.rs     # クリップボード操作 & キー入力シミュレーション
└── tray.rs          # システムトレイ常駐 & グローバルホットキー
```

## テスト

```powershell
cargo test
```
