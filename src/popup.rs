// popup.rs - egui ポップアップウィンドウ
//
// 翻訳ポップアップ + Markdown整形結果表示ポップアップ
//
// ポップアップの種類:
//   1. TranslatePopup - テキスト入力 → リアルタイム翻訳
//   2. ResultPopup - 翻訳結果の表示（選択テキスト翻訳用）
//   3. FormatResultPopup - Markdown整形結果の表示

use eframe::egui;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::translator;

// ------------------------------------------------------------
// 単一起動ロック（ポップアップ多重起動防止）
// ------------------------------------------------------------

#[cfg(windows)]
struct PopupInstanceGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for PopupInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn acquire_popup_instance_lock(name: &str) -> Option<PopupInstanceGuard> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let mut name_wide: Vec<u16> = name.encode_utf16().collect();
    name_wide.push(0);

    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 1, name_wide.as_ptr());
        if handle.is_null() {
            return None;
        }

        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = windows_sys::Win32::Foundation::CloseHandle(handle);
            return None;
        }

        Some(PopupInstanceGuard { handle })
    }
}

#[cfg(not(windows))]
fn acquire_popup_instance_lock(_name: &str) -> Option<()> {
    Some(())
}

fn popup_request_file_path() -> std::path::PathBuf {
    std::env::temp_dir().join("quick_translate_popup_request.txt")
}

fn consume_popup_request(path: &std::path::Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let _ = fs::remove_file(path);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(windows)]
fn is_current_process_foreground() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

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
        if let Ok(font_data) = fs::read(path) {
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

    eprintln!("警告: 日本語フォントが見つかりませんでした");
}

// ============================================================
// 翻訳入力ポップアップ
// ============================================================

pub struct TranslatePopup {
    config: Config,
    input_text: String,
    result_text: String,
    current_engine: String,
    last_input_change: Option<Instant>,
    is_translating: bool,
    result_receiver: Option<mpsc::Receiver<Result<translator::TranslationResult, String>>>,
    first_frame: bool,
    copy_requested: bool,
    had_focus: bool,
    request_file_path: std::path::PathBuf,
    last_request_poll: Instant,
    created_at: Instant,
    last_viewport_size: Option<(f32, f32)>,
}

impl TranslatePopup {
    pub fn new(config: Config, initial_text: String) -> Self {
        let request_file_path = popup_request_file_path();
        let initial_text = if initial_text.trim().is_empty() {
            consume_popup_request(&request_file_path).unwrap_or_default()
        } else {
            initial_text
        };

        Self {
            current_engine: config.engine.clone(),
            config,
            input_text: initial_text.clone(),
            result_text: String::new(),
            last_input_change: if initial_text.is_empty() {
                None
            } else {
                Some(Instant::now() - std::time::Duration::from_secs(1))
            },
            is_translating: false,
            result_receiver: None,
            first_frame: true,
            copy_requested: false,
            had_focus: false,
            request_file_path,
            last_request_poll: Instant::now() - Duration::from_secs(1),
            created_at: Instant::now(),
            last_viewport_size: None,
        }
    }

    fn check_external_request(&mut self) {
        if self.last_request_poll.elapsed() < Duration::from_millis(80) {
            return;
        }
        self.last_request_poll = Instant::now();

        let Some(text) = consume_popup_request(&self.request_file_path) else {
            return;
        };

        if text == self.input_text {
            return;
        }

        self.input_text = text;
        self.first_frame = true;
        self.start_translation();
    }

    fn start_translation(&mut self) {
        let text = self.input_text.trim().to_string();
        if text.is_empty() {
            self.result_text.clear();
            return;
        }

        self.is_translating = true;
        self.result_text = "翻訳中...".to_string();

        let (tx, rx) = mpsc::channel();
        self.result_receiver = Some(rx);

        let config = self.config.clone();

        thread::spawn(move || {
            let result = translator::translate(&text, &config);
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }

    fn check_translation_result(&mut self) {
        if let Some(ref rx) = self.result_receiver {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    self.result_text = result.translated;
                    self.is_translating = false;
                    self.result_receiver = None;
                }
                Ok(Err(error)) => {
                    self.result_text = format!("エラー: {}", error);
                    self.is_translating = false;
                    self.result_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.result_text = "翻訳スレッドが予期せず終了しました".to_string();
                    self.is_translating = false;
                    self.result_receiver = None;
                }
            }
        }
    }

    fn adjust_viewport_size(&mut self, ctx: &egui::Context) {
        let (width, height) =
            estimate_live_popup_size(&self.input_text, &self.result_text, self.config.font_size);
        let should_resize = match self.last_viewport_size {
            None => true,
            Some((w, h)) => (w - width).abs() > 6.0 || (h - height).abs() > 6.0,
        };

        if should_resize {
            self.last_viewport_size = Some((width, height));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
        }
    }
}

