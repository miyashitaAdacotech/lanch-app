// launcher.rs - Alfred/Spotlight風ランチャーUI
//
// Ctrl+Space で起動し、テキスト入力に応じてリアルタイムで候補を表示する。
// 別プロセスとして起動される（eframe再利用問題回避のため）。
//
// 候補の優先順位:
//   1. 履歴サジェスト（頻度順）
//   2. ショートカット完全一致
//   3. 電卓
//   4. URL検出
//   5. アプリケーション検索
//   6. ブックマーク部分一致
//   7. ショートカット部分一致
//   8. Web検索フォールバック

use eframe::egui;

use crate::config::Config;
use crate::launcher_apps::{self, AppEntry};
use crate::launcher_bookmarks::{self, BookmarkEntry};
use crate::launcher_calc;
use crate::launcher_history::LauncherHistory;

/// 候補アイテム
#[derive(Clone)]
struct Candidate {
    icon: &'static str,
    label: String,
    sublabel: String,
    action: CandidateAction,
}

#[derive(Clone)]
enum CandidateAction {
    /// ショートカット: コマンド実行
    RunCommand { command: String, args: Vec<String> },
    /// 電卓: 結果をクリップボードにコピー
    CopyText(String),
    /// URL: ブラウザで開く
    OpenUrl(String),
    /// ブックマーク: ブラウザで開く
    OpenBookmark(String),
    /// アプリ: 起動
    LaunchApp(AppEntry),
    /// Web検索
    WebSearch(String),
}

struct LauncherApp {
    config: Config,
    query: String,
    candidates: Vec<Candidate>,
    selected_index: i32,
    first_frame: bool,
    had_focus: bool,
    created_at: std::time::Instant,
    // プリロード済みデータ
    bookmarks: Vec<BookmarkEntry>,
    apps: Vec<AppEntry>,
    history: LauncherHistory,
}

impl LauncherApp {
    fn new(config: Config) -> Self {
        let bookmarks = launcher_bookmarks::load_bookmarks();
        let apps = launcher_apps::scan_apps();
        let history = LauncherHistory::load();

        let mut app = Self {
            config,
            query: String::new(),
            candidates: Vec::new(),
            selected_index: 0,
            first_frame: true,
            had_focus: false,
            created_at: std::time::Instant::now(),
            bookmarks,
            apps,
            history,
        };
        app.update_candidates();
        app
    }

