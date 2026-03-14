// clipboard_ui.rs - クリップボード履歴 検索UI
//
// 責務:
// - egui ポップアップで履歴を表示（検索窓 + ページャー付きリスト + 詳細パネル）
// - キーワード検索（テキスト内容 + 日付）
// - 100件ごとのページャー（「...see more」で次のページ）
// - 上下キーでエントリ選択、Enterでコピー&閉じる
// - エントリ選択でクリップボードにコピー & ウィンドウを閉じる
// - 選択中エントリの詳細表示（テキスト全文 / 画像サムネイル）

use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::clipboard;
use crate::clipboard_history::SharedStore;
use crate::clipboard_store::{ClipboardEntry, EntryType};

/// 1ページあたりの表示件数
const ITEMS_PER_PAGE: usize = 100;

/// 詳細パネルのテキスト表示最大文字数
const DETAIL_TEXT_MAX_CHARS: usize = 1000;

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
    /// キーボード選択中のインデックス（-1 = 未選択 / 検索窓にフォーカス）
    selected_index: i32,
    /// 画像テクスチャのキャッシュ (blob_file名 → TextureHandle)
    image_cache: HashMap<String, egui::TextureHandle>,
    /// エントリ選択によるペーストが必要か（ウィンドウ終了後に参照）
    paste_after_close: Arc<AtomicBool>,
}

impl ClipboardHistoryPopup {
    pub fn new(store: SharedStore, paste_after_close: Arc<AtomicBool>) -> Self {
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
            selected_index: -1,
            image_cache: HashMap::new(),
            paste_after_close,
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
                        if let Ok(png_data) = std::fs::read(&path) {
                            if let Ok(img) = image::load_from_memory_with_format(&png_data, image::ImageFormat::Png) {
                                let rgba = img.to_rgba8();
                                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                                let img_data = arboard::ImageData {
                                    width: w,
                                    height: h,
                                    bytes: std::borrow::Cow::Owned(rgba.into_raw()),
                                };
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    let _ = cb.set_image(img_data);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// キーボードナビゲーション処理。戻り値: true = ウィンドウを閉じる
    fn handle_keyboard_navigation(&mut self, ctx: &egui::Context) -> bool {
        let entry_count = self.display_entries.len() as i32;

        // ↓キー: 次のエントリへ
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            if self.selected_index < entry_count - 1 {
                self.selected_index += 1;
            }
            return false;
        }

        // ↑キー: 前のエントリへ（-1 = 検索窓に戻る）
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            if self.selected_index > -1 {
                self.selected_index -= 1;
            }
            return false;
        }

        // Enter キー: 選択中のエントリをコピーして閉じる
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            if self.selected_index >= 0 && (self.selected_index as usize) < self.display_entries.len() {
                self.selected_entry_id = Some(self.display_entries[self.selected_index as usize].id.clone());
            }
            return false;
        }

        false
    }

