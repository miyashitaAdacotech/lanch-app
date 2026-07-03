// formatter.rs - Markdown整形（ハイブリッド方式）
//
// 選択テキストを Claude に送信し、Markdown形式に整形して返す。
//
// 認証の優先順位:
//   1. ANTHROPIC_API_KEY 環境変数 → HTTP API 直接呼び出し（高速: 2-3秒）
//   2. Claude Code CLI (`claude -p`) → Max Plan サブスクリプション枠（低速: 20-30秒）
//
// フロー:
//   1. ホットキー (Ctrl+Shift+F) で選択テキストをコピー
//   2. Claude に整形リクエストを送信
//   3. 整形結果をクリップボードにコピー

use crate::config::Config;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Claude CLI 呼び出しのタイムアウト（秒）。
/// Max Plan 経由の CLI は API 直接より遅いため、API 側（30秒）より長めに取る。
/// ハングを無制限にしない Fail-Fast の歯止め。
const CLI_TIMEOUT_SECS: u64 = 90;

// タイムアウト値の妥当性をコンパイル時に保証する（API の 30 秒以上、かつ現実的な上限）。
const _: () = assert!(CLI_TIMEOUT_SECS >= 30 && CLI_TIMEOUT_SECS <= 300);

/// 整形結果を格納する構造体
#[derive(Debug, Clone)]
pub struct FormatResult {
    /// 整形されたMarkdownテキスト
    pub formatted: String,
}

/// システムプロンプト（Markdown整形用）
const FORMAT_SYSTEM_PROMPT: &str = r#"あなたはテキスト整形の専門家です。
与えられたテキストをMarkdown形式に整形してください。

【最重要ルール】
- 元のテキストの文章・単語・表現を絶対に変更・追加・削除・言い換えしないこと
- 文章のリライト、要約、補足説明は禁止。構造とフォーマットだけを整える
- 説明や前置きは不要。整形結果のみを返す

【整形ルール】
- コードブロックがあれば適切な言語タグ付きのfenced code blockにする（言語の自動検出）
- テーブルデータ（TSV、CSV、スペース区切り、崩れた表）を検出してMarkdownテーブルに復元する
- 箇条書きや番号付きリストを適切に整形する
- 見出しレベルを適切に付与する
- 不要な空白や改行を整理する
- URLがあればMarkdownリンクとして整形する
- インラインコード（変数名、コマンド、ファイル名等）は`バッククォート`で囲む
- JSON、YAML、XML等の構造化データはコードブロックで整形する
- ログ出力やスタックトレースはコードブロック（text or log）で整形する"#;

/// 利用可能なバックエンドの種類
#[derive(Debug, Clone, PartialEq)]
pub enum Backend {
    /// Anthropic HTTP API 直接呼び出し（高速）
    Api,
    /// Claude Code CLI 経由（Max Plan 枠、低速）
    Cli,
    /// どちらも利用不可
    None,
}

/// 利用可能なバックエンドを判定する
pub fn detect_backend() -> Backend {
    // 1. API キーがあれば API 直接（高速）
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.trim().is_empty() {
            return Backend::Api;
        }
    }

    // 2. Claude CLI があれば CLI 経由（低速だが無料）
    if check_cli_available() {
        return Backend::Cli;
    }

    Backend::None
}

/// バックエンドの表示名を返す
pub fn backend_label(backend: &Backend) -> &'static str {
    match backend {
        Backend::Api => "API直接（高速）",
        Backend::Cli => "Claude CLI（Max Plan）",
        Backend::None => "利用不可",
    }
}

/// Claude Code CLI が利用可能か確認する
pub fn check_cli_available() -> bool {
    Command::new("claude")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// テキストをMarkdown整形する（バックエンドを自動選択）
///
/// ANTHROPIC_API_KEY が設定されていれば高速な API 直接呼び出し、
/// なければ Claude CLI フォールバック。
pub fn format_markdown(
    text: &str,
    config: &Config,
) -> Result<FormatResult, Box<dyn std::error::Error>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(FormatResult {
            formatted: String::new(),
        });
    }

    let backend = detect_backend();
    eprintln!("[format] バックエンド: {}", backend_label(&backend));

    let formatted = match backend {
        Backend::Api => call_api_direct(text, config)?,
        Backend::Cli => call_claude_cli(text, &config.claude_model)?,
        Backend::None => {
            return Err("Markdown整形を利用するには:\n\
                ① ANTHROPIC_API_KEY 環境変数を設定（高速）\n\
                ② または Claude Code をインストール（claude login）"
                .into());
        }
    };

    Ok(FormatResult { formatted })
}

// ============================================================
// バックエンド A: Anthropic HTTP API 直接呼び出し（高速）
// ============================================================