    fn update_candidates(&mut self) {
        self.candidates.clear();
        let q = self.query.trim();

        // 1. 履歴サジェスト
        let history_suggestions = self.history.suggest(q, 3);
        for entry in &history_suggestions {
            let icon = match entry.category.as_str() {
                "shortcut" => "\u{26A1}", // ⚡
                "bookmark" => "\u{2B50}", // ⭐
                "app" => "\u{1F4BB}",     // 💻
                "search" => "\u{1F50D}",  // 🔍
                "calc" => "\u{1F522}",    // 🔢
                "url" => "\u{1F310}",     // 🌐
                _ => "\u{23F3}",          // ⏳
            };
            self.candidates.push(Candidate {
                icon,
                label: entry.label.clone(),
                sublabel: format!("({}回使用)", entry.count),
                action: match entry.category.as_str() {
                    "app" => CandidateAction::LaunchApp(AppEntry {
                        name: entry.label.clone(),
                        path: entry.action.clone(),
                    }),
                    "bookmark" | "url" => CandidateAction::OpenBookmark(entry.action.clone()),
                    "search" => {
                        let search_term = entry
                            .action
                            .strip_prefix("search:")
                            .unwrap_or(&entry.action);
                        CandidateAction::WebSearch(search_term.to_string())
                    }
                    "calc" => CandidateAction::CopyText(entry.action.clone()),
                    _ => CandidateAction::RunCommand {
                        command: entry.action.clone(),
                        args: Vec::new(),
                    },
                },
            });
        }

        if q.is_empty() {
            // 空クエリ → 履歴のみ表示
            self.clamp_selection();
            return;
        }

        // 2. ショートカット完全一致
        let shortcuts = &self.config.launcher_shortcuts;
        for sc in shortcuts {
            if sc.key.eq_ignore_ascii_case(q) {
                self.candidates.push(Candidate {
                    icon: "\u{26A1}", // ⚡
                    label: sc.name.clone(),
                    sublabel: format!("[{}]", sc.key),
                    action: CandidateAction::RunCommand {
                        command: sc.command.clone(),
                        args: sc.args.clone(),
                    },
                });
            }
        }

        // 3. 電卓
        if let Some(result) = launcher_calc::try_evaluate(q) {
            self.candidates.push(Candidate {
                icon: "\u{1F522}", // 🔢
                label: format!("{} = {}", q, result),
                sublabel: "Enterでコピー".into(),
                action: CandidateAction::CopyText(result),
            });
        }

        // 4. URL検出
        if looks_like_url(q) {
            let url = if q.starts_with("http://") || q.starts_with("https://") {
                q.to_string()
            } else {
                format!("https://{}", q)
            };
            self.candidates.push(Candidate {
                icon: "\u{1F310}", // 🌐
                label: q.to_string(),
                sublabel: "ブラウザで開く".into(),
                action: CandidateAction::OpenUrl(url),
            });
        }

        // 5. アプリケーション検索
        let app_results = launcher_apps::search(&self.apps, q, 5);
        for app in &app_results {
            // 履歴と重複チェック
            if self.candidates.iter().any(|c| {
                if let CandidateAction::LaunchApp(ref a) = c.action {
                    a.path == app.path
                } else {
                    false
                }
            }) {
                continue;
            }
            self.candidates.push(Candidate {
                icon: "\u{1F4BB}", // 💻
                label: app.name.clone(),
                sublabel: "アプリを起動".into(),
                action: CandidateAction::LaunchApp(app.clone()),
            });
        }

        // 6. ブックマーク検索
        let bm_results = launcher_bookmarks::search(&self.bookmarks, q, 5);
        for bm in &bm_results {
            // 履歴と重複チェック
            if self.candidates.iter().any(|c| {
                if let CandidateAction::OpenBookmark(ref u) = c.action {
                    *u == bm.url
                } else {
                    false
                }
            }) {
                continue;
            }
            self.candidates.push(Candidate {
                icon: "\u{2B50}", // ⭐
                label: bm.name.clone(),
                sublabel: truncate_url(&bm.url, 50),
                action: CandidateAction::OpenBookmark(bm.url.clone()),
            });
        }

        // 7. ショートカット部分一致（完全一致で既に追加されていないもの）
        for sc in shortcuts {
            if !sc.key.eq_ignore_ascii_case(q)
                && (sc.key.to_lowercase().contains(&q.to_lowercase())
                    || sc.name.to_lowercase().contains(&q.to_lowercase()))
            {
                self.candidates.push(Candidate {
                    icon: "\u{26A1}", // ⚡
                    label: sc.name.clone(),
                    sublabel: format!("[{}]", sc.key),
                    action: CandidateAction::RunCommand {
                        command: sc.command.clone(),
                        args: sc.args.clone(),
                    },
                });
            }
        }

        // 8. Web検索フォールバック
        self.candidates.push(Candidate {
            icon: "\u{1F50D}", // 🔍
            label: format!("Google検索: {}", q),
            sublabel: String::new(),
            action: CandidateAction::WebSearch(q.to_string()),
        });

        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let max = self.candidates.len() as i32 - 1;
        if self.selected_index > max {
            self.selected_index = max.max(0);
        }
    }

