// launcher_bookmarks.rs - Chromeブックマーク検索
//
// Chrome の Bookmarks JSON を読み取り、部分一致で検索する。
// Chrome 未インストールなら空リスト（エラーなし）。

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

/// ブックマークエントリ
#[derive(Debug, Clone)]
pub struct BookmarkEntry {
    pub name: String,
    pub url: String,
}

/// Chrome Bookmarks JSON のルート
#[derive(Deserialize)]
struct BookmarksFile {
    roots: std::collections::HashMap<String, BookmarkNode>,
}

/// ブックマークノード（フォルダ or URL）
#[derive(Deserialize)]
struct BookmarkNode {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    children: Option<Vec<BookmarkNode>>,
}

/// Chrome ブックマークを読み込む
pub fn load_bookmarks() -> Vec<BookmarkEntry> {
    let mut entries = Vec::new();
    for path in bookmark_paths() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(file) = serde_json::from_str::<BookmarksFile>(&content) {
                for node in file.roots.values() {
                    flatten_bookmarks(node, &mut entries);
                }
            }
        }
    }
    // 重複URL除去
    entries.dedup_by(|a, b| a.url == b.url);
    entries
}

/// ブックマークを部分一致検索（AND検索、名前優先スコアリング）
pub fn search(bookmarks: &[BookmarkEntry], query: &str, limit: usize) -> Vec<BookmarkEntry> {
    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(i32, &BookmarkEntry)> = bookmarks
        .iter()
        .filter_map(|entry| {
            let name_lower = entry.name.to_lowercase();
            let url_lower = entry.url.to_lowercase();
            // 全termsがname or urlのいずれかに含まれること
            let all_match = terms.iter().all(|term| {
                name_lower.contains(term) || url_lower.contains(term)
            });
            if !all_match {
                return None;
            }
            // スコア: 名前一致は高得点
            let mut score = 0;
            for term in &terms {
                if name_lower.contains(term) {
                    score += 10;
                }
                if url_lower.contains(term) {
                    score += 1;
                }
                // 先頭一致ボーナス
                if name_lower.starts_with(term) {
                    score += 20;
                }
            }
            Some((score, entry))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, e)| e.clone()).collect()
}

fn flatten_bookmarks(node: &BookmarkNode, out: &mut Vec<BookmarkEntry>) {
    if let Some(url) = &node.url {
        if !url.is_empty() {
            out.push(BookmarkEntry {
                name: node.name.clone(),
                url: url.clone(),
            });
        }
    }
    if let Some(children) = &node.children {
        for child in children {
            flatten_bookmarks(child, out);
        }
    }
}

fn bookmark_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(&local_app_data).join("Google").join("Chrome").join("User Data");
            // Default profile
            let default = base.join("Default").join("Bookmarks");
            if default.exists() {
                paths.push(default);
            }
            // Numbered profiles
            for i in 1..=5 {
                let profile = base.join(format!("Profile {}", i)).join("Bookmarks");
                if profile.exists() {
                    paths.push(profile);
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            let default = home
                .join(".config/google-chrome/Default/Bookmarks");
            if default.exists() {
                paths.push(default);
            }
        }
    }

    paths
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bookmarks() -> Vec<BookmarkEntry> {
        vec![
            BookmarkEntry { name: "Rust Programming".into(), url: "https://www.rust-lang.org".into() },
            BookmarkEntry { name: "GitHub".into(), url: "https://github.com".into() },
            BookmarkEntry { name: "Rust by Example".into(), url: "https://doc.rust-lang.org/rust-by-example".into() },
            BookmarkEntry { name: "Google".into(), url: "https://www.google.com".into() },
            BookmarkEntry { name: "Zenn".into(), url: "https://zenn.dev".into() },
        ]
    }

    #[test]
    fn test_search_single_term() {
        let bm = sample_bookmarks();
        let results = search(&bm, "rust", 5);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.name.to_lowercase().contains("rust") || r.url.to_lowercase().contains("rust")));
    }

    #[test]
    fn test_search_multi_term() {
        let bm = sample_bookmarks();
        let results = search(&bm, "rust example", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Rust by Example");
    }

    #[test]
    fn test_search_url_match() {
        let bm = sample_bookmarks();
        let results = search(&bm, "github", 5);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_empty_query() {
        let bm = sample_bookmarks();
        let results = search(&bm, "", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let bm = sample_bookmarks();
        let results = search(&bm, "nonexistent", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_limit() {
        let bm = sample_bookmarks();
        let results = search(&bm, "o", 2); // matches Google, rust-lang.org, etc.
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_load_bookmarks_no_crash() {
        // Chrome未インストール環境でもパニックしない
        let _ = load_bookmarks();
    }
}