/// Anthropic Messages API を直接呼び出す
fn call_api_direct(text: &str, config: &Config) -> Result<String, Box<dyn std::error::Error>> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY が未設定です")?;

    // Markdown整形は単純タスク → Haiku で十分（高速＆安価）
    let model = if config.claude_model.is_empty() {
        "claude-haiku-4-5-20251001".to_string()
    } else {
        config.claude_model.clone()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": FORMAT_SYSTEM_PROMPT,
        "messages": [
            {
                "role": "user",
                "content": text
            }
        ]
    });

    let response = match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
    {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Err("API タイムアウト（30秒）".into());
            } else if e.is_connect() {
                return Err("API接続エラー。ネットワークを確認してください".into());
            } else {
                return Err(format!("API通信エラー: {}", e).into());
            }
        }
    };

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().unwrap_or_default();

        let user_msg = if error_body.contains("credit balance is too low") {
            "クレジット残高不足。console.anthropic.com でチャージしてください"
        } else {
            match status.as_u16() {
                400 => "リクエストエラー",
                401 => "APIキーが無効です",
                403 => "APIアクセス拒否",
                429 => "レート制限。しばらく待ってください",
                500..=599 => "APIサーバーエラー",
                _ => "APIエラー",
            }
        };
        eprintln!("[format] API エラー詳細 ({}): {}", status, error_body);
        return Err(format!("{} ({})", user_msg, status).into());
    }

    let json: serde_json::Value = response.json()?;

    let formatted = json
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(|t| t.as_str())
        .ok_or("API レスポンスのパースに失敗")?;

    Ok(formatted.to_string())
}

// ============================================================
// バックエンド B: Claude Code CLI 経由（Max Plan 枠）
// ============================================================

/// claude -p に渡す引数リストを組み立てる（純粋関数、テスト可能に分離）。
///
/// `--setting-sources project` で user グローバル設定（フック / MCP サーバー）を
/// ロードしないようにする。これと隔離した作業ディレクトリ（`call_claude_cli` 側で
/// `current_dir` に指定）を併用することで、アプリの CWD が Claude Code プロジェクトでも
/// CLAUDE.md / SessionStart フック / MCP 群を巻き込まず、整形タスクの乗っ取り・ハングを防ぐ。
/// OAuth 認証は設定ソースに依存しないため、Max Plan 枠のまま動作する。
fn build_cli_args(model_arg: &str) -> Vec<&str> {
    vec![
        "-p",
        "--model",
        model_arg,
        "--system-prompt",
        FORMAT_SYSTEM_PROMPT,
        "--setting-sources",
        "project",
        "--no-session-persistence",
    ]
}

/// CLI 整形の診断ログを追記するファイルパスを返す
fn diagnostic_log_path() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("lanch-app-fmt")
        .join("format-diagnostic.log")
}

/// CLI 整形の診断情報をログファイルに追記する。
///
/// アプリは GUI（windows subsystem）プロセスのため `eprintln!` の出力先が無く、
/// 失敗原因を追えない。失敗時に exit code / stderr / stdout をファイルへ残すことで、
/// 実機での再現時に真の原因を確認できるようにする（可観測性）。
fn append_diagnostic(entry: &str) {
    use std::io::Write as _;
    let path = diagnostic_log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{ts}] {entry}");
    }
}

/// CLI の stderr/stdout からユーザー向けエラーメッセージを分類する（純粋関数、テスト可能）
fn classify_cli_error(err_text: &str) -> &'static str {
    let lower = err_text.to_lowercase();
    if lower.contains("not logged in") || lower.contains("authentication") {
        "Claude Code にログインしてください: claude login"
    } else if lower.contains("retired")
        || lower.contains("may not exist")
        || lower.contains("issue with the selected model")
    {
        "モデルが無効です（引退済み等）。設定の claude_model を最新モデルに変更してください"
    } else if lower.contains("rate limit") || lower.contains("too many") {
        "レート制限。しばらく待ってから再試行してください"
    } else if lower.contains("credit balance") {
        "CLI経由でもクレジット不足。claude login で正しいアカウントにログインしてください"
    } else {
        "Claude CLI でエラーが発生しました"
    }
}