impl eframe::App for TranslatePopup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = is_current_process_foreground();
        if focused {
            self.had_focus = true;
        } else if self.had_focus || self.created_at.elapsed() > Duration::from_millis(1500) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        self.check_external_request();
        self.check_translation_result();
        self.adjust_viewport_size(ctx);

        if let Some(last_change) = self.last_input_change {
            if last_change.elapsed().as_millis() >= 400 && !self.is_translating {
                self.last_input_change = None;
                self.start_translation();
            }
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
            if !self.result_text.is_empty() && self.result_text != "翻訳中..." {
                self.copy_requested = true;
            }
        }

        if self.copy_requested {
            self.copy_requested = false;
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(&self.result_text);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // --- UI ---
        let bg_color = egui::Color32::from_rgb(30, 30, 46);
        let fg_color = egui::Color32::from_rgb(205, 214, 244);
        let accent_color = egui::Color32::from_rgb(137, 180, 250);
        let result_color = egui::Color32::from_rgb(166, 227, 161);
        let hint_color = egui::Color32::from_rgb(108, 112, 134);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(bg_color)
                    .inner_margin(egui::Margin::same(16))
            )
            .show(ctx, |ui| {
                // ヘッダー（ドラッグで移動可能）
                let header_response = ui.horizontal(|ui| {
                    ui.colored_label(
                        accent_color,
                        egui::RichText::new(format!("⚡ {}", self.current_engine.to_uppercase()))
                            .size(12.0)
                            .strong(),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(
                            hint_color,
                            egui::RichText::new("Ctrl+Enter=コピー | Esc=閉じる").size(10.0),
                        );
                    });
                }).response.interact(egui::Sense::drag());
                if header_response.dragged() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                ui.add_space(8.0);

                let input_response = ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut self.input_text)
                        .font(egui::TextStyle::Heading)
                        .text_color(fg_color)
                        .hint_text(
                            egui::RichText::new("翻訳するテキストを入力...")
                                .color(hint_color),
                        ),
                );

                if self.first_frame {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    input_response.request_focus();
                    self.first_frame = false;
                }

                if input_response.changed() {
                    self.last_input_change = Some(Instant::now());
                }

                ui.add_space(12.0);

                if !self.result_text.is_empty() {
                    let color = if self.result_text.starts_with("エラー") || self.result_text.starts_with("翻訳中") {
                        hint_color
                    } else {
                        result_color
                    };

                    ui.colored_label(
                        color,
                        egui::RichText::new(&self.result_text)
                            .size(self.config.font_size),
                    );
                }
            });

        if self.is_translating || self.last_input_change.is_some() {
            ctx.request_repaint();
        }
    }
}

fn count_wrapped_lines(text: &str, chars_per_line: usize) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let per_line = chars_per_line.max(8);
    let mut lines = 0usize;
    for line in text.lines() {
        let n = line.chars().count().max(1);
        lines += ((n + per_line - 1) / per_line).max(1);
    }
    lines.max(1)
}

fn estimate_live_popup_size(input_text: &str, result_text: &str, font_size: f32) -> (f32, f32) {
    let char_width = font_size * 0.68;
    let width_chars = if !result_text.trim().is_empty() {
        let result_max = result_text
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(24);
        let has_spaces = result_text.contains(' ');
        let preferred = if has_spaces { 52 } else { 34 };
        result_max.min(preferred + 14).max(28)
    } else {
        let input_max = input_text
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(24)
            .min(60);
        input_max.max(24)
    };
    let width = (width_chars as f32 * char_width + 88.0).clamp(520.0, 860.0);

    let text_area_width = (width - 32.0).max(220.0);
    let chars_per_line = (text_area_width / char_width).floor() as usize;
    let result_lines = count_wrapped_lines(result_text, chars_per_line);
    let result_height = result_lines as f32 * font_size * 1.45;

    let height = (32.0 + 8.0 + 32.0 + 12.0 + result_height + 28.0)
        .clamp(190.0, 840.0);

    (width, height)
}

pub fn show_popup(config: Config, initial_text: String) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = match acquire_popup_instance_lock("QuickToolsPopupSingleton") {
        Some(g) => g,
        None => return Ok(()),
    };

    let (initial_width, initial_height) =
        estimate_live_popup_size(&initial_text, "", config.font_size);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([initial_width, initial_height])
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        "Quick Tools",
        options,
        Box::new(move |cc| {
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(TranslatePopup::new(config, initial_text)) as Box<dyn eframe::App>)
        }),
    )?;

    Ok(())
}

// ============================================================
// 結果表示ポップアップ（選択テキスト翻訳用）
// ============================================================

struct ResultPopup {
    translated: String,
    original: String,
    font_size: f32,
    had_focus: bool,
    created_at: Instant,
    /// ポップアップのタイプラベル（"翻訳" / "Markdown" など）
    mode_label: String,
    /// 結果テキストの色
    result_color: egui::Color32,
}

