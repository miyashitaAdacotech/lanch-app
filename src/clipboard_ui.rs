// clipboard_ui.rs - クリップボード履歴 検索UI
//
// 責務:
// - egui ポップアップで履歴を表示（検索窓 + ページャー付きリスト）
// - キーワード検索（テキスト内容 + 日付）
// - 100件ごとのページャー（「...see more」で次のページ）
// - エントリ選択でクリップボードにコピー & ウィンドウを閉じる

use eframe::egui;
use std::time::{Duration, Instant};

use crate::clipboard_history::SharedStore;
use crate::clipboard_store::{ClipboardEntry, EntryType};

/// 1ページあたりの表示件数
const ITEMS_PER_PAGE: usize = 100;

/// クリップボード履歴ポップアップ
pub struct ClipboardHistoryPopup {
    store: SharedStore,
    search_query: String,
    current_page: usize,
    /// 表示中のエントリ（キャッシュ）
    display_entries: Vec<ClipboardEntry>,
    /// 検索結果の合計件数
    total_matches: usize,
    /// 検索が変更されたフラグ
    search_dirty: bool,
    /// 初回フレームフラグ
    first_frame: bool,
    /// フォーカス追跡
    had_focus: bool,
    created_at: Instant,
    /// 選択されたエントリのID（コピー用）
    selected_entry_id: Option<String>,
}

impl ClipboardHistoryPopup {
    pub fn new(store: SharedStore) -> Self {
        let mut popup = Self {
            store,
            search_query: String::new(),
            current_page: 0,
            display_entries: Vec::new(),
            total_matches: 0,
            search_dirty: true,
            first_frame: true,
            had_focus: false,
            created_at: Instant::now(),
            selected_entry_id: None,
        };
        popup.refresh_results();
        popup
    }

    fn refresh_results(&mut self) {
        if let Ok(store) = self.store.lock() {
            let (entries, total) =
                store.search(&self.search_query, self.current_page, ITEMS_PER_PAGE);
            self.display_entries = entries;
            self.total_matches = total;
        }
        self.search_dirty = false;
    }

    fn has_more_pages(&self) -> bool {
        (self.current_page + 1) * ITEMS_PER_PAGE < self.total_matches
    }