/// Claude Code CLI を呼び出してテキストを整形する
fn call_claude_cli(text: &str, model: &str) -> Result<String, Box<dyn std::error::Error>> {
    let model_arg = normalize_model_name(model);

    // claude -p を「プロジェクトを持たない専用ディレクトリ」で実行する。
    // アプリの作業ディレクトリが Claude Code プロジェクト（CLAUDE.md / .claude/settings.json /
    // SessionStart フック / MCP サーバー）だと、それらを毎回ロードして整形タスクを乗っ取り、
    // 会話応答を返したり数分ハングする。作業ディレクトリを隔離することで防ぐ。
    let isolated_dir = std::env::temp_dir().join("lanch-app-fmt");
    let _ = std::fs::create_dir_all(&isolated_dir);

    append_diagnostic(&format!(
        "CLI呼び出し開始 model={} cwd={} PATH_has_local_bin={}",
        model_arg,
        isolated_dir.display(),
        std::env::var("PATH").unwrap_or_default().contains(".local")
    ));

    let mut child = Command::new("claude")
        .args(build_cli_args(&model_arg))
        .current_dir(&isolated_dir)
        // ANTHROPIC_API_KEY が設定されていると Claude CLI が
        // Max Plan ではなく API キーを使ってしまうため、明示的に除外
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            append_diagnostic(&format!("spawn失敗 kind={:?} err={}", e.kind(), e));
            if e.kind() == std::io::ErrorKind::NotFound {
                "claude コマンドが見つかりません。Claude Code をインストールしてください"
                    .to_string()
            } else {
                format!("claude コマンドの起動に失敗: {}", e)
            }
        })?;

    // stdin にテキストを書き込んでクローズ
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }

    // stdout / stderr はパイプバッファが詰まると子プロセスがブロックしてデッドロックするため、
    // それぞれ別スレッドで最後まで読み切る。読み切りをスレッドに逃がすことで、本スレッドは
    // try_wait でデッドラインを監視し、タイムアウト時に kill できる。
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    // デッドライン監視（Fail-Fast: ハングを無制限にしない。API 経路のタイムアウトと対を成す）
    let deadline = Instant::now() + Duration::from_secs(CLI_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    append_diagnostic(&format!("タイムアウト {}秒でkill", CLI_TIMEOUT_SECS));
                    eprintln!(
                        "[format] CLI タイムアウト（{}秒）: プロセスを強制終了しました",
                        CLI_TIMEOUT_SECS
                    );
                    return Err(format!(
                        "Claude CLI タイムアウト（{}秒）。再試行してください",
                        CLI_TIMEOUT_SECS
                    )
                    .into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();

    if !status.success() {
        let stderr_s = String::from_utf8_lossy(&stderr);
        let stdout_s = String::from_utf8_lossy(&stdout);

        let err_text = format!("{}{}", stderr_s, stdout_s);
        let user_msg = classify_cli_error(&err_text);

        append_diagnostic(&format!(
            "非ゼロ終了 exit={} stderr=[{}] stdout=[{}]",
            status,
            stderr_s.trim(),
            stdout_s.trim().chars().take(500).collect::<String>()
        ));
        eprintln!("[format] CLI エラー (exit={})", status);
        eprintln!("[format]   stderr: {}", stderr_s.trim());
        eprintln!("[format]   stdout: {}", stdout_s.trim());
        return Err(user_msg.into());
    }

    let result = String::from_utf8(stdout)?;
    let trimmed = result.trim().to_string();

    if trimmed.is_empty() {
        append_diagnostic("空の応答（exit=0 だが stdout が空）");
        return Err("Claude CLI: 空の応答が返されました".into());
    }

    append_diagnostic(&format!("成功 len={}", trimmed.len()));
    Ok(trimmed)
}

/// モデル名を CLI 用に正規化する
fn normalize_model_name(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        "haiku".to_string()
    } else {
        m.to_string()
    }
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Backend enum のテスト ---

    #[test]
    fn test_backend_equality() {
        assert_eq!(Backend::Api, Backend::Api);
        assert_eq!(Backend::Cli, Backend::Cli);
        assert_eq!(Backend::None, Backend::None);
        assert_ne!(Backend::Api, Backend::Cli);
        assert_ne!(Backend::Api, Backend::None);
    }

    #[test]
    fn test_backend_clone() {
        let backend = Backend::Api;
        let cloned = backend.clone();
        assert_eq!(backend, cloned);
    }

    // --- backend_label のテスト ---

    #[test]
    fn test_backend_label_api() {
        assert_eq!(backend_label(&Backend::Api), "API直接（高速）");
    }

    #[test]
    fn test_backend_label_cli() {
        assert_eq!(backend_label(&Backend::Cli), "Claude CLI（Max Plan）");
    }

    #[test]
    fn test_backend_label_none() {
        assert_eq!(backend_label(&Backend::None), "利用不可");
    }

    // --- normalize_model_name のテスト ---

    #[test]
    fn test_normalize_model_name_empty() {
        assert_eq!(normalize_model_name(""), "haiku");
    }

    #[test]
    fn test_normalize_model_name_whitespace() {
        assert_eq!(normalize_model_name("   "), "haiku");
        assert_eq!(normalize_model_name("\t"), "haiku");
    }

    #[test]
    fn test_normalize_model_name_valid() {
        assert_eq!(
            normalize_model_name("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5-20251001"
        );
        assert_eq!(normalize_model_name("haiku"), "haiku");
    }

    #[test]
    fn test_normalize_model_name_trims() {
        assert_eq!(
            normalize_model_name("  claude-haiku-4-5-20251001  "),
            "claude-haiku-4-5-20251001"
        );
    }

    // --- FormatResult のテスト ---

    #[test]
    fn test_format_result_struct() {
        let result = FormatResult {
            formatted: "# Hello\n\nWorld".to_string(),
        };
        assert_eq!(result.formatted, "# Hello\n\nWorld");
    }

    #[test]
    fn test_format_result_clone() {
        let result = FormatResult {
            formatted: "test".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(result.formatted, cloned.formatted);
    }

    // --- format_markdown の空入力テスト ---

    #[test]
    fn test_format_markdown_empty_input() {
        let config = crate::config::Config::default();
        let result = format_markdown("", &config).unwrap();
        assert_eq!(result.formatted, "");
    }

    #[test]
    fn test_format_markdown_whitespace_input() {
        let config = crate::config::Config::default();
        let result = format_markdown("   \n\t  ", &config).unwrap();
        assert_eq!(result.formatted, "");
    }

    // --- detect_backend のテスト ---
    // 注: 環境変数に依存するため、実行環境で結果が変わる

    #[test]
    fn test_detect_backend_returns_valid_variant() {
        let backend = detect_backend();
        // 何らかの Backend が返ることを確認（環境依存）
        match backend {
            Backend::Api | Backend::Cli | Backend::None => {} // OK
        }
    }

    // --- システムプロンプトのテスト ---

    #[test]
    fn test_system_prompt_not_empty() {
        assert!(!FORMAT_SYSTEM_PROMPT.is_empty());
    }

    #[test]
    fn test_system_prompt_contains_key_instructions() {
        assert!(FORMAT_SYSTEM_PROMPT.contains("Markdown"));
        assert!(FORMAT_SYSTEM_PROMPT.contains("コードブロック"));
        assert!(FORMAT_SYSTEM_PROMPT.contains("テーブル"));
    }

    // --- build_cli_args のテスト（プロジェクト文脈の混入・ハング防止の回帰ガード） ---

    #[test]
    fn test_build_cli_args_isolates_from_project_context() {
        let args = build_cli_args("haiku");
        // user グローバル設定（フック / MCP）をロードしない指定が必ず含まれること。
        // これが欠けると CWD がプロジェクトのとき CLAUDE.md / SessionStart フック /
        // MCP 群を巻き込み、整形タスクの乗っ取り・数分ハングが再発する。
        let pos = args
            .iter()
            .position(|a| *a == "--setting-sources")
            .expect("--setting-sources フラグが必要");
        assert_eq!(args.get(pos + 1), Some(&"project"));
    }

    // --- classify_cli_error のテスト（引退モデル等のエラー分類の回帰ガード） ---

    #[test]
    fn test_classify_cli_error_retired_model() {
        // 実際に発生した引退モデルの CLI 出力（claude-sonnet-4-20250514）
        let err = "⚠ Claude Sonnet 4 was retired on June 15, 2026. \
            There's an issue with the selected model (claude-sonnet-4-20250514). \
            It may not exist or you may not have access to it.";
        assert_eq!(
            classify_cli_error(err),
            "モデルが無効です（引退済み等）。設定の claude_model を最新モデルに変更してください"
        );
    }

    #[test]
    fn test_classify_cli_error_not_logged_in() {
        assert_eq!(
            classify_cli_error("Not logged in · Please run /login"),
            "Claude Code にログインしてください: claude login"
        );
    }

    #[test]
    fn test_classify_cli_error_rate_limit() {
        assert_eq!(
            classify_cli_error("Error: rate limit exceeded"),
            "レート制限。しばらく待ってから再試行してください"
        );
    }

    #[test]
    fn test_classify_cli_error_generic_fallback() {
        assert_eq!(
            classify_cli_error("some unknown failure"),
            "Claude CLI でエラーが発生しました"
        );
    }

    #[test]
    fn test_build_cli_args_contains_required_flags() {
        let args = build_cli_args("claude-haiku-4-5-20251001");
        assert!(args.contains(&"-p"));
        assert!(args.contains(&"--no-session-persistence"));
        // 渡したモデル名が引数列に含まれること
        let pos = args
            .iter()
            .position(|a| *a == "--model")
            .expect("--model フラグが必要");
        assert_eq!(args.get(pos + 1), Some(&"claude-haiku-4-5-20251001"));
        // システムプロンプトが引数列に含まれること
        assert!(args.contains(&FORMAT_SYSTEM_PROMPT));
    }
}
