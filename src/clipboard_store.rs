// clipboard_store.rs - クリップボード履歴ストレージ
//
// 責務: エントリの永続化（JSON index + バイナリファイル）、検索、ローテーション
//
// ストレージ構造:
//   ~/.lanch-app/clipboard-history/
//   ├── index.json          # メタデータインデックス
//   └── blobs/              # 画像等のバイナリファイル
//       ├── Image 2026-03-12 10-30-00.png
//       └── ...

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// クリップボードエントリの種類
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntryType {
    Text,
    Image,
    Json,
    // 将来拡張: File, Binary
}

impl std::fmt::Display for EntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryType::Text => write!(f, "Text"),
            EntryType::Image => write!(f, "Image"),
            EntryType::Json => write!(f, "JSON"),
        }
    }
}

/// クリップボード履歴の1エントリ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardEntry {
    /// ユニークID
    pub id: String,
    /// コピーされた日時
    pub timestamp: DateTime<Local>,
    /// エントリの種類
    pub entry_type: EntryType,
    /// テキスト内容（Text/Json の場合）
    pub text_content: Option<String>,
    /// バイナリファイル名（Image の場合、blobs/ 内の相対パス）
    pub blob_file: Option<String>,
    /// プレビュー文字列（検索・一覧表示用、最大200文字）
    pub preview: String,
    /// データサイズ（バイト）
    pub size_bytes: usize,
}

impl ClipboardEntry {
    /// テキストエントリを作成
    pub fn new_text(text: &str) -> Self {
        let now = Local::now();
        let entry_type = if looks_like_json(text) {
            EntryType::Json
        } else {
            EntryType::Text
        };
        let preview = truncate_preview(text, 200);
        let size = text.len();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now,
            entry_type,
            text_content: Some(text.to_string()),
            blob_file: None,
            preview,
            size_bytes: size,
        }
    }

    /// 画像エントリを作成（PNG バイト列を受け取る）
    pub fn new_image(png_data: &[u8], store_dir: &PathBuf) -> std::io::Result<Self> {
        let now = Local::now();
        let filename = format!("Image {}.png", now.format("%Y-%m-%d %H-%M-%S"));
        let blobs_dir = store_dir.join("blobs");
        fs::create_dir_all(&blobs_dir)?;

        let file_path = blobs_dir.join(&filename);
        fs::write(&file_path, png_data)?;

        let preview = format!(
            "Image {} ({})",
            now.format("%Y-%m-%d %H:%M:%S"),
            format_bytes(png_data.len())
        );

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now,
            entry_type: EntryType::Image,
            text_content: None,
            blob_file: Some(filename),
            preview,
            size_bytes: png_data.len(),
        })
    }

    /// 検索クエリにマッチするか判定
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        let query_lower = query.to_lowercase();

        // テキスト内容で検索
        if let Some(ref text) = self.text_content {
            if text.to_lowercase().contains(&query_lower) {
                return true;
            }
        }

        // プレビューで検索
        if self.preview.to_lowercase().contains(&query_lower) {
            return true;
        }

        // 日付検索（YYYY-MM-DD 形式）
        let date_str = self.timestamp.format("%Y-%m-%d").to_string();
        if date_str.contains(&query_lower) {
            return true;
        }

        // エントリ種別で検索
        let type_str = self.entry_type.to_string().to_lowercase();
        if type_str.contains(&query_lower) {
            return true;
        }

        false
    }
}

/// クリップボード履歴ストア
pub struct ClipboardStore {
    /// エントリ一覧（新しい順）
    entries: Vec<ClipboardEntry>,
    /// ストレージディレクトリ
    store_dir: PathBuf,
    /// 保持期間（日数）
    retention_days: i64,
}

impl ClipboardStore {
    /// 新しいストアを作成し、既存のインデックスがあれば読み込む
    pub fn new(retention_days: i64) -> Self {
        let store_dir = dirs::home_dir()
            .expect("ホームディレクトリが見つかりません")
            .join(".lanch-app")
            .join("clipboard-history");

        let mut store = Self {
            entries: Vec::new(),
            store_dir,
            retention_days,
        };
        store.load_index();
        store
    }

    /// テスト用: 任意のディレクトリでストアを作成（永続化なし）
    #[cfg(test)]
    pub fn new_with_dir(store_dir: PathBuf, retention_days: i64) -> Self {
        let mut store = Self {
            entries: Vec::new(),
            store_dir,
            retention_days,
        };
        store.load_index();
        store
    }

    /// ストレージディレクトリのパスを返す
    #[allow(dead_code)]
    pub fn store_dir(&self) -> &PathBuf {
        &self.store_dir
    }