    /// エントリのテキスト内容を取得してクリップボードにコピー
    fn copy_entry_to_clipboard(&self, entry: &ClipboardEntry) {
        match entry.entry_type {
            EntryType::Text | EntryType::Json => {
                if let Some(ref text) = entry.text_content {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(text);
                    }
                }
            }
            EntryType::Image => {
                if let Some(ref blob_file) = entry.blob_file {
                    if let Ok(store) = self.store.lock() {
                        let path = store.blob_path(blob_file);
                        if let Ok(data) = std::fs::read(&path) {
                            // PNG → arboard::ImageData への変換は複雑なので
                            // ファイルパスをテキストとしてコピー（暫定）
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(path.to_string_lossy().as_ref());
                            }
                            let _ = data; // suppress warning
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for ClipboardHistoryPopup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // フォーカス喪失で閉じる
        let focused = is_current_process_foreground();
        if focused {
            self.had_focus = true;
        } else if self.had_focus || self.created_at.elapsed() > Duration::from_millis(1500) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Esc で閉じる
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // エントリ選択後の処理
        if let Some(ref id) = self.selected_entry_id.take() {
            if let Some(entry) = self.display_entries.iter().find(|e| &e.id == id) {
                self.copy_entry_to_clipboard(entry);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // 検索結果の更新
        if self.search_dirty {
            self.current_page = 0;
            self.refresh_results();
        }

        // --- UI ---
        let bg_color = egui::Color32::from_rgb(30, 30, 46);
        let fg_color = egui::Color32::from_rgb(205, 214, 244);
        let accent_color = egui::Color32::from_rgb(137, 180, 250);
        let hint_color = egui::Color32::from_rgb(108, 112, 134);
        let text_entry_color = egui::Color32::from_rgb(166, 227, 161);
        let json_entry_color = egui::Color32::from_rgb(250, 179, 135);
        let image_entry_color = egui::Color32::from_rgb(148, 226, 213);
        let border_color = egui::Color32::from_rgb(69, 71, 90);
        let hover_color = egui::Color32::from_rgb(49, 50, 68);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(bg_color)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ctx, |ui| {
                // ヘッダー
                ui.horizontal(|ui| {
                    ui.colored_label(
                        accent_color,
                        egui::RichText::new("📋 Clipboard History").size(14.0).strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let count_text = if let Ok(store) = self.store.lock() {
                            format!("{} items", store.len())
                        } else {
                            "---".to_string()
                        };
                        ui.colored_label(
                            hint_color,
                            egui::RichText::new(count_text).size(11.0),
                        );
                    });
                });

                ui.add_space(8.0);

                // 検索窓
                let search_response = ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::TextEdit::singleline(&mut self.search_query)
                        .font(egui::TextStyle::Body)
                        .text_color(fg_color)
                        .hint_text(
                            egui::RichText::new("🔍 検索 (キーワード / YYYY-MM-DD)...")
                                .color(hint_color),
                        ),
                );

                if self.first_frame {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    search_response.request_focus();
                    self.first_frame = false;
                }

                if search_response.changed() {
                    self.search_dirty = true;
                }

                ui.add_space(6.0);

                // 結果件数
                ui.colored_label(
                    hint_color,
                    egui::RichText::new(format!(
                        "{} 件中 {}-{} を表示",
                        self.total_matches,
                        if self.total_matches == 0 {
                            0
                        } else {
                            self.current_page * ITEMS_PER_PAGE + 1
                        },
                        (self.current_page * ITEMS_PER_PAGE + self.display_entries.len())
                            .min(self.total_matches),
                    ))
                    .size(10.0),
                );

                ui.add_space(4.0);

                // スクロールエリア
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for entry in &self.display_entries {
                            let (type_icon, type_color) = match entry.entry_type {
                                EntryType::Text => ("T", text_entry_color),
                                EntryType::Json => ("J", json_entry_color),
                                EntryType::Image => ("I", image_entry_color),
                            };

                            let time_str = entry.timestamp.format("%m/%d %H:%M").to_string();

                            // エントリ行（クリック可能）
                            let response = ui
                                .horizontal(|ui| {
                                    // 種別バッジ
                                    let badge = egui::RichText::new(type_icon)
                                        .size(10.0)
                                        .color(egui::Color32::from_rgb(30, 30, 46))
                                        .strong();

                                    let badge_rect = ui.available_rect_before_wrap();
                                    let badge_pos = badge_rect.left_top();
                                    let _ = badge_pos;

                                    ui.colored_label(type_color, badge);
                                    ui.add_space(4.0);

                                    // タイムスタンプ
                                    ui.colored_label(
                                        hint_color,
                                        egui::RichText::new(&time_str).size(10.0),
                                    );

                                    ui.add_space(4.0);

                                    // プレビュー（折り返さず1行で）
                                    let preview = if entry.preview.len() > 80 {
                                        format!(
                                            "{}...",
                                            entry.preview.chars().take(80).collect::<String>()
                                        )
                                    } else {
                                        entry.preview.clone()
                                    };
                                    ui.colored_label(
                                        fg_color,
                                        egui::RichText::new(preview).size(12.0),
                                    );
                                })
                                .response;

                            // ホバー時の背景
                            if response.hovered() {
                                ui.painter().rect_filled(
                                    response.rect,
                                    2.0,
                                    hover_color,
                                );
                            }

                            // クリックでコピー
                            if response.clicked() {
                                self.selected_entry_id = Some(entry.id.clone());
                            }

                            // 区切り線
                            ui.painter().line_segment(
                                [
                                    egui::pos2(response.rect.left(), response.rect.bottom()),
                                    egui::pos2(response.rect.right(), response.rect.bottom()),
                                ],
                                egui::Stroke::new(0.5, border_color),
                            );
                        }

                        // ページャー: 「...see more」
                        if self.has_more_pages() {
                            ui.add_space(8.0);
                            let see_more = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "...see more ({} remaining)",
                                        self.total_matches
                                            - (self.current_page + 1) * ITEMS_PER_PAGE
                                    ))
                                    .size(12.0)
                                    .color(accent_color),
                                )
                                .sense(egui::Sense::click()),
                            );

                            if see_more.clicked() {
                                self.current_page += 1;
                                self.refresh_results();
                            }

                            if see_more.hovered() {
                                ui.output_mut(|o| {
                                    o.cursor_icon = egui::CursorIcon::PointingHand;
                                });
                            }
                        }

                        // 結果なし
                        if self.display_entries.is_empty() && !self.search_query.is_empty() {
                            ui.add_space(20.0);
                            ui.colored_label(
                                hint_color,
                                egui::RichText::new("検索結果がありません").size(14.0),
                            );
                        } else if self.display_entries.is_empty() {
                            ui.add_space(20.0);
                            ui.colored_label(
                                hint_color,
                                egui::RichText::new(
                                    "クリップボード履歴はまだありません\nテキストや画像をコピーすると自動的に記録されます",
                                )
                                .size(13.0),
                            );
                        }
                    });
            });

        // 常にリペイント（検索変更の即時反映用）
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

/// クリップボード履歴ポップアップを表示する
pub fn show_clipboard_history(store: SharedStore) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(true),
        // 別スレッドからEventLoopを作成可能にする（tray.rsからthread::spawnで呼ばれるため）
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        })),
        ..Default::default()
    };

    eframe::run_native(
        "Lanch App - Clipboard History",
        options,
        Box::new(move |cc| {
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(ClipboardHistoryPopup::new(store)) as Box<dyn eframe::App>)
        }),
    )?;

    Ok(())
}

// --- ヘルパー ---

#[cfg(windows)]
fn is_current_process_foreground() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, &mut pid);
        pid == GetCurrentProcessId()
    }
}

#[cfg(not(windows))]
fn is_current_process_foreground() -> bool {
    true
}

/// Windows のシステムフォントから日本語対応フォントを読み込む
fn setup_japanese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_candidates = [
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
        r"C:\Windows\Fonts\msmincho.ttc",
    ];

    for path in &font_candidates {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "jp_font".to_owned(),
                egui::FontData::from_owned(font_data).into(),
            );

            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.push("jp_font".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("jp_font".to_owned());
            }

            ctx.set_fonts(fonts);
            return;
        }
    }
}