    fn execute_selected(&mut self) -> bool {
        if self.candidates.is_empty() {
            return false;
        }
        let idx = self.selected_index.max(0) as usize;
        if idx >= self.candidates.len() {
            return false;
        }

        let candidate = self.candidates[idx].clone();

        // 履歴に記録
        let (action_str, category) = match &candidate.action {
            CandidateAction::RunCommand { command, args } => {
                let full = if args.is_empty() {
                    command.clone()
                } else {
                    format!("{} {}", command, args.join(" "))
                };
                (full, "shortcut")
            }
            CandidateAction::CopyText(text) => (text.clone(), "calc"),
            CandidateAction::OpenUrl(url) => (url.clone(), "url"),
            CandidateAction::OpenBookmark(url) => (url.clone(), "bookmark"),
            CandidateAction::LaunchApp(app) => (app.path.clone(), "app"),
            CandidateAction::WebSearch(q) => (format!("search:{}", q), "search"),
        };
        self.history.record(&action_str, &candidate.label, category);

        // アクション実行
        match &candidate.action {
            CandidateAction::RunCommand { command, args } => {
                let _ = std::process::Command::new(command).args(args).spawn();
            }
            CandidateAction::CopyText(text) => {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(text);
                }
            }
            CandidateAction::OpenUrl(url) | CandidateAction::OpenBookmark(url) => {
                open_url(url);
            }
            CandidateAction::LaunchApp(app) => {
                launcher_apps::launch(app);
            }
            CandidateAction::WebSearch(query) => {
                let url = format!("https://www.google.com/search?q={}", urlencoded(query));
                open_url(&url);
            }
        }

        true
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // フォーカス喪失で閉じる（初期フレーム後）
        let focused = ctx.input(|i| i.focused);
        if self.created_at.elapsed().as_millis() > 500 {
            if self.had_focus && !focused {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if focused {
            self.had_focus = true;
        }

        // Escで閉じる
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // 上下キー
        let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
        if up {
            self.selected_index = (self.selected_index - 1).max(0);
        }
        if down {
            self.selected_index = (self.selected_index + 1)
                .min(self.candidates.len() as i32 - 1)
                .max(0);
        }

        // Enter で実行
        let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let mut clicked_index: Option<i32> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(30, 30, 46)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);

                // ヘッダー (ドラッグ移動)
                let header_resp = ui
                    .horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.colored_label(egui::Color32::from_rgb(180, 180, 200), "Lanch");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("\u{2715}").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            ui.add_space(4.0);
                        });
                    })
                    .response
                    .interact(egui::Sense::drag());
                if header_resp.dragged() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                ui.separator();

                // 検索バー
                let search_resp = ui
                    .horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("\u{1F50E}").size(20.0)); // 🔎
                        let resp = ui.add_sized(
                            egui::vec2(ui.available_width() - 16.0, 32.0),
                            egui::TextEdit::singleline(&mut self.query)
                                .font(egui::TextStyle::Heading)
                                .hint_text("検索...")
                                .frame(false)
                                .text_color(egui::Color32::WHITE),
                        );
                        resp
                    })
                    .inner;

                if self.first_frame {
                    search_resp.request_focus();
                    self.first_frame = false;
                }

                // クエリ変更時に候補更新
                if search_resp.changed() {
                    self.selected_index = 0;
                    self.update_candidates();
                }

                ui.separator();

                // 候補リスト
                let available_height = ui.available_height();
                egui::ScrollArea::vertical()
                    .max_height(available_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        if self.candidates.is_empty() {
                            ui.add_space(16.0);
                            ui.centered_and_justified(|ui| {
                                ui.colored_label(
                                    egui::Color32::from_rgb(100, 100, 120),
                                    "入力して検索...",
                                );
                            });
                        } else {
                            for (i, candidate) in self.candidates.iter().enumerate() {
                                let is_selected = i as i32 == self.selected_index;
                                let bg = if is_selected {
                                    egui::Color32::from_rgb(60, 60, 80)
                                } else {
                                    egui::Color32::TRANSPARENT
                                };

                                let frame = egui::Frame::NONE
                                    .fill(bg)
                                    .inner_margin(egui::Margin::symmetric(12, 6))
                                    .corner_radius(4.0);

                                let resp = frame
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(candidate.icon).size(18.0),
                                            );
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    egui::RichText::new(&candidate.label)
                                                        .size(15.0)
                                                        .color(if is_selected {
                                                            egui::Color32::WHITE
                                                        } else {
                                                            egui::Color32::from_rgb(220, 220, 230)
                                                        }),
                                                );
                                                if !candidate.sublabel.is_empty() {
                                                    ui.label(
                                                        egui::RichText::new(&candidate.sublabel)
                                                            .size(11.0)
                                                            .color(egui::Color32::from_rgb(
                                                                120, 120, 140,
                                                            )),
                                                    );
                                                }
                                            });
                                        });
                                    })
                                    .response
                                    .interact(egui::Sense::click());

                                if resp.clicked() {
                                    clicked_index = Some(i as i32);
                                }

                                // 選択中アイテムをスクロールに追従
                                if is_selected {
                                    resp.scroll_to_me(Some(egui::Align::Center));
                                }
                            }
                        }
                    });
            });

        // クリック実行（借用を分離するためループ外で処理）
        if let Some(idx) = clicked_index {
            self.selected_index = idx;
            if self.execute_selected() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Enter実行（UIの外で処理）
        if enter && self.execute_selected() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        ctx.request_repaint();
    }
}