    /// 選択中エントリの画像をロードしてテクスチャキャッシュに登録
    ///
    /// 大きな画像はデコード後にリサイズしてからテクスチャ化する。
    /// デコード失敗時はNoneを返す（パニックしない）。
    fn load_image_texture(&mut self, ctx: &egui::Context, blob_file: &str) -> Option<egui::TextureHandle> {
        // キャッシュにあればそれを返す
        if let Some(handle) = self.image_cache.get(blob_file) {
            return Some(handle.clone());
        }

        // blobファイルからPNGデータを読み込み
        let path = if let Ok(store) = self.store.lock() {
            store.blob_path(blob_file)
        } else {
            return None;
        };

        let png_data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("[clipboard_ui] 画像ファイル読み込み失敗: {}: {}", path.display(), e);
                return None;
            }
        };

        let (w, h, rgba_buf) = match decode_and_resize_png(&png_data) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("[clipboard_ui] {}: {}", blob_file, e);
                return None;
            }
        };

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            &rgba_buf,
        );

        let texture = ctx.load_texture(
            format!("clipboard_img_{}", blob_file),
            color_image,
            egui::TextureOptions::LINEAR,
        );

        self.image_cache.insert(blob_file.to_string(), texture.clone());
        Some(texture)
    }

    /// 詳細パネルを描画する
    fn render_detail_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let hint_color = egui::Color32::from_rgb(108, 112, 134);
        let fg_color = egui::Color32::from_rgb(205, 214, 244);
        let accent_color = egui::Color32::from_rgb(137, 180, 250);

        // 選択中のエントリを取得
        let selected = if self.selected_index >= 0
            && (self.selected_index as usize) < self.display_entries.len()
        {
            Some(self.display_entries[self.selected_index as usize].clone())
        } else {
            None
        };

        match selected {
            None => {
                // 未選択時のプレースホルダー
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.colored_label(
                        hint_color,
                        egui::RichText::new("↑↓ キーでエントリを選択\n詳細がここに表示されます")
                            .size(13.0),
                    );
                });
            }
            Some(entry) => {
                // ヘッダー: 種別 + 日時 + サイズ
                ui.colored_label(
                    accent_color,
                    egui::RichText::new(format!(
                        "{} | {} | {}",
                        entry.entry_type,
                        entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                        format_size(entry.size_bytes),
                    ))
                    .size(11.0),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                match entry.entry_type {
                    EntryType::Text | EntryType::Json => {
                        // テキスト内容を表示（1000文字超は切り詰め）
                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                if let Some(ref text) = entry.text_content {
                                    let display_text = if text.chars().count() > DETAIL_TEXT_MAX_CHARS {
                                        let truncated: String = text.chars().take(DETAIL_TEXT_MAX_CHARS).collect();
                                        format!("{}...", truncated)
                                    } else {
                                        text.clone()
                                    };

                                    // JSONの場合はモノスペースで表示
                                    let text_style = if entry.entry_type == EntryType::Json {
                                        egui::RichText::new(&display_text)
                                            .size(12.0)
                                            .color(fg_color)
                                            .family(egui::FontFamily::Monospace)
                                    } else {
                                        egui::RichText::new(&display_text)
                                            .size(12.0)
                                            .color(fg_color)
                                    };
                                    ui.label(text_style);

                                    if text.chars().count() > DETAIL_TEXT_MAX_CHARS {
                                        ui.add_space(4.0);
                                        ui.colored_label(
                                            hint_color,
                                            egui::RichText::new(format!(
                                                "({} 文字中 {} 文字を表示)",
                                                text.chars().count(),
                                                DETAIL_TEXT_MAX_CHARS,
                                            ))
                                            .size(10.0),
                                        );
                                    }
                                } else {
                                    ui.colored_label(
                                        hint_color,
                                        egui::RichText::new("(テキストデータなし)").size(12.0),
                                    );
                                }
                            });
                    }
                    EntryType::Image => {
                        // 画像サムネイルを表示
                        if let Some(ref blob_file) = entry.blob_file {
                            let blob_key = blob_file.clone();
                            let available = ui.available_size();
                            // 表示領域が小さすぎる場合はスキップ（負サイズパニック防止）
                            if available.x < 10.0 || available.y < 10.0 {
                                ui.colored_label(
                                    hint_color,
                                    egui::RichText::new("(表示領域が小さすぎます)").size(12.0),
                                );
                            } else if let Some(texture) = self.load_image_texture(ctx, &blob_key) {
                                let tex_size = texture.size_vec2();
                                let (dw, dh) = calculate_image_display_size(
                                    tex_size.x, tex_size.y, available.x, available.y,
                                );

                                egui::ScrollArea::both()
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        ui.image(egui::load::SizedTexture::new(
                                            texture.id(),
                                            egui::vec2(dw.max(1.0), dh.max(1.0)),
                                        ));
                                    });
                            } else {
                                ui.colored_label(
                                    hint_color,
                                    egui::RichText::new("(画像の読み込みに失敗しました)").size(12.0),
                                );
                            }
                        } else {
                            ui.colored_label(
                                hint_color,
                                egui::RichText::new("(画像ファイルが見つかりません)").size(12.0),
                            );
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

        // キーボードナビゲーション
        if self.handle_keyboard_navigation(ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // エントリ選択後の処理: コピー → ウィンドウ閉じる → フォーカス復元 & ペースト
        if let Some(ref id) = self.selected_entry_id.take() {
            if let Some(entry) = self.display_entries.iter().find(|e| &e.id == id) {
                self.copy_entry_to_clipboard(entry);
                self.paste_after_close.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // 検索結果の更新
        if self.search_dirty {
            self.current_page = 0;
            self.selected_index = -1; // 検索変更時にリセット
            self.refresh_results();
        }

        // --- カラーテーマ ---
        let bg_color = egui::Color32::from_rgb(30, 30, 46);
        let fg_color = egui::Color32::from_rgb(205, 214, 244);
        let accent_color = egui::Color32::from_rgb(137, 180, 250);
        let hint_color = egui::Color32::from_rgb(108, 112, 134);
        let text_entry_color = egui::Color32::from_rgb(166, 227, 161);
        let json_entry_color = egui::Color32::from_rgb(250, 179, 135);
        let image_entry_color = egui::Color32::from_rgb(148, 226, 213);
        let border_color = egui::Color32::from_rgb(69, 71, 90);
        let hover_color = egui::Color32::from_rgb(49, 50, 68);
        // 選択ハイライト: 黄色系
        let selected_color = egui::Color32::from_rgba_premultiplied(250, 227, 80, 40);
        let selected_border_color = egui::Color32::from_rgb(250, 227, 80);
        let panel_divider_color = egui::Color32::from_rgb(69, 71, 90);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(bg_color)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ctx, |ui| {
                // ヘッダー（ドラッグで移動可能）
                let header_response = ui.horizontal(|ui| {
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
                }).response.interact(egui::Sense::drag());
                if header_response.dragged() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                ui.add_space(8.0);

                // 検索窓
                let search_response = ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::TextEdit::singleline(&mut self.search_query)
                        .font(egui::TextStyle::Body)
                        .text_color(fg_color)
                        .hint_text(
                            egui::RichText::new("🔍 検索 (キーワード / YYYY-MM-DD / ↑↓で選択 / Enterで貼付)")
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

                // ===== 水平分割レイアウト: リスト(左) | 詳細パネル(右) =====
                let available_width = ui.available_width();
                let available_height = ui.available_height();
                let list_width = available_width * 0.45; // 左45%

                let scroll_to_index = self.selected_index;

                let remaining_rect = ui.available_rect_before_wrap();
                ui.allocate_ui_at_rect(remaining_rect, |ui| {
                  ui.horizontal(|ui| {
                    ui.set_min_height(available_height);

                    // --- 左パネル: エントリリスト ---
                    ui.vertical(|ui| {
                        ui.set_width(list_width);
                        ui.set_min_height(available_height);

                        egui::ScrollArea::vertical()
                            .id_salt("entry_list")
                            .auto_shrink([false; 2])
                            .max_height(available_height)
                            .show(ui, |ui| {
                                for (i, entry) in self.display_entries.iter().enumerate() {
                                    let is_selected = self.selected_index == i as i32;

                                    let (type_icon, type_color) = match entry.entry_type {
                                        EntryType::Text => ("T", text_entry_color),
                                        EntryType::Json => ("J", json_entry_color),
                                        EntryType::Image => ("I", image_entry_color),
                                    };

                                    let time_str = entry.timestamp.format("%m/%d %H:%M").to_string();

                                    // エントリ行（クリック可能）
                                    let response = ui
                                        .horizontal(|ui| {
                                            // 選択インジケーター
                                            if is_selected {
                                                ui.colored_label(
                                                    selected_border_color,
                                                    egui::RichText::new("▸").size(12.0),
                                                );
                                            } else {
                                                ui.add_space(14.0);
                                            }

                                            // 種別バッジ
                                            let badge = egui::RichText::new(type_icon)
                                                .size(10.0)
                                                .color(egui::Color32::from_rgb(30, 30, 46))
                                                .strong();
                                            ui.colored_label(type_color, badge);
                                            ui.add_space(4.0);

                                            // タイムスタンプ
                                            ui.colored_label(
                                                hint_color,
                                                egui::RichText::new(&time_str).size(10.0),
                                            );

                                            ui.add_space(4.0);

                                            // プレビュー（折り返さず1行で）
                                            let preview = if entry.preview.len() > 50 {
                                                format!(
                                                    "{}...",
                                                    entry.preview.chars().take(50).collect::<String>()
                                                )
                                            } else {
                                                entry.preview.clone()
                                            };

                                            // 選択中は太字 + 黄色テキストで表示
                                            let text = if is_selected {
                                                egui::RichText::new(preview)
                                                    .size(12.0)
                                                    .strong()
                                                    .color(selected_border_color)
                                            } else {
                                                egui::RichText::new(preview)
                                                    .size(12.0)
                                                    .color(fg_color)
                                            };
                                            ui.label(text);
                                        })
                                        .response;

                                    // 選択中 or ホバー時の背景
                                    if is_selected {
                                        // 選択行: 黄色の半透明背景 + 左ボーダー
                                        ui.painter().rect_filled(
                                            response.rect,
                                            2.0,
                                            selected_color,
                                        );
                                        // 左側に黄色のバー
                                        let bar_rect = egui::Rect::from_min_size(
                                            response.rect.left_top(),
                                            egui::vec2(3.0, response.rect.height()),
                                        );
                                        ui.painter().rect_filled(bar_rect, 1.0, selected_border_color);
                                    } else if response.hovered() {
                                        ui.painter().rect_filled(
                                            response.rect,
                                            2.0,
                                            hover_color,
                                        );
                                    }

                                    // クリックで選択（ダブルクリックでコピー）
                                    if response.clicked() {
                                        self.selected_index = i as i32;
                                    }
                                    if response.double_clicked() {
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

                                    // キーボード選択時に自動スクロール
                                    if is_selected && scroll_to_index >= 0 {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                    }
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
                                            "履歴なし\nコピーすると自動記録",
                                        )
                                        .size(13.0),
                                    );
                                }
                            });
                    });

                    // --- 中央の区切り線 ---
                    let divider_rect = ui.available_rect_before_wrap();
                    ui.painter().line_segment(
                        [
                            egui::pos2(divider_rect.left(), divider_rect.top()),
                            egui::pos2(divider_rect.left(), divider_rect.bottom()),
                        ],
                        egui::Stroke::new(1.0, panel_divider_color),
                    );
                    ui.add_space(8.0);

                    // --- 右パネル: 詳細表示 ---
                    ui.vertical(|ui| {
                        ui.set_min_height(available_height);
                        self.render_detail_panel(ui, ctx);
                    });
                });
                });
            });

        // 常にリペイント（検索変更の即時反映用）
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

/// クリップボード履歴ポップアップを表示する
pub fn show_clipboard_history(store: SharedStore) -> Result<(), Box<dyn std::error::Error>> {
    // ポップアップ起動前のフォアグラウンドウィンドウを記憶
    let previous_hwnd = clipboard::get_foreground_hwnd();
    let paste_flag = Arc::new(AtomicBool::new(false));
    let paste_flag_clone = paste_flag.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 600.0])
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
            Ok(Box::new(ClipboardHistoryPopup::new(store, paste_flag_clone)) as Box<dyn eframe::App>)
        }),
    )?;

    // ウィンドウ終了後: エントリ選択があった場合は元ウィンドウにフォーカスを戻してペースト
    if paste_flag.load(Ordering::SeqCst) && previous_hwnd != 0 {
        clipboard::restore_focus_and_paste(previous_hwnd);
    }

    Ok(())
}

