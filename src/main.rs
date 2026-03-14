// main.rs - Lanch App エントリポイント
//
// quick-translate + Markdown整形を統合した統一ランチャー。
//
// 動作モード:
//   lanch-app                          → トレイ常駐モード（デフォルト）
//   lanch-app --popup                  → 翻訳入力ポップアップ
//   lanch-app --popup-file <path>      → ファイルからテキストを読んでポップアップ
//   lanch-app --translate "text"       → CLIで翻訳して標準出力
//   lanch-app --format "text"          → CLIでMarkdown整形して標準出力
//   lanch-app --result-file <path>     → 翻訳結果をポップアップ表示（内部用）
//   lanch-app --format-result <path>   → 整形結果をポップアップ表示（内部用）

// モジュール宣言
mod clipboard;
mod clipboard_history;
mod clipboard_store;
mod clipboard_ui;
mod config;
mod formatter;
mod lang;
mod notification;
mod popup;
mod translator;
mod spinner;
mod tray;

use std::env;
use std::fs;

/// コマンドライン引数
struct CliArgs {
    /// --translate "text": 翻訳するテキスト
    translate_text: Option<String>,
    /// --format "text": Markdown整形するテキスト
    format_text: Option<String>,
    /// --popup: ポップアップ表示
    show_popup: bool,
    /// --popup-file <path>: ファイルからテキストを読んでポップアップ表示
    popup_file: Option<String>,
    /// --result-file <path>: 翻訳結果の表示（内部用）
    result_file: Option<String>,
    /// --format-result <path>: 整形結果の表示（内部用）
    format_result_file: Option<String>,
    /// --engine <name>: エンジン指定
    engine: Option<String>,
    /// --no-tray: トレイモードを使わず直接ポップアップ
    no_tray: bool,
    /// --clipboard-history: クリップボード履歴ポップアップ（内部用: 別プロセスで起動）
    clipboard_history: bool,
}