/// ランチャーウィンドウを表示する
pub fn show_launcher(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        "Lanch Launcher",
        options,
        Box::new(move |cc| {
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(LauncherApp::new(config)))
        }),
    )
    .map_err(|e| format!("ランチャーウィンドウの起動に失敗: {}", e))?;

    Ok(())
}

// === ヘルパー ===

fn looks_like_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        return true;
    }
    // 小数（例: 123.456）を URL と誤判定しないよう数値パース不可を条件に加える
    s.contains('.') && !s.contains(' ') && s.len() > 4 && s.parse::<f64>().is_err()
}

fn open_url(url: &str) {
    // cmd.exe 経由（cmd /C start）だと URL 中の `&` 等がコマンド区切りとして再解釈され、
    // クエリ付き URL の破損やコマンド注入につながる。ShellExecute 相当を安全に扱う
    // `open` crate に委ねる（Windows/macOS/Linux を横断で正しく処理）。
    if let Err(e) = open::that(url) {
        eprintln!("[launcher] URL を開けませんでした ({}): {}", url, e);
    }
}

fn urlencoded(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

fn truncate_url(url: &str, max_len: usize) -> String {
    // バイト単位スライスは UTF-8 のマルチバイト境界で panic するため、char 単位で切り詰める
    if url.chars().count() <= max_len {
        url.to_string()
    } else {
        let truncated: String = url.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

/// 日本語フォントをセットアップする
fn setup_japanese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Windows: Yu Gothic UI / Meiryo をロード
    let font_paths = [
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
    ];

    for path in &font_paths {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "japanese".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(data)),
            );
            // Proportional と Monospace の両方に追加
            if let Some(families) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                families.push("japanese".to_owned());
            }
            if let Some(families) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                families.push("japanese".to_owned());
            }
            break;
        }
    }

    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_url_scheme() {
        assert!(looks_like_url("http://example.com"));
        assert!(looks_like_url("https://example.com"));
    }

    #[test]
    fn test_looks_like_url_domain() {
        assert!(looks_like_url("github.com"));
        assert!(looks_like_url("example.co.jp"));
    }

    #[test]
    fn test_looks_like_url_rejects_decimal() {
        // 小数は電卓入力なので URL 扱いしない（gemini review 指摘の回帰防止）
        assert!(!looks_like_url("123.456"));
        assert!(!looks_like_url("0.12345"));
    }

    #[test]
    fn test_looks_like_url_rejects_plain_text() {
        assert!(!looks_like_url("hello world"));
        assert!(!looks_like_url("abc"));
    }
}