// =============================================================================
// ヘルパー関数
// =============================================================================

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

/// サイズを人間可読な文字列に変換
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// PNGデータをデコードし、大きな画像はリサイズしてRGBA8バッファを返す。
///
/// 戻り値: (幅, 高さ, RGBAバイト列)。デコード失敗時はErrを返す。
fn decode_and_resize_png(png_data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory_with_format(png_data, image::ImageFormat::Png)
        .map_err(|e| format!("PNG デコード失敗: {}", e))?;

    const MAX_TEXTURE_DIM: u32 = 1024;
    let img = if img.width() > MAX_TEXTURE_DIM || img.height() > MAX_TEXTURE_DIM {
        img.resize(MAX_TEXTURE_DIM, MAX_TEXTURE_DIM, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());

    if w == 0 || h == 0 {
        return Err(format!("画像サイズが0: {}x{}", w, h));
    }

    Ok((w, h, rgba.into_raw()))
}

/// 画像の表示サイズを計算する（アスペクト比維持、利用可能領域にフィット）。
///
/// - `tex_w`, `tex_h`: テクスチャの元サイズ
/// - `available_w`, `available_h`: 利用可能な描画領域（ヘッダー余白差し引き前の生値）
///
/// 戻り値: (表示幅, 表示高さ)。常に正の値を返す。
fn calculate_image_display_size(tex_w: f32, tex_h: f32, available_w: f32, available_h: f32) -> (f32, f32) {
    let avail_w = available_w.max(1.0);
    let avail_h = (available_h - 30.0).max(1.0); // ヘッダー分の余白

    let scale = (avail_w / tex_w.max(1.0)).min(avail_h / tex_h.max(1.0)).max(0.01);

    (tex_w * scale, tex_h * scale)
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // format_size のユニットテスト
    // -------------------------------------------------------------------------

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1023), "1023.0 KB");
    }

    #[test]
    fn test_format_size_mb() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(11 * 1024 * 1024), "11.0 MB");
    }

    // -------------------------------------------------------------------------
    // calculate_image_display_size のユニットテスト
    // -------------------------------------------------------------------------

    #[test]
    fn test_calc_display_size_normal_landscape() {
        // 横長画像 800x400 を 600x400 領域に表示
        let (w, h) = calculate_image_display_size(800.0, 400.0, 600.0, 430.0);
        // avail_h = 430-30 = 400, scale = min(600/800, 400/400) = min(0.75, 1.0) = 0.75
        assert!((w - 600.0).abs() < 0.1);
        assert!((h - 300.0).abs() < 0.1);
    }

    #[test]
    fn test_calc_display_size_normal_portrait() {
        // 縦長画像 400x800 を 600x430 領域に表示
        let (w, h) = calculate_image_display_size(400.0, 800.0, 600.0, 430.0);
        // avail_h = 400, scale = min(600/400, 400/800) = min(1.5, 0.5) = 0.5
        assert!((w - 200.0).abs() < 0.1);
        assert!((h - 400.0).abs() < 0.1);
    }

    #[test]
    fn test_calc_display_size_square() {
        let (w, h) = calculate_image_display_size(500.0, 500.0, 300.0, 330.0);
        // avail_h = 300, scale = min(300/500, 300/500) = 0.6
        assert!((w - 300.0).abs() < 0.1);
        assert!((h - 300.0).abs() < 0.1);
    }

    #[test]
    fn test_calc_display_size_negative_available_height() {
        // BUG-1 の再現: available_h が 30 未満 → avail_h が負になっていた
        let (w, h) = calculate_image_display_size(100.0, 100.0, 50.0, 10.0);
        // avail_h = max(10-30, 1) = 1, avail_w = 50
        // scale = min(50/100, 1/100) = 0.01
        assert!(w > 0.0, "幅は正であること: {}", w);
        assert!(h > 0.0, "高さは正であること: {}", h);
    }

    #[test]
    fn test_calc_display_size_zero_available() {
        let (w, h) = calculate_image_display_size(100.0, 100.0, 0.0, 0.0);
        assert!(w > 0.0);
        assert!(h > 0.0);
    }

    #[test]
    fn test_calc_display_size_negative_available() {
        let (w, h) = calculate_image_display_size(100.0, 100.0, -50.0, -20.0);
        assert!(w > 0.0);
        assert!(h > 0.0);
    }

    #[test]
    fn test_calc_display_size_very_large_texture() {
        // 巨大テクスチャを小さい領域に表示
        let (w, h) = calculate_image_display_size(4000.0, 3000.0, 400.0, 330.0);
        assert!(w <= 400.0 + 0.1);
        assert!(h <= 300.0 + 0.1);
        assert!(w > 0.0);
        assert!(h > 0.0);
    }

    #[test]
    fn test_calc_display_size_tiny_texture() {
        // 1x1 テクスチャ
        let (w, h) = calculate_image_display_size(1.0, 1.0, 600.0, 430.0);
        assert!(w > 0.0);
        assert!(h > 0.0);
    }

    #[test]
    fn test_calc_display_size_zero_texture() {
        // テクスチャサイズ 0 （ガード: max(1.0)で除算保護）
        let (w, h) = calculate_image_display_size(0.0, 0.0, 600.0, 430.0);
        assert!(w.is_finite());
        assert!(h.is_finite());
    }

    // -------------------------------------------------------------------------
    // decode_and_resize_png のユニットテスト
    // -------------------------------------------------------------------------

    /// テスト用の最小限の有効なPNGを生成する（clipboard_historyのencode関数に依存しない）
    fn create_test_png(width: u32, height: u32) -> Vec<u8> {
        use image::{ImageBuffer, Rgba};
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_fn(width, height, |x, y| {
                Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
            });
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn test_decode_valid_small_png() {
        let png = create_test_png(4, 4);
        let result = decode_and_resize_png(&png);
        assert!(result.is_ok());
        let (w, h, rgba) = result.unwrap();
        assert_eq!(w, 4);
        assert_eq!(h, 4);
        assert_eq!(rgba.len(), 4 * 4 * 4); // RGBA
    }

    #[test]
    fn test_decode_1x1_png() {
        let png = create_test_png(1, 1);
        let (w, h, rgba) = decode_and_resize_png(&png).unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        assert_eq!(rgba.len(), 4);
    }

    #[test]
    fn test_decode_invalid_data_returns_err() {
        let result = decode_and_resize_png(b"not a png");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("デコード失敗"));
    }

    #[test]
    fn test_decode_empty_data_returns_err() {
        let result = decode_and_resize_png(b"");
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_truncated_png_returns_err() {
        let png = create_test_png(4, 4);
        // PNGヘッダーだけ残して切り詰め
        let truncated = &png[..8];
        let result = decode_and_resize_png(truncated);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_large_image_gets_resized() {
        // 2048x1536 → 最大辺 1024 にリサイズされるはず
        let png = create_test_png(2048, 1536);
        let (w, h, _) = decode_and_resize_png(&png).unwrap();
        assert!(w <= 1024, "幅が1024以下: {}", w);
        assert!(h <= 1024, "高さが1024以下: {}", h);
        // アスペクト比が維持されること
        let ratio_orig = 2048.0 / 1536.0;
        let ratio_resized = w as f64 / h as f64;
        assert!((ratio_orig - ratio_resized).abs() < 0.05, "アスペクト比維持: {} vs {}", ratio_orig, ratio_resized);
    }

    #[test]
    fn test_decode_exact_1024_not_resized() {
        let png = create_test_png(1024, 768);
        let (w, h, _) = decode_and_resize_png(&png).unwrap();
        assert_eq!(w, 1024);
        assert_eq!(h, 768);
    }

    #[test]
    fn test_decode_just_over_1024_gets_resized() {
        let png = create_test_png(1025, 512);
        let (w, h, _) = decode_and_resize_png(&png).unwrap();
        assert!(w <= 1024);
        assert!(h <= 1024);
    }

    #[test]
    fn test_decode_rgba_buffer_size_matches() {
        let png = create_test_png(100, 50);
        let (w, h, rgba) = decode_and_resize_png(&png).unwrap();
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }

    // -------------------------------------------------------------------------
    // 結合テスト: PNGファイルの読み書き + デコード + サイズ計算の一連の流れ
    // -------------------------------------------------------------------------

    #[test]
    fn test_integration_png_file_decode_and_display_size() {
        // 1. テスト用PNGファイルを一時ディレクトリに作成
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("test_image.png");
        let png_data = create_test_png(800, 600);
        std::fs::write(&png_path, &png_data).unwrap();

        // 2. ファイルから読み込み + デコード
        let loaded = std::fs::read(&png_path).unwrap();
        let (w, h, rgba) = decode_and_resize_png(&loaded).unwrap();
        assert_eq!(w, 800);
        assert_eq!(h, 600);
        assert_eq!(rgba.len(), (800 * 600 * 4) as usize);

        // 3. 表示サイズ計算（1100x600 のウィンドウ右55%パネル相当）
        let (dw, dh) = calculate_image_display_size(w as f32, h as f32, 605.0, 500.0);
        assert!(dw > 0.0 && dw <= 605.0);
        assert!(dh > 0.0 && dh <= 470.0); // 500 - 30 = 470
    }

    #[test]
    fn test_integration_large_png_file_resize_and_display() {
        // 大きなPNG → リサイズ → 表示サイズ計算の一連フロー
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("large_image.png");
        let png_data = create_test_png(2000, 1500);
        std::fs::write(&png_path, &png_data).unwrap();

        let loaded = std::fs::read(&png_path).unwrap();
        let (w, h, rgba) = decode_and_resize_png(&loaded).unwrap();

        // リサイズされて1024以下になること
        assert!(w <= 1024);
        assert!(h <= 1024);
        assert_eq!(rgba.len(), (w * h * 4) as usize);

        // 表示サイズは正の値
        let (dw, dh) = calculate_image_display_size(w as f32, h as f32, 605.0, 500.0);
        assert!(dw > 0.0);
        assert!(dh > 0.0);
    }

    #[test]
    fn test_integration_corrupt_file_graceful_error() {
        // 壊れたファイル → デコードエラーが返る（パニックしない）
        let dir = tempfile::tempdir().unwrap();
        let corrupt_path = dir.path().join("corrupt.png");
        std::fs::write(&corrupt_path, b"this is not a valid PNG file").unwrap();

        let loaded = std::fs::read(&corrupt_path).unwrap();
        let result = decode_and_resize_png(&loaded);
        assert!(result.is_err());
    }

    #[test]
    fn test_integration_missing_file_io_error() {
        // 存在しないファイル → fs::read がエラー
        let result = std::fs::read("/nonexistent/path/image.png");
        assert!(result.is_err());
    }

    #[test]
    fn test_integration_bug1_scenario_negative_size_no_panic() {
        // BUG-1 の再現シナリオ: 画像デコード成功 → 表示サイズ計算で負の値
        let png = create_test_png(1920, 1080);
        let (w, h, _) = decode_and_resize_png(&png).unwrap();

        // 初期フレーム等で利用可能領域がほぼゼロのケース
        let scenarios: Vec<(f32, f32)> = vec![
            (0.0, 0.0),       // ゼロ
            (-5.0, -3.0),     // 負の値
            (10.0, 20.0),     // ヘッダー(30px)より小さい → avail_h が負になっていた
            (1.0, 1.0),       // 極小
            (100.0, 29.0),    // avail_h = max(29-30, 1) = 1
            (0.5, 0.5),       // 1未満
        ];

        for (avail_w, avail_h) in scenarios {
            let (dw, dh) = calculate_image_display_size(w as f32, h as f32, avail_w, avail_h);
            assert!(dw > 0.0, "avail=({},{}) → dw={} は正であること", avail_w, avail_h, dw);
            assert!(dh > 0.0, "avail=({},{}) → dh={} は正であること", avail_w, avail_h, dh);
            assert!(dw.is_finite(), "dw は有限値であること");
            assert!(dh.is_finite(), "dh は有限値であること");
        }
    }
}