impl eframe::App for ResultPopup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = is_current_process_foreground();
        if focused {
            self.had_focus = true;
        } else if self.had_focus || self.created_at.elapsed() > Duration::from_millis(1500) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(&self.translated);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let bg_color = egui::Color32::from_rgb(30, 30, 46);
        let original_color = egui::Color32::from_rgb(108, 112, 134);
        let hint_color = egui::Color32::from_rgb(88, 91, 112);
        let accent_color = egui::Color32::from_rgb(137, 180, 250);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(bg_color)
                    .inner_margin(egui::Margin::same(16))
            )
            .show(ctx, |ui| {
                // モードラベル（ドラッグで移動可能）
                let header_response = ui.horizontal(|ui| {
                    ui.colored_label(
                        accent_color,
                        egui::RichText::new(&self.mode_label)
                            .size(11.0)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(
                            hint_color,
                            egui::RichText::new("Ctrl+Enter=コピー | Esc=閉じる").size(10.0),
                        );
                    });
                }).response.interact(egui::Sense::drag());
                if header_response.dragged() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                ui.add_space(6.0);

                // --- 結果（メイン表示） ---
                ui.colored_label(
                    self.result_color,
                    egui::RichText::new(&self.translated)
                        .size(self.font_size),
                );

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // --- 原文（小さくグレーで） ---
                ui.colored_label(
                    original_color,
                    egui::RichText::new(&self.original)
                        .size(self.font_size * 0.75),
                );
            });
    }
}

fn estimate_window_size(text: &str, original: &str, font_size: f32) -> (f32, f32) {
    let all_text = format!("{}\n{}", text, original);
    let lines: Vec<&str> = all_text.lines().collect();
    let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(10);

    let char_width = font_size * 0.7;
    let width = (max_chars as f32 * char_width + 64.0)
        .clamp(350.0, 900.0);

    let translated_lines = text.lines().count().max(1);
    let original_lines = original.lines().count().max(1);
    let line_height = font_size * 1.5;
    let original_line_height = font_size * 0.75 * 1.5;
    let height = (translated_lines as f32 * line_height
        + original_lines as f32 * original_line_height
        + 100.0) // パディング + 区切り線 + ヒント + モードラベル
        .clamp(140.0, 700.0);

    (width, height)
}

/// 翻訳結果ポップアップを表示する
pub fn show_result_popup(result_file: &str, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = match acquire_popup_instance_lock("QuickToolsPopupSingleton") {
        Some(g) => g,
        None => return Ok(()),
    };

    let content = fs::read_to_string(result_file)?;
    let _ = fs::remove_file(result_file);

    let mut translated = String::new();
    let mut original = String::new();
    let mut is_original = false;

    for line in content.lines() {
        if line.trim() == "---" {
            is_original = true;
            continue;
        }
        if is_original {
            if !original.is_empty() {
                original.push('\n');
            }
            original.push_str(line);
        } else {
            if !translated.is_empty() {
                translated.push('\n');
            }
            translated.push_str(line);
        }
    }

    let (width, height) = estimate_window_size(&translated, &original, config.font_size);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width, height])
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false),
        ..Default::default()
    };

    let font_size = config.font_size;
    let result_color = egui::Color32::from_rgb(166, 227, 161); // 緑

    eframe::run_native(
        "Quick Tools Result",
        options,
        Box::new(move |cc| {
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(ResultPopup {
                translated,
                original,
                font_size,
                had_focus: false,
                created_at: Instant::now(),
                mode_label: "⚡ 翻訳結果".to_string(),
                result_color,
            }) as Box<dyn eframe::App>)
        }),
    )?;

    Ok(())
}

/// Markdown整形結果ポップアップを表示する
pub fn show_format_result_popup(result_file: &str, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = match acquire_popup_instance_lock("QuickToolsFormatPopupSingleton") {
        Some(g) => g,
        None => return Ok(()),
    };

    let content = fs::read_to_string(result_file)?;
    let _ = fs::remove_file(result_file);

    let mut formatted = String::new();
    let mut original = String::new();
    let mut is_original = false;

    for line in content.lines() {
        if line.trim() == "---" {
            is_original = true;
            continue;
        }
        if is_original {
            if !original.is_empty() {
                original.push('\n');
            }
            original.push_str(line);
        } else {
            if !formatted.is_empty() {
                formatted.push('\n');
            }
            formatted.push_str(line);
        }
    }

    let is_error = formatted.starts_with("エラー");

    let (width, height) = estimate_window_size(&formatted, &original, config.font_size);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width, height])
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false),
        ..Default::default()
    };

    let font_size = config.font_size;
    let result_color = if is_error {
        egui::Color32::from_rgb(243, 139, 168) // 赤（エラー）
    } else {
        egui::Color32::from_rgb(148, 226, 213) // シアン（Markdown整形）
    };

    eframe::run_native(
        "Quick Tools Format",
        options,
        Box::new(move |cc| {
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(ResultPopup {
                translated: formatted,
                original,
                font_size,
                had_focus: false,
                created_at: Instant::now(),
                mode_label: "📝 Markdown整形".to_string(),
                result_color,
            }) as Box<dyn eframe::App>)
        }),
    )?;

    Ok(())
}
