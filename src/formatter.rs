// formatter.rs - Claude Code CLI によるMarkdown整形
//
// 選択テキストを Claude Code CLI (`claude -p`) に送信し、
// Markdown形式に整形して返す。
//
// Max Plan のサブスクリプション枠を使用するため、
// 別途 API キーやクレジット購入は不要。
//
// フロー:
//   1. ホットキー (Ctrl+Shift+F) で選択テキストをコピー
//   2. Claude CLI にパイプで整形リクエストを送信
//   3. 整形結果をクリップボードにコピー

use crate::config::Config;
use std::io::Write;
use std::process::{Command, Stdio};

/// 整形結果を格納する構造体
#[derive(Debug, Clone)]
pub struct FormatResult {
    /// 整形されたMarkdownテキスト
    pub formatted: String,
}

/// システムプロンプト（Markdown整形用）
const FORMAT_SYSTEM_PROMPT: &str = r#"あなたはテキスト整形の専門家です。
与えられたテキストをMarkdown形式に整形してください。

ルール:
- コードブロックがあれば適切な言語タグ付きのfenced code blockにする（言語の自動検出）
- テーブルデータ（TSV、CSV、スペース区切り、崩れた表）を検出してMarkdownテーブルに復元する
- 箇条書きや番号付きリストを適切に整形する
- 見出しレベルを適切に付与する
- 不要な空白や改行を整理する
- 元のテキストの内容は変更しない（整形のみ）
- 説明や前置きは不要。整形結果のみを返す
- URLがあればMarkdownリンクとして整形する
- インラインコード（変数名、コマンド、ファイル名等）は`バッククォート`で囲む
- JSON、YAML、XML等の構造化データはコードブロックで整形する
- ログ出力やスタックトレースはコードブロック（text or log）で整形する"#;

/// Claude Code CLI を使ってテキストをMarkdown整形する
///
/// `claude -p --model <model> --system-prompt <prompt>` で呼び出し、
/// stdin にテキストを流して stdout から結果を受け取る。
///
/// # 引数
/// - `text`: 整形するテキスト
/// - `config`: アプリケーション設定（モデル名等）
///
/// # 戻り値
/// - `Ok(FormatResult)`: 整形結果
/// - `Err(...)`: CLI実行エラー
pub fn format_markdown(text: &str, config: &Config) -> Result<FormatResult, Box<dyn std::error::Error>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(FormatResult {
            formatted: String::new(),
        });
    }

    let formatted = call_claude_cli(text, &config.claude_model)?;

    Ok(FormatResult { formatted })
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

/// Claude Code CLI を呼び出してテキストを整形する
///
/// コマンド:
///   echo <text> | claude -p --model <model> --system-prompt <prompt>
///
/// - `-p` (--print): 非対話モード。結果を stdout に出力して終了
/// - `--model`: 使用するモデル（sonnet 等）
/// - `--system-prompt`: システムプロンプト
/// - `--allowedTools`: ツール不要なので空にしてCLI操作を防ぐ
fn call_claude_cli(
    text: &str,
    model: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // モデル名をCLI用に変換（config の完全名 → CLI のエイリアス対応）
    let model_arg = normalize_model_name(model);

    let mut child = Command::new("claude")
        .args([
            "-p",
            "--model", &model_arg,
            "--system-prompt", FORMAT_SYSTEM_PROMPT,
            "--no-session-persistence",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("claude コマンドが見つかりません。Claude Code をインストールしてください")
            } else {
                format!("claude コマンドの起動に失敗: {}", e)
            }
        })?;

    // stdin にテキストを書き込んでクローズ
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
        // stdin をドロップして EOF を送信
    }

    // 結果を待つ
    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // よくあるエラーの分類
        let err_text = format!("{}{}", stderr, stdout);
        let user_msg = if err_text.contains("not logged in") || err_text.contains("authentication") {
            "Claude Code にログインしてください: claude login"
        } else if err_text.contains("rate limit") || err_text.contains("too many") {
            "レート制限に達しました。しばらく待ってから再試行してください"
        } else if err_text.contains("model") && err_text.contains("not found") {
            "指定されたモデルが見つかりません。config.json の claude_model を確認してください"
        } else {
            "Claude CLI でエラーが発生しました"
        };

        eprintln!("[format] Claude CLI エラー (exit={})", output.status);
        eprintln!("[format]   stderr: {}", stderr.trim());
        eprintln!("[format]   stdout: {}", stdout.trim());
        return Err(format!("{}", user_msg).into());
    }

    let result = String::from_utf8(output.stdout)?;
    let trimmed = result.trim().to_string();

    if trimmed.is_empty() {
        return Err("Claude CLI: 空の応答が返されました".into());
    }

    Ok(trimmed)
}

/// モデル名を CLI 用に正規化する
///
/// config.json の `claude_model` は API 用の完全名（例: "claude-sonnet-4-20250514"）
/// だが、Claude CLI は短縮名も受け付ける（例: "sonnet"）
/// 完全名もそのまま使えるので、基本はパススルー
fn normalize_model_name(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        // デフォルト: sonnet（コスパ最良）
        "sonnet".to_string()
    } else {
        m.to_string()
    }
}
