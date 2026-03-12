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
mod config;
mod formatter;
mod lang;
mod notification;
mod popup;
mod translator;
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();
    let mut config = config::load_config();

    if let Some(engine) = args.engine {
        config.engine = engine;
    }

    // --- モード分岐 ---

    if let Some(text) = args.translate_text {
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