impl CliArgs {
    fn parse() -> Self {
        let args: Vec<String> = env::args().skip(1).collect();
        let mut cli = CliArgs {
            translate_text: None,
            format_text: None,
            show_popup: false,
            popup_file: None,
            result_file: None,
            format_result_file: None,
            engine: None,
            no_tray: false,
            clipboard_history: false,
        };

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--translate" | "-t" => {
                    if i + 1 < args.len() {
                        cli.translate_text = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--format" | "-f" => {
                    if i + 1 < args.len() {
                        cli.format_text = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--popup" | "-p" => {
                    cli.show_popup = true;
                }
                "--popup-file" => {
                    if i + 1 < args.len() {
                        cli.popup_file = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--result-file" => {
                    if i + 1 < args.len() {
                        cli.result_file = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--format-result" => {
                    if i + 1 < args.len() {
                        cli.format_result_file = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--engine" | "-e" => {
                    if i + 1 < args.len() {
                        cli.engine = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--no-tray" => {
                    cli.no_tray = true;
                }
                "--clipboard-history" => {
                    cli.clipboard_history = true;
                }
                "--help" | "-h" => {
                    println!("Lanch App - 統一ランチャー (翻訳 + Markdown整形)");
                    println!();
                    println!("使い方:");
                    println!("  lanch-app                            トレイ常駐モード");
                    println!("  lanch-app --popup                    翻訳ポップアップを表示");
                    println!("  lanch-app --translate \"text\"         テキストを翻訳");
                    println!("  lanch-app --format \"text\"            テキストをMarkdown整形");
                    println!("  lanch-app --popup-file <path>        ファイルから読んで翻訳");
                    println!("  lanch-app --no-tray                  トレイなしでポップアップ直接起動");
                    println!("  lanch-app --engine google|deepl      翻訳エンジンを指定");
                    println!();
                    println!("トレイ常駐時のショートカット:");
                    println!("  Ctrl+Shift+T  翻訳ポップアップを開く");
                    println!("  Ctrl+Shift+Y  選択テキストを翻訳");
                    println!("  Ctrl+Shift+F  選択テキストをMarkdown整形（サイレント）");
                    println!();
                    println!("設定ファイル: ~/.lanch-app/config.json");
                    println!();
                    println!("Markdown整形（ハイブリッド方式）:");
                    println!("  優先1: 環境変数 ANTHROPIC_API_KEY を設定 → API直接（高速 2-3秒）");
                    println!("  優先2: Claude Code CLI (claude login) → Max Plan枠（低速 20-30秒）");
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("不明な引数: {}", args[i]);
                }
            }
            i += 1;
        }
        cli
    }
}

/// ログファイルの初期化
///
/// ~/.lanch-app/lanch-app.log に eprintln! の出力をリダイレクトする。
/// ローテーション: 2日以上前のログは自動削除。サイズが1MB超でも即ローテーション。
fn init_logging() {
    let log_dir = dirs::home_dir()
        .map(|h| h.join(".lanch-app"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("lanch-app.log");
    let old_path = log_dir.join("lanch-app.log.old");

    // 2日以上前の .log.old は削除
    if let Ok(meta) = fs::metadata(&old_path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(elapsed) = modified.elapsed() {
                if elapsed.as_secs() > 2 * 24 * 60 * 60 {
                    let _ = fs::remove_file(&old_path);
                }
            }
        }
    }

    // 現行ログが2日以上前 or 1MB超 → ローテーション（古いoldは上で消済み）
    let should_rotate = if let Ok(meta) = fs::metadata(&log_path) {
        let too_old = meta.modified().ok().and_then(|m| m.elapsed().ok())
            .map(|e| e.as_secs() > 2 * 24 * 60 * 60)
            .unwrap_or(false);
        too_old || meta.len() > 1_000_000
    } else {
        false
    };

    if should_rotate {
        let _ = fs::rename(&log_path, &old_path);
    }

    // ログファイルを追記モードで開いてstderrをリダイレクト
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        use std::fs::OpenOptions;

        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let handle = file.into_raw_handle();
            unsafe {
                use windows_sys::Win32::System::Threading::GetCurrentProcess;
                use windows_sys::Win32::Foundation::HANDLE;

                // SetStdHandle で stderr をログファイルに差し替え
                // STD_ERROR_HANDLE = -12i32 as u32
                const STD_ERROR_HANDLE: u32 = (-12i32) as u32;
                windows_sys::Win32::System::Console::SetStdHandle(
                    STD_ERROR_HANDLE,
                    handle as HANDLE,
                );
                let _ = GetCurrentProcess(); // suppress unused warning
            }
        }
    }

    // 起動ログ
    let now = chrono::Local::now();
    let args: Vec<String> = env::args().collect();
    eprintln!("\n=== lanch-app started at {} ===", now.format("%Y-%m-%d %H:%M:%S"));
    eprintln!("  args: {:?}", args);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = CliArgs::parse();
    let mut config = config::load_config();

    if let Some(engine) = args.engine {
        config.engine = engine;
    }

    // --- モード分岐 ---

    if args.clipboard_history {
        // クリップボード履歴ポップアップ（別プロセスで起動される）
        use std::sync::{Arc, Mutex};
        let store = clipboard_store::ClipboardStore::new(7);
        let shared: clipboard_history::SharedStore = Arc::new(Mutex::new(store));
        clipboard_ui::show_clipboard_history(shared)?;
    } else if let Some(text) = args.translate_text {
        // CLI翻訳モード
        let result = translator::translate(&text, &config)?;
        println!("{}", result.translated);
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(&result.translated);
        }
    } else if let Some(text) = args.format_text {
        // CLI Markdown整形モード
        let result = formatter::format_markdown(&text, &config)?;
        println!("{}", result.formatted);
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(&result.formatted);
        }
    } else if let Some(result_file) = args.result_file {
        // 翻訳結果表示モード（内部用: トレイから呼ばれる）
        popup::show_result_popup(&result_file, config)?;
    } else if let Some(result_file) = args.format_result_file {
        // 整形結果表示モード（内部用: トレイから呼ばれる）
        popup::show_format_result_popup(&result_file, config)?;
    } else if let Some(file_path) = args.popup_file {
        // ファイルからテキスト読み込みポップアップ
        let initial_text = match fs::read_to_string(&file_path) {
            Ok(text) => {
                let _ = fs::remove_file(&file_path);
                text.trim().to_string()
            }
            Err(_) => String::new(),
        };
        popup::show_popup(config, initial_text)?;
    } else if args.show_popup || args.no_tray {
        // 入力ポップアップモード
        popup::show_popup(config, String::new())?;
    } else {
        // デフォルト: トレイ常駐モード
        tray::run_tray(&config)?;
    }

    Ok(())
}
