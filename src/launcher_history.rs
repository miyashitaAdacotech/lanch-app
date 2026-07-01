// launcher_history.rs - ランチャー入力履歴
//
// ユーザーがランチャーで選択したアクションの履歴を保持し、
// 頻度の高いものをサジェストする。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 履歴エントリ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// 実行されたアクション（URLやコマンド等）
    pub action: String,
    /// 表示ラベル
    pub label: String,
    /// カテゴリ（"shortcut", "bookmark", "app", "search", "calc", "url"）
    pub category: String,
    /// 使用回数
    pub count: u32,
}

/// ランチャー履歴管理
pub struct LauncherHistory {
    entries: Vec<HistoryEntry>,
}

impl LauncherHistory {
    /// 履歴をファイルから読み込む。ファイルがなければ空で開始。
    pub fn load() -> Self {
        let path = history_path();
        let entries = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Self { entries }
    }

    /// アクションを記録する（既存なら count++、新規なら追加）
    pub fn record(&mut self, action: &str, label: &str, category: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.action == action) {
            entry.count += 1;
            entry.label = label.to_string(); // ラベル更新
        } else {
            self.entries.push(HistoryEntry {
                action: action.to_string(),
                label: label.to_string(),
                category: category.to_string(),
                count: 1,
            });
        }
        self.save();
    }

    /// 履歴をファイルに保存する
    pub fn save(&self) {
        let path = history_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            let _ = fs::write(&path, json);
        }
    }

    /// クエリに基づくサジェスト（頻度順）
    /// 空クエリなら全履歴を頻度順で返す（トップN）
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<HistoryEntry> {
        let query_lower = query.to_lowercase();

        let mut matches: Vec<&HistoryEntry> = if query_lower.is_empty() {
            self.entries.iter().collect()
        } else {
            self.entries.iter().filter(|e| {
                e.label.to_lowercase().contains(&query_lower)
                    || e.action.to_lowercase().contains(&query_lower)
            }).collect()
        };

        matches.sort_by(|a, b| b.count.cmp(&a.count));
        matches.into_iter().take(limit).cloned().collect()
    }
}

fn history_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".lanch-app")
        .join("launcher_history.json")
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_history() -> LauncherHistory {
        LauncherHistory {
            entries: vec![
                HistoryEntry { action: "cc".into(), label: "Claude Code".into(), category: "shortcut".into(), count: 10 },
                HistoryEntry { action: "https://github.com".into(), label: "GitHub".into(), category: "bookmark".into(), count: 5 },
                HistoryEntry { action: "notepad".into(), label: "Notepad++".into(), category: "app".into(), count: 3 },
                HistoryEntry { action: "search:rust".into(), label: "Google検索: rust".into(), category: "search".into(), count: 1 },
            ],
        }
    }

    #[test]
    fn test_suggest_empty_query() {
        let h = make_history();
        let results = h.suggest("", 10);
        assert_eq!(results.len(), 4);
        // 頻度順
        assert_eq!(results[0].action, "cc");
        assert_eq!(results[1].action, "https://github.com");
    }

    #[test]
    fn test_suggest_with_query() {
        let h = make_history();
        let results = h.suggest("git", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].label, "GitHub");
    }

    #[test]
    fn test_suggest_limit() {
        let h = make_history();
        let results = h.suggest("", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_record_new() {
        let mut h = LauncherHistory { entries: Vec::new() };
        h.entries.push(HistoryEntry {
            action: "test".into(),
            label: "Test".into(),
            category: "shortcut".into(),
            count: 1,
        });
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0].count, 1);
    }

    #[test]
    fn test_record_existing() {
        let mut h = make_history();
        h.record("cc", "Claude Code", "shortcut");
        let cc = h.entries.iter().find(|e| e.action == "cc").unwrap();
        assert_eq!(cc.count, 11);
    }

    #[test]
    fn test_load_no_crash() {
        let _ = LauncherHistory::load();
    }
}
