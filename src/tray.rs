// tray.rs - システムトレイ常駐 & グローバルホットキー
//
// アーキテクチャ:
// - メインプロセス: トレイアイコン + ホットキーリスナー（Windowsメッセージループ）
// - ポップアップ: 別プロセスとして起動（eframeがメインスレッドを占有するため）
//
// ホットキー:
//   Ctrl+Shift+T → 入力ポップアップ（自分で文字を打つ）
//   Ctrl+Shift+Y → 選択テキスト翻訳（Ctrl+Cしてから翻訳→結果表示）
//   Ctrl+Shift+F → 選択テキストMarkdown整形（Ctrl+Cしてから整形→クリップボード）

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder, TrayIconEvent, Icon,
};

use std::env;
use std::fs;
use std::process::Command;
use std::thread;

use crate::clipboard;
use crate::clipboard_history;
use crate::config::Config;
use crate::formatter;
use crate::notification;

#[cfg(windows)]
struct TrayInstanceGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for TrayInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn acquire_tray_instance_lock(name: &str) -> Option<TrayInstanceGuard> {
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

        Some(TrayInstanceGuard { handle })
    }
}

#[cfg(not(windows))]
fn acquire_tray_instance_lock(_name: &str) -> Option<()> {
    Some(())
}

/// トレイアイコン用の16x16 "Q" アイコンをRGBAデータから作成する
/// ("Q" = Quick Tools)
fn create_icon() -> Icon {
    let size = 16;
    let mut rgba = vec![0u8; size * size * 4];

    // "Q" の外枠を水色で描画 (#6495ED = RGB(100, 149, 237))
    let color = [100u8, 149, 237, 255];

    // 上辺 (y=2, x=4..12)
    for x in 4..12 {
        let i = (2 * size + x) * 4;
        rgba[i..i + 4].copy_from_slice(&color);
    }
    // 下辺 (y=12, x=4..12)
    for x in 4..12 {
        let i = (12 * size + x) * 4;
        rgba[i..i + 4].copy_from_slice(&color);
    }
    // 左辺 (y=3..12, x=3)
    for y in 3..12 {
        let i = (y * size + 3) * 4;
        rgba[i..i + 4].copy_from_slice(&color);
    }
    // 右辺 (y=3..12, x=12)
    for y in 3..12 {
        let i = (y * size + 12) * 4;
        rgba[i..i + 4].copy_from_slice(&color);
    }
    // 斜め線 (Q のしっぽ)
    for d in 0..4 {
        let x = 9 + d;
        let y = 10 + d;
        if x < size && y < size {
            let i = (y * size + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }

    Icon::from_rgba(rgba, size as u32, size as u32)
        .expect("アイコンの作成に失敗")
}

/// 選択テキスト翻訳を実行する
fn handle_selected_translation() {
    let text = match clipboard::copy_selected_text() {
        Some(t) => t,
        None => {
            eprintln!("選択テキストの取得に失敗");
            return;
        }
    };

    if text.is_empty() {
        return;
    }

    let request_path = env::temp_dir().join("quick_translate_popup_request.txt");

    if let Err(e) = fs::write(&request_path, text) {
        eprintln!("リクエストファイルの作成に失敗: {}", e);
        return;
    }

    spawn_self(&["--popup"]);
}

/// 選択テキストをMarkdown整形する（サイレントモード）
///
/// 1. Ctrl+C シミュレーション → クリップボードから読み取り
/// 2. Claude API に送信してMarkdown整形
/// 3. 結果をクリップボードにコピー
/// 4. トースト通知のみ表示（ポップアップなし）
fn handle_markdown_format(config: &Config) {
    let text = match clipboard::copy_selected_text() {
        Some(t) => t,
        None => {
            eprintln!("選択テキストの取得に失敗");
            notification::show_error("Lanch App", "選択テキストの取得に失敗しました");
            return;
        }
    };

    if text.is_empty() {
        return;
    }

    // Claude API 呼び出しはブロッキングなので別スレッドで実行
    let config = config.clone();
    thread::spawn(move || {
        eprintln!("[format] Markdown整形を開始...");

        match formatter::format_markdown(&text, &config) {
            Ok(result) => {
                if result.formatted.is_empty() {
                    eprintln!("[format] 整形結果が空でした");
                    return;
                }

                // クリップボードに整形結果をコピー
                match arboard::Clipboard::new() {
                    Ok(mut cb) => {
                        if let Err(e) = cb.set_text(&result.formatted) {
                            eprintln!("[format] クリップボードへのコピーに失敗: {}", e);
                            notification::show_error("Lanch App", "クリップボードへのコピーに失敗しました");
                            return;
                        }
                        eprintln!("[format] Markdown整形完了 → クリップボードにコピーしました");
                        // サイレントモード: トースト通知のみ
                        notification::show("Lanch App", "Markdown整形完了 → クリップボードにコピーしました");
                    }
                    Err(e) => {
                        eprintln!("[format] クリップボードのオープンに失敗: {}", e);
                        notification::show_error("Lanch App", "クリップボードのオープンに失敗しました");
                    }
                }
            }
            Err(e) => {
                let msg = format!("Markdown整形に失敗: {}", e);
                eprintln!("[format] {}", msg);
                notification::show_error("Lanch App", &msg);
            }
        }
    });
}

/// 自分自身を別プロセスとして起動する
fn spawn_self(args: &[&str]) {
    match env::current_exe() {
        Ok(exe) => {
            let _ = Command::new(exe).args(args).spawn();
        }
        Err(e) => eprintln!("自分自身のパス取得に失敗: {}", e),
    }
}

fn parse_hotkey(spec: &str) -> Option<HotKey> {
    let normalized = spec.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::empty();
    let mut key_code: Option<Code> = None;

    for part in normalized.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            "a" => key_code = Some(Code::KeyA),
            "b" => key_code = Some(Code::KeyB),
            "c" => key_code = Some(Code::KeyC),
            "d" => key_code = Some(Code::KeyD),
            "e" => key_code = Some(Code::KeyE),
            "f" => key_code = Some(Code::KeyF),
            "g" => key_code = Some(Code::KeyG),
            "h" => key_code = Some(Code::KeyH),
            "i" => key_code = Some(Code::KeyI),
            "j" => key_code = Some(Code::KeyJ),
            "k" => key_code = Some(Code::KeyK),
            "l" => key_code = Some(Code::KeyL),
            "m" => key_code = Some(Code::KeyM),
            "n" => key_code = Some(Code::KeyN),
            "o" => key_code = Some(Code::KeyO),
            "p" => key_code = Some(Code::KeyP),
            "q" => key_code = Some(Code::KeyQ),
            "r" => key_code = Some(Code::KeyR),
            "s" => key_code = Some(Code::KeyS),
            "t" => key_code = Some(Code::KeyT),
            "u" => key_code = Some(Code::KeyU),
            "v" => key_code = Some(Code::KeyV),
            "w" => key_code = Some(Code::KeyW),
            "x" => key_code = Some(Code::KeyX),
            "y" => key_code = Some(Code::KeyY),
            "z" => key_code = Some(Code::KeyZ),
            _ => return None,
        }
    }

    key_code.map(|code| {
        if modifiers.is_empty() {
            HotKey::new(None, code)
        } else {
            HotKey::new(Some(modifiers), code)
        }
    })
}

