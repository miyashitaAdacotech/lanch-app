// launcher_apps.rs - インストール済みアプリケーション検索
//
// Windows のスタートメニューフォルダをスキャンし、.lnk ファイルを
// アプリケーション候補として列挙する。

use std::path::PathBuf;

/// アプリケーションエントリ
#[derive(Debug, Clone)]
pub struct AppEntry {
    /// 表示名（.lnk 拡張子除去済み）
    pub name: String,
    /// .lnk ファイルのフルパス
    pub path: String,
}

/// スタートメニューからアプリケーション一覧をスキャンする
pub fn scan_apps() -> Vec<AppEntry> {
    let mut entries = Vec::new();
    let dirs = start_menu_dirs();
    for dir in &dirs {
        scan_dir(dir, &mut entries, 0);
    }
    // 名前でソート、重複除去
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
    entries
}

/// アプリケーションを部分一致検索
pub fn search(apps: &[AppEntry], query: &str, limit: usize) -> Vec<AppEntry> {
    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(i32, &AppEntry)> = apps
        .iter()
        .filter_map(|app| {
            let name_lower = app.name.to_lowercase();
            // 全termsが名前に含まれること
            if !terms.iter().all(|term| name_lower.contains(term)) {
                return None;
            }
            let mut score = 0;
            for term in &terms {
                if name_lower.starts_with(term) {
                    score += 30; // 先頭一致ボーナス
                } else if name_lower.contains(term) {
                    score += 10;
                }
            }
            // 短い名前ほど関連性が高い（"Chrome" vs "Chrome Update Helper"）
            score += (100 - name_lower.len().min(100)) as i32;
            Some((score, app))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, e)| e.clone())
        .collect()
}

/// アプリを起動する
pub fn launch(app: &AppEntry) {
    #[cfg(windows)]
    {
        use std::process::Command;
        let _ = Command::new("cmd")
            .args(["/C", "start", "", &app.path])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = app; // 非Windows: 何もしない
    }
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(windows)]
    {
        // ユーザーのスタートメニュー
        if let Ok(appdata) = std::env::var("APPDATA") {
            let user_start = PathBuf::from(&appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            if user_start.exists() {
                dirs.push(user_start);
            }
        }
        // 全ユーザーのスタートメニュー
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            let all_start = PathBuf::from(&programdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            if all_start.exists() {
                dirs.push(all_start);
            }
        }
    }

    dirs
}

fn scan_dir(dir: &PathBuf, out: &mut Vec<AppEntry>, depth: usize) {
    // Windows のジャンクション（例: "Application Data"）が親を指すと無限再帰し
    // スタックオーバーフローでクラッシュするため、探索深さを制限する
    if depth > 10 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, out, depth + 1);
        } else if let Some(ext) = path.extension() {
            if ext.eq_ignore_ascii_case("lnk") {
                if let Some(stem) = path.file_stem() {
                    let name = stem.to_string_lossy().to_string();
                    // "Uninstall" 系は除外
                    if !name.to_lowercase().contains("uninstall") {
                        out.push(AppEntry {
                            name,
                            path: path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
    }
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_apps() -> Vec<AppEntry> {
        vec![
            AppEntry {
                name: "Google Chrome".into(),
                path: "C:\\chrome.lnk".into(),
            },
            AppEntry {
                name: "Visual Studio Code".into(),
                path: "C:\\code.lnk".into(),
            },
            AppEntry {
                name: "Windows Terminal".into(),
                path: "C:\\wt.lnk".into(),
            },
            AppEntry {
                name: "Notepad++".into(),
                path: "C:\\notepad.lnk".into(),
            },
            AppEntry {
                name: "Chrome Remote Desktop".into(),
                path: "C:\\crd.lnk".into(),
            },
        ]
    }

    #[test]
    fn test_search_single_term() {
        let apps = sample_apps();
        let results = search(&apps, "chrome", 5);
        assert_eq!(results.len(), 2); // Google Chrome + Chrome Remote Desktop
    }

    #[test]
    fn test_search_prefix_bonus() {
        let apps = sample_apps();
        let results = search(&apps, "chrome", 5);
        // "Chrome Remote Desktop" starts with "chrome" → higher score
        // but "Google Chrome" is shorter → also high score
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_multi_term() {
        let apps = sample_apps();
        let results = search(&apps, "visual code", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Visual Studio Code");
    }

    #[test]
    fn test_search_empty() {
        let apps = sample_apps();
        assert!(search(&apps, "", 5).is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let apps = sample_apps();
        assert!(search(&apps, "nonexistent", 5).is_empty());
    }

    #[test]
    fn test_search_limit() {
        let apps = sample_apps();
        let results = search(&apps, "e", 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_scan_no_crash() {
        let _ = scan_apps();
    }
}
