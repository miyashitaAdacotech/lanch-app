// formatter.rs - Claude API によるMarkdown整形
//
// 選択テキストを Anthropic Claude API に送信し、
// Markdown形式に整形して返す。
//
// フロー:
//   1. ホットキー (Ctrl+Shift+F) で選択テキストをコピー
//   2. Claude API に整形リクエストを送信
//   3. 整形結果をクリップボードにコピー

use crate::config::Config;

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

/// Claude API キーを取得する
///
/// 優先順位:
/// 1. 環境変数 `ANTHROPIC_API_KEY`
/// 2. config.json の `claude_api_key`
fn resolve_api_key(config: &Config) -> Result<String, Box<dyn std::error::Error>> {
    // 環境変数を最優先
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(key.trim().to_string());
        }
    }

    // config.json のフォールバック
    if !config.claude_api_key.is_empty() {
        return Ok(config.claude_api_key.clone());
    }

    Err("APIキー未設定。ANTHROPIC_API_KEY 環境変数を設定してください".into())
}

/// Claude API を使ってテキストをMarkdown整形する
///
/// # 引数
/// - `text`: 整形するテキスト
/// - `config`: アプリケーション設定（APIキー等）
///
/// # 戻り値
/// - `Ok(FormatResult)`: 整形結果
/// - `Err(...)`: APIエラー
pub fn format_markdown(text: &str, config: &Config) -> Result<FormatResult, Box<dyn std::error::Error>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(FormatResult {
            formatted: String::new(),
        });
    }

    let api_key = resolve_api_key(config)?;
    let formatted = call_claude_api(text, &api_key, &config.claude_model)?;

    Ok(FormatResult { formatted })
}

/// Anthropic Messages API を呼び出す
///
/// エンドポイント: https://api.anthropic.com/v1/messages
///
/// リクエスト形式:
/// {
///   "model": "claude-sonnet-4-20250514",
///   "max_tokens": 4096,
///   "system": "...",
///   "messages": [{"role": "user", "content": "..."}]
/// }
fn call_claude_api(
    text: &str,
    api_key: &str,
    model: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // リクエストボディを構築
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
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
    {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Err("API タイムアウト（30秒）。テキストが長すぎる可能性があります".into());
            } else if e.is_connect() {
                return Err("API接続エラー。ネットワーク接続を確認してください".into());
            } else {
                return Err(format!("API通信エラー: {}", e).into());
            }
        }
    };

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().unwrap_or_default();
        // ステータスコード別のわかりやすいメッセージ
        let user_msg = match status.as_u16() {
            401 => "APIキーが無効です。正しいキーを設定してください",
            403 => "APIアクセスが拒否されました。キーの権限を確認してください",
            429 => "APIレート制限に達しました。しばらく待ってから再試行してください",
            500..=599 => "APIサーバーエラー。しばらく待ってから再試行してください",
            _ => "APIエラー",
        };
        eprintln!("[format] API エラー詳細 ({}): {}", status, error_body);
        return Err(format!("{} ({})", user_msg, status).into());
    }

    // レスポンスJSON:
    // {
    //   "content": [{"type": "text", "text": "..."}],
    //   "model": "...",
    //   "stop_reason": "end_turn",
    //   ...
    // }
    let json: serde_json::Value = response.json()?;

    let formatted = json
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(|t| t.as_str())
        .ok_or("Claude API: レスポンスのパースに失敗")?;

    Ok(formatted.to_string())
}