    /// テキストエントリを追加（重複チェック付き）
    pub fn add_text(&mut self, text: &str) {
        // 空文字・空白のみは無視
        if text.trim().is_empty() {
            return;
        }

        // 直前のエントリと同一内容なら無視（連続コピー防止）
        if let Some(last) = self.entries.first() {
            if let Some(ref last_text) = last.text_content {
                if last_text == text {
                    return;
                }
            }
        }

        let entry = ClipboardEntry::new_text(text);
        self.entries.insert(0, entry);
        self.save_index();
    }

    /// 画像エントリを追加
    pub fn add_image(&mut self, png_data: &[u8]) {
        if png_data.is_empty() {
            return;
        }

        // 直前が画像で同サイズなら重複とみなす（簡易チェック）
        if let Some(last) = self.entries.first() {
            if last.entry_type == EntryType::Image && last.size_bytes == png_data.len() {
                return;
            }
        }

        match ClipboardEntry::new_image(png_data, &self.store_dir) {
            Ok(entry) => {
                self.entries.insert(0, entry);
                self.save_index();
            }
            Err(e) => {
                eprintln!("[clipboard_store] 画像の保存に失敗: {}", e);
            }
        }
    }

    /// 検索してページ単位で返す
    ///
    /// `page`: 0始まりのページ番号
    /// `per_page`: 1ページあたりの件数
    ///
    /// 戻り値: (エントリ一覧, 合計マッチ数)
    pub fn search(&self, query: &str, page: usize, per_page: usize) -> (Vec<ClipboardEntry>, usize) {
        let matched: Vec<&ClipboardEntry> = self
            .entries
            .iter()
            .filter(|e| e.matches(query))
            .collect();

        let total = matched.len();
        let start = page * per_page;

        if start >= total {
            return (Vec::new(), total);
        }

        let end = (start + per_page).min(total);
        let page_entries = matched[start..end].iter().map(|e| (*e).clone()).collect();

        (page_entries, total)
    }

    /// エントリ総数を返す
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 古いエントリを削除（ローテーション）
    pub fn rotate(&mut self) {
        let cutoff = Local::now() - chrono::Duration::days(self.retention_days);
        let before = self.entries.len();

        // 期限切れエントリの blob ファイルを削除
        let expired: Vec<ClipboardEntry> = self
            .entries
            .iter()
            .filter(|e| e.timestamp < cutoff)
            .cloned()
            .collect();

        for entry in &expired {
            if let Some(ref blob) = entry.blob_file {
                let path = self.store_dir.join("blobs").join(blob);
                let _ = fs::remove_file(path);
            }
        }

        self.entries.retain(|e| e.timestamp >= cutoff);

        let removed = before - self.entries.len();
        if removed > 0 {
            eprintln!(
                "[clipboard_store] {}件の古いエントリを削除しました（{}日以上前）",
                removed, self.retention_days
            );
            self.save_index();
        }
    }

    /// 画像の blob ファイルパスを返す
    pub fn blob_path(&self, filename: &str) -> PathBuf {
        self.store_dir.join("blobs").join(filename)
    }

    // --- 永続化 ---

    fn index_path(&self) -> PathBuf {
        self.store_dir.join("index.json")
    }

    fn load_index(&mut self) {
        let path = self.index_path();
        if !path.exists() {
            return;
        }

        match fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<Vec<ClipboardEntry>>(&json) {
                Ok(entries) => {
                    self.entries = entries;
                    eprintln!(
                        "[clipboard_store] {}件のエントリを読み込みました",
                        self.entries.len()
                    );
                }
                Err(e) => {
                    eprintln!("[clipboard_store] インデックスのパースに失敗: {}", e);
                }
            },
            Err(e) => {
                eprintln!("[clipboard_store] インデックスの読み込みに失敗: {}", e);
            }
        }
    }

    fn save_index(&self) {
        if let Err(e) = fs::create_dir_all(&self.store_dir) {
            eprintln!("[clipboard_store] ディレクトリ作成に失敗: {}", e);
            return;
        }

        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = fs::write(self.index_path(), json) {
                    eprintln!("[clipboard_store] インデックスの保存に失敗: {}", e);
                }
            }
            Err(e) => {
                eprintln!("[clipboard_store] JSONシリアライズに失敗: {}", e);
            }
        }
    }
}

// --- ヘルパー関数 ---

/// テキストが JSON っぽいかどうか簡易判定
fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

/// プレビュー用に文字列を切り詰める
fn truncate_preview(text: &str, max_chars: usize) -> String {
    let single_line = text.replace('\n', " ").replace('\r', "");
    if single_line.chars().count() > max_chars {
        let truncated: String = single_line.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        single_line
    }
}

