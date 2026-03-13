# lanch-app 引き継ぎ資料 (2026-03-13)

## 現状サマリ

lanch-app は Rust 製 Windows デスクトップアプリ（Alfred/Raycast 風ランチャー）。
翻訳・Markdown整形・クリップボード履歴の3機能を持つ。

クリップボード履歴UIの改修中に2つのバグが残っている。

---

## 未解決バグ（優先度順）

### BUG-1: 画像エントリ選択時にクラッシュ

**症状**: Ctrl+Shift+V でクリップボード履歴ポップアップを開き、↑↓キーで Image エントリを選択すると、アプリがクラッシュ（ウィンドウ消失）する。テキストエントリの選択は正常動作。

**原因の推測**: `src/clipboard_ui.rs` の `load_image_texture()` (L144-178) で `image` クレートの PNG デコード → egui テクスチャ変換時にパニックしている可能性が高い。

- 11.2MB の大きな PNG をデコードしてメモリ上に RGBA 展開するとき、メモリ不足 or unwrap パニック
- `rgba.dimensions()` が `image` クレートの `GenericImageView` トレイトを要求するが、import が削除済み（warning修正時に消した）。ただしコンパイルは通っているので `to_rgba8()` の戻り値の固有メソッドで解決しているかもしれない
- `egui::ColorImage::from_rgba_unmultiplied` に渡すバッファサイズが不一致の可能性

**修正方針**:
1. `load_image_texture()` 内の処理を全て `catch_unwind` で包むか、各ステップを個別に `eprintln!` でログ出力して原因特定
2. 大きな画像はデコード前にリサイズする（image クレートの `resize` を使う）
3. 最低限、画像デコード失敗時にクラッシュせず「読み込み失敗」メッセージを表示するようにする

**関連ファイル**: `src/clipboard_ui.rs` L144-178, L270-300

### BUG-2: ~~EventLoop 再作成エラー~~ (修正済み・未検証)

**症状**: Ctrl+Shift+V を2回目以降押すと `winit EventLoopError: EventLoop can't be recreated` エラー。

**対応済み内容**: `tray.rs` で `thread::spawn` + `eframe::run_native` の代わりに `spawn_self(&["--clipboard-history"])` で別プロセス起動するように変更済み。`main.rs` に `--clipboard-history` モード追加済み。

**検証状況**: BUG-1 のクラッシュで2回目起動を十分テストできていない。テキストのみの履歴で2回目起動が動くか確認すること。

---

## 直近で完了した変更（未コミット）

### 1. 選択ハイライトを黄色に変更 ✅
- `src/clipboard_ui.rs` L367-368
- 背景: `Color32::from_rgba_premultiplied(250, 227, 80, 40)` (半透明黄色)
- ボーダー/テキスト: `Color32::from_rgb(250, 227, 80)`

### 2. 水平分割レイアウト（リスト | 詳細パネル）✅
- `src/clipboard_ui.rs` 全体
- 左45%: エントリリスト、右55%: 詳細パネル
- ウィンドウサイズ: 600x500 → 1100x600
- シングルクリック → 選択のみ（詳細表示）、ダブルクリック → コピー＆閉じる

### 3. テキスト詳細表示 ✅
- 1000文字以上は `...` で切り詰め
- JSON はモノスペースフォントで表示
- 定数 `DETAIL_TEXT_MAX_CHARS = 1000`

### 4. 画像サムネイル表示 ❌ (BUG-1)
- `image` クレート追加済み (`Cargo.toml`: `image = { version = "0.25", features = ["png"] }`)
- `load_image_texture()` 実装済みだがクラッシュする
- 定数 `DETAIL_IMAGE_MIN_SIZE = 500.0`

### 5. 別プロセス起動 ✅ (BUG-2対策)
- `main.rs`: `--clipboard-history` CLI引数追加
- `tray.rs`: `spawn_self(&["--clipboard-history"])` に変更
- `clipboard_store::ClipboardStore::new(7)` でファイルから読み直す

### 6. ファイルログ出力 ✅
- `main.rs` の `init_logging()`: `~/.lanch-app/lanch-app.log` に stderr リダイレクト
- Win32 `SetStdHandle` で stderr をファイルハンドルに差し替え
- ローテーション: 2日超 or 1MB超で `.log.old` に、2日超の `.log.old` は削除
- `Cargo.toml`: `Win32_System_Console` feature 追加済み

---

## ファイル構成と責務

```
src/
├── main.rs              # エントリポイント、CLI引数解析、ログ初期化
├── tray.rs              # システムトレイ、ホットキー登録、メニュー
├── clipboard_ui.rs      # クリップボード履歴 egui ポップアップ ★修正対象
├── clipboard_store.rs   # 履歴ストレージ（JSON index + blobs/）
├── clipboard_history.rs # Win32 クリップボード監視 + PNG エンコーダー
├── clipboard.rs         # クリップボード操作ヘルパー
├── config.rs            # 設定ファイル管理
├── formatter.rs         # Markdown整形（API/CLI ハイブリッド）
├── translator.rs        # 翻訳機能
├── popup.rs             # 翻訳ポップアップUI
├── notification.rs      # 通知ヘルパー
└── lang.rs              # 言語検出
```

## ストレージ構造

```
~/.lanch-app/
├── config.json
├── lanch-app.log           # ← 新規追加
├── lanch-app.log.old       # ← 新規追加
└── clipboard-history/
    ├── index.json          # エントリメタデータ
    └── blobs/              # 画像PNG
        ├── Image 2026-03-13 17-56-42.png
        └── ...
```

## 主要な型

- `ClipboardEntry`: id, timestamp, entry_type(Text/Image/Json), text_content, blob_file, preview, size_bytes
- `ClipboardStore`: entries Vec, store_dir, search(), rotate(), blob_path()
- `SharedStore` = `Arc<Mutex<ClipboardStore>>`
- `ClipboardHistoryPopup`: eframe::App 実装、image_cache: HashMap<String, TextureHandle>

## テスト

84テスト全パス（`cargo test`）。clipboard_store(30+), clipboard_history(15), config(12), formatter(14), lang(6) のユニットテスト。

## 設計原則（CLAUDE.md）

SOLID, KISS, YAGNI, DRY

## ビルド・実行

```powershell
# ビルド＆起動（PowerShell）
cargo build; if ($?) { Start-Process .\target\debug\lanch-app.exe }

# テスト
cargo test

# ログ確認
Get-Content ~\.lanch-app\lanch-app.log -Tail 50
```

## GitHub Issues（未クローズ）

- #1 テスト
- #2 自動起動
- #3 ホットキー競合
- #4 WinRT通知
- #5 CLIモード検証
- #6 設定クリーンアップ
- #7 ClipboardHistory（未作成）

---

## Claude Code への依頼事項

1. **BUG-1 修正**: 画像選択時のクラッシュを直す。`load_image_texture()` でパニックしないようにし、大画像はリサイズしてからテクスチャ化する。ログ (`~/.lanch-app/lanch-app.log`) を確認してクラッシュ原因を特定すること。
2. **BUG-2 検証**: テキストのみでの2回目起動を確認。
3. **warning 0 を維持**: `cargo build` で warning が出ないようにする。
4. **テスト維持**: `cargo test` で 84 テスト全パスを維持。
5. **コミット**: 全修正完了後に git commit & push。