/// システムトレイアプリのメインループを実行する
pub fn run_tray(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = match acquire_tray_instance_lock("QuickToolsTraySingleton") {
        Some(g) => g,
        None => {
            println!("Quick Tools は既に起動しています");
            return Ok(());
        }
    };

    // --- クリップボード履歴監視を開始 ---
    let _clipboard_store = clipboard_history::start_monitoring();

    // --- メニューの作成 ---
    let menu = Menu::new();
    let popup_label = format!("翻訳ポップアップ ({})", config.hotkey_popup);
    let selected_label = format!("選択テキスト翻訳 ({})", config.hotkey_selected);
    let format_label = format!("Markdown整形 ({})", config.hotkey_format);
    let history_label = format!("クリップボード履歴 ({})", config.hotkey_clipboard_history);

    let item_popup = MenuItem::new(&popup_label, true, None);
    let item_selected = MenuItem::new(&selected_label, true, None);
    let item_format = MenuItem::new(&format_label, true, None);
    let item_history = MenuItem::new(&history_label, true, None);
    let item_quit = MenuItem::new("終了", true, None);

    menu.append(&item_popup)?;
    menu.append(&item_selected)?;
    menu.append(&item_format)?;
    menu.append(&item_history)?;
    menu.append(&item_quit)?;

    let item_popup_id = item_popup.id().clone();
    let item_selected_id = item_selected.id().clone();
    let item_format_id = item_format.id().clone();
    let item_history_id = item_history.id().clone();
    let item_quit_id = item_quit.id().clone();

    // --- トレイアイコンの作成 ---
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Quick Tools - 翻訳 & Markdown整形")
        .with_icon(create_icon())
        .build()?;

    // --- グローバルホットキーの登録 ---
    let hotkey_manager = GlobalHotKeyManager::new()?;

    let default_popup = "ctrl+shift+t".to_string();
    let default_selected = "ctrl+shift+y".to_string();
    let default_format = "ctrl+shift+f".to_string();
    let default_history = "ctrl+shift+v".to_string();

    let popup_spec = if config.hotkey_popup.trim().is_empty() {
        default_popup.as_str()
    } else {
        config.hotkey_popup.as_str()
    };
    let selected_spec = if config.hotkey_selected.trim().is_empty() {
        default_selected.as_str()
    } else {
        config.hotkey_selected.as_str()
    };
    let format_spec = if config.hotkey_format.trim().is_empty() {
        default_format.as_str()
    } else {
        config.hotkey_format.as_str()
    };
    let history_spec = if config.hotkey_clipboard_history.trim().is_empty() {
        default_history.as_str()
    } else {
        config.hotkey_clipboard_history.as_str()
    };

    let hk_popup = parse_hotkey(popup_spec)
        .or_else(|| parse_hotkey(&default_popup))
        .ok_or_else(|| format!("ポップアップホットキーの形式が不正です: {}", popup_spec))?;
    let hk_selected = parse_hotkey(selected_spec)
        .or_else(|| parse_hotkey(&default_selected))
        .ok_or_else(|| format!("選択翻訳ホットキーの形式が不正です: {}", selected_spec))?;
    let hk_format = parse_hotkey(format_spec)
        .or_else(|| parse_hotkey(&default_format))
        .ok_or_else(|| format!("Markdown整形ホットキーの形式が不正です: {}", format_spec))?;
    let hk_history = parse_hotkey(history_spec)
        .or_else(|| parse_hotkey(&default_history))
        .ok_or_else(|| format!("クリップボード履歴ホットキーの形式が不正です: {}", history_spec))?;

    // ホットキー登録（競合時はスキップして警告）
    let popup_registered = match hotkey_manager.register(hk_popup) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("  ⚠ {} の登録に失敗（他アプリと競合の可能性）: {}", popup_spec, e);
            false
        }
    };
    let selected_registered = match hotkey_manager.register(hk_selected) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("  ⚠ {} の登録に失敗（他アプリと競合の可能性）: {}", selected_spec, e);
            false
        }
    };
    let format_registered = match hotkey_manager.register(hk_format) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("  ⚠ {} の登録に失敗（他アプリと競合の可能性）: {}", format_spec, e);
            false
        }
    };
    let history_registered = match hotkey_manager.register(hk_history) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("  ⚠ {} の登録に失敗（他アプリと競合の可能性）: {}", history_spec, e);
            false
        }
    };

    if !popup_registered && !selected_registered && !format_registered && !history_registered {
        return Err("全てのホットキーの登録に失敗しました。他のアプリ（quick_translate.ahk 等）を終了してから再起動してください".into());
    }

    println!("Lanch App がシステムトレイで起動しました");
    if popup_registered {
        println!("  {}: 翻訳ポップアップを開く", popup_spec);
    }
    if selected_registered {
        println!("  {}: 選択テキストを翻訳", selected_spec);
    }
    if format_registered {
        println!("  {}: 選択テキストをMarkdown整形", format_spec);
    }
    if history_registered {
        println!("  {}: クリップボード履歴を開く", history_spec);
    }
    println!("  📋 クリップボード監視: 有効（最大7日間保持）");

    // Markdown整形バックエンドの検出
    let backend = formatter::detect_backend();
    match &backend {
        formatter::Backend::Api => {
            println!("  Markdown整形: API直接モード ✓（高速: 2-3秒）");
        }
        formatter::Backend::Cli => {
            println!("  Markdown整形: Claude CLI モード ✓（Max Plan 枠使用）");
            println!("    ※ ANTHROPIC_API_KEY を設定すると高速モード（2-3秒）に切替可能");
        }
        formatter::Backend::None => {
            println!();
            println!("  ============================================");
            println!("  ⚠ Markdown整形が利用できません！");
            println!("  以下のいずれかを設定してください:");
            println!();
            println!("  【高速】ANTHROPIC_API_KEY 環境変数を設定");
            println!("  【無料】Claude Code をインストール:");
            println!("    npm install -g @anthropic-ai/claude-code");
            println!("    claude login");
            println!("  ============================================");
            notification::show_error("Lanch App", "Markdown整形が利用できません。設定方法はコンソールを確認してください。");
        }
    }

    // ホットキー連打防止用タイムスタンプ
    let mut last_popup_hotkey_time =
        std::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut last_selected_hotkey_time =
        std::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut last_format_hotkey_time =
        std::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut last_history_hotkey_time =
        std::time::Instant::now() - std::time::Duration::from_secs(10);

    // --- Windows メッセージループ ---
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG,
        };

        loop {
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret <= 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // --- ホットキーイベントの処理 ---
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.id == hk_popup.id() {
                    if last_popup_hotkey_time.elapsed().as_millis() < 300 {
                        continue;
                    }
                    last_popup_hotkey_time = std::time::Instant::now();
                    spawn_self(&["--popup"]);
                } else if event.id == hk_selected.id() {
                    if last_selected_hotkey_time.elapsed().as_millis() < 1500 {
                        continue;
                    }
                    last_selected_hotkey_time = std::time::Instant::now();
                    handle_selected_translation();
                } else if event.id == hk_format.id() {
                    if last_format_hotkey_time.elapsed().as_millis() < 2000 {
                        continue;
                    }
                    last_format_hotkey_time = std::time::Instant::now();
                    handle_markdown_format(config);
                } else if event.id == hk_history.id() {
                    if last_history_hotkey_time.elapsed().as_millis() < 500 {
                        continue;
                    }
                    last_history_hotkey_time = std::time::Instant::now();
                    // 別プロセスとして履歴UIを起動（EventLoop再作成エラー回避）
                    spawn_self(&["--clipboard-history"]);
                }
            }

            // --- トレイメニューイベントの処理 ---
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == item_popup_id {
                    spawn_self(&["--popup"]);
                } else if event.id == item_selected_id {
                    handle_selected_translation();
                } else if event.id == item_format_id {
                    handle_markdown_format(config);
                } else if event.id == item_history_id {
                    spawn_self(&["--clipboard-history"]);
                } else if event.id == item_quit_id {
                    return Ok(());
                }
            }

            while TrayIconEvent::receiver().try_recv().is_ok() {}
        }
    }

    #[cfg(not(windows))]
    {
        eprintln!("トレイモードは Windows でのみ動作します");
    }

    Ok(())
}