/// バイト数を人間が読みやすい形式に変換
fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- ヘルパー関数のテスト ---

    #[test]
    fn test_looks_like_json_object() {
        assert!(looks_like_json(r#"{"key": "value"}"#));
        assert!(looks_like_json(r#"  { "a": 1 }  "#)); // 前後に空白
    }

    #[test]
    fn test_looks_like_json_array() {
        assert!(looks_like_json(r#"[1, 2, 3]"#));
        assert!(looks_like_json(r#"  [{"a":1}]  "#));
    }

    #[test]
    fn test_looks_like_json_negative() {
        assert!(!looks_like_json("hello world"));
        assert!(!looks_like_json("{ incomplete"));
        assert!(!looks_like_json("[no closing"));
        assert!(!looks_like_json(""));
    }

    #[test]
    fn test_truncate_preview_short() {
        let text = "short text";
        assert_eq!(truncate_preview(text, 200), "short text");
    }

    #[test]
    fn test_truncate_preview_long() {
        let text = "a".repeat(250);
        let preview = truncate_preview(&text, 200);
        assert!(preview.ends_with("..."));
        // 200文字 + "..." = 203文字
        assert_eq!(preview.chars().count(), 203);
    }

    #[test]
    fn test_truncate_preview_newlines() {
        let text = "line1\nline2\rline3";
        let preview = truncate_preview(text, 200);
        // \n → " ", \r → "" (削除) なので "line1 line2line3"
        assert_eq!(preview, "line1 line2line3");
        assert!(!preview.contains('\n'));
        assert!(!preview.contains('\r'));
    }

    #[test]
    fn test_format_bytes_small() {
        assert_eq!(format_bytes(100), "100 B");
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn test_format_bytes_kb() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn test_format_bytes_mb() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 5), "5.0 MB");
    }

    // --- EntryType のテスト ---

    #[test]
    fn test_entry_type_display() {
        assert_eq!(format!("{}", EntryType::Text), "Text");
        assert_eq!(format!("{}", EntryType::Json), "JSON");
        assert_eq!(format!("{}", EntryType::Image), "Image");
    }

    #[test]
    fn test_entry_type_equality() {
        assert_eq!(EntryType::Text, EntryType::Text);
        assert_ne!(EntryType::Text, EntryType::Json);
    }

    // --- ClipboardEntry のテスト ---

    #[test]
    fn test_new_text_entry() {
        let entry = ClipboardEntry::new_text("Hello, world!");
        assert_eq!(entry.entry_type, EntryType::Text);
        assert_eq!(entry.text_content, Some("Hello, world!".to_string()));
        assert_eq!(entry.preview, "Hello, world!");
        assert_eq!(entry.size_bytes, 13);
        assert!(entry.blob_file.is_none());
        assert!(!entry.id.is_empty());
    }

    #[test]
    fn test_new_text_entry_json_detection() {
        let entry = ClipboardEntry::new_text(r#"{"key": "value"}"#);
        assert_eq!(entry.entry_type, EntryType::Json);
    }

    #[test]
    fn test_new_text_entry_array_json_detection() {
        let entry = ClipboardEntry::new_text(r#"[1, 2, 3]"#);
        assert_eq!(entry.entry_type, EntryType::Json);
    }

    #[test]
    fn test_new_text_entry_not_json() {
        let entry = ClipboardEntry::new_text("just plain text");
        assert_eq!(entry.entry_type, EntryType::Text);
    }

    // --- ClipboardEntry::matches のテスト ---

    #[test]
    fn test_matches_empty_query() {
        let entry = ClipboardEntry::new_text("anything");
        assert!(entry.matches("")); // 空クエリは全マッチ
    }

    #[test]
    fn test_matches_text_content() {
        let entry = ClipboardEntry::new_text("Hello World こんにちは");
        assert!(entry.matches("hello")); // 大文字小文字無視
        assert!(entry.matches("WORLD"));
        assert!(entry.matches("こんにちは"));
        assert!(!entry.matches("goodbye"));
    }

    #[test]
    fn test_matches_by_date() {
        let entry = ClipboardEntry::new_text("test");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert!(entry.matches(&today));
    }

    #[test]
    fn test_matches_by_entry_type() {
        let text_entry = ClipboardEntry::new_text("plain text");
        assert!(text_entry.matches("text"));

        let json_entry = ClipboardEntry::new_text(r#"{"a": 1}"#);
        assert!(json_entry.matches("json"));
    }

    #[test]
    fn test_matches_by_preview() {
        let entry = ClipboardEntry::new_text("unique_keyword_xyz");
        assert!(entry.matches("unique_keyword"));
    }

    // --- ClipboardStore のテスト（一時ディレクトリ使用） ---

    fn create_temp_store() -> (ClipboardStore, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("一時ディレクトリの作成に失敗");
        let store = ClipboardStore::new_with_dir(tmp.path().to_path_buf(), 7);
        (store, tmp)
    }

    #[test]
    fn test_store_add_text() {
        let (mut store, _tmp) = create_temp_store();
        assert_eq!(store.len(), 0);

        store.add_text("first entry");
        assert_eq!(store.len(), 1);

        store.add_text("second entry");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn test_store_add_text_empty_ignored() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("");
        store.add_text("   ");
        store.add_text("\n\t ");
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_store_add_text_duplicate_ignored() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("same text");
        store.add_text("same text"); // 重複 → 無視
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_add_text_different_not_duplicate() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("text A");
        store.add_text("text B");
        store.add_text("text A"); // 直前と異なるので追加される
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn test_store_add_image_empty_ignored() {
        let (mut store, _tmp) = create_temp_store();
        store.add_image(&[]);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_store_add_image() {
        let (mut store, _tmp) = create_temp_store();
        let fake_png = vec![0x89, 0x50, 0x4E, 0x47]; // 最小限のPNGヘッダー
        store.add_image(&fake_png);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_add_image_duplicate_size_ignored() {
        let (mut store, _tmp) = create_temp_store();
        let data = vec![1, 2, 3, 4];
        store.add_image(&data);
        store.add_image(&data); // 同サイズ → 重複とみなす
        assert_eq!(store.len(), 1);
    }

    // --- 検索・ページネーションのテスト ---

    #[test]
    fn test_search_all() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("apple");
        store.add_text("banana");
        store.add_text("cherry");

        let (entries, total) = store.search("", 0, 100);
        assert_eq!(total, 3);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_search_keyword() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("apple pie");
        store.add_text("banana split");
        store.add_text("apple sauce");

        let (entries, total) = store.search("apple", 0, 100);
        assert_eq!(total, 2);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_search_case_insensitive() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("Hello World");

        let (_, total) = store.search("hello", 0, 100);
        assert_eq!(total, 1);

        let (_, total) = store.search("HELLO", 0, 100);
        assert_eq!(total, 1);
    }

    #[test]
    fn test_search_pagination() {
        let (mut store, _tmp) = create_temp_store();
        for i in 0..5 {
            store.add_text(&format!("entry {}", i));
        }

        // 1ページ2件で3ページに分かれる
        let (page0, total) = store.search("", 0, 2);
        assert_eq!(total, 5);
        assert_eq!(page0.len(), 2);

        let (page1, _) = store.search("", 1, 2);
        assert_eq!(page1.len(), 2);

        let (page2, _) = store.search("", 2, 2);
        assert_eq!(page2.len(), 1);

        // 範囲外ページ
        let (page3, _) = store.search("", 3, 2);
        assert_eq!(page3.len(), 0);
    }

    #[test]
    fn test_search_no_results() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("hello");

        let (entries, total) = store.search("nonexistent", 0, 100);
        assert_eq!(total, 0);
        assert_eq!(entries.len(), 0);
    }

    // --- 永続化のテスト ---

    #[test]
    fn test_persistence_save_and_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        // ストアに追加して保存
        {
            let mut store = ClipboardStore::new_with_dir(dir.clone(), 7);
            store.add_text("persisted entry 1");
            store.add_text("persisted entry 2");
        }

        // 新しいストアでリロード
        {
            let store = ClipboardStore::new_with_dir(dir, 7);
            assert_eq!(store.len(), 2);
            let (entries, _) = store.search("persisted", 0, 100);
            assert_eq!(entries.len(), 2);
        }
    }

    #[test]
    fn test_blob_path() {
        let (store, _tmp) = create_temp_store();
        let path = store.blob_path("test.png");
        assert!(path.ends_with("blobs/test.png") || path.ends_with("blobs\\test.png"));
    }

    // --- ローテーションのテスト ---

    #[test]
    fn test_rotate_removes_old_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();
        let mut store = ClipboardStore::new_with_dir(dir, 7);

        // 現在のエントリを追加
        store.add_text("recent");

        // 古いエントリを手動で追加（8日前）
        let old_entry = ClipboardEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Local::now() - chrono::Duration::days(8),
            entry_type: EntryType::Text,
            text_content: Some("old entry".to_string()),
            blob_file: None,
            preview: "old entry".to_string(),
            size_bytes: 9,
        };
        store.entries.push(old_entry);
        assert_eq!(store.len(), 2);

        store.rotate();
        assert_eq!(store.len(), 1); // 古いのが削除された

        let (entries, _) = store.search("recent", 0, 100);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_rotate_keeps_recent_entries() {
        let (mut store, _tmp) = create_temp_store();
        store.add_text("today's entry");

        let before = store.len();
        store.rotate();
        assert_eq!(store.len(), before); // 最近のは消えない
    }
}
