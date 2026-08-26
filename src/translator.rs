// translator.rs - 翻訳エンジン
//
// Google翻訳の無料エンドポイント / DeepL API を使ってテキストを翻訳する。

use crate::config::Config;
use crate::lang::detect_target_lang;

/// 翻訳結果を格納する構造体
#[derive(Debug, Clone)]
pub struct TranslationResult {
    /// 翻訳されたテキスト
    pub translated: String,
    /// 翻訳先言語コード
    #[allow(dead_code)]
    pub target_lang: String,
}

fn has_any_line_break(text: &str) -> bool {
    text.contains('\n') || text.contains('\r')
}

fn is_whitespace_only(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

fn split_nonempty_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn normalize_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn source_line_weights(lines: &[&str]) -> Vec<usize> {
    lines
        .iter()
        .map(|line| line.chars().count().max(1))
        .collect()
}

fn distribute_indices(total_units: usize, weights: &[usize]) -> Vec<usize> {
    if weights.is_empty() {
        return vec![0];
    }
    if total_units == 0 {
        return vec![0; weights.len() + 1];
    }

    let total_weight: usize = weights.iter().sum();
    if total_weight == 0 {
        let mut indices = Vec::with_capacity(weights.len() + 1);
        indices.push(0);
        for i in 1..=weights.len() {
            indices.push(((total_units * i) / weights.len()).min(total_units));
        }
        return indices;
    }

    let mut indices = Vec::with_capacity(weights.len() + 1);
    indices.push(0);
    let mut cumulative = 0usize;
    for weight in weights {
        cumulative += *weight;
        let idx = ((total_units as f64) * (cumulative as f64) / (total_weight as f64)).round() as usize;
        indices.push(idx.min(total_units));
    }
    indices
}

fn reflow_by_source_lines(source: &str, translated: &str) -> String {
    let source_lines = split_nonempty_lines(source);
    if source_lines.len() <= 1 {
        return translated.trim().to_string();
    }

    if has_any_line_break(translated) {
        return translated.trim().to_string();
    }

    let weights = source_line_weights(&source_lines);
    let normalized = normalize_spaces(translated);
    if normalized.is_empty() {
        return String::new();
    }

    if normalized.contains(' ') {
        let units: Vec<&str> = normalized.split(' ').filter(|s| !s.is_empty()).collect();
        let indices = distribute_indices(units.len(), &weights);
        let mut lines = Vec::with_capacity(weights.len());

        for i in 0..weights.len() {
            let mut start = indices[i];
            let mut end = indices[i + 1];
            if i > 0 && start < indices[i - 1] {
                start = indices[i - 1];
            }
            if end < start {
                end = start;
            }
            if i + 1 == weights.len() {
                end = units.len();
            }

            if start >= units.len() {
                lines.push(String::new());
                continue;
            }

            let end = end.min(units.len());
            lines.push(units[start..end].join(" "));
        }

        let joined = lines
            .into_iter()
            .filter(|line| !is_whitespace_only(line))
            .collect::<Vec<_>>()
            .join("\n");
        return if joined.is_empty() {
            normalized
        } else {
            joined
        };
    }

    let units: Vec<char> = normalized.chars().collect();
    let indices = distribute_indices(units.len(), &weights);
    let mut out = String::new();
    for i in 0..weights.len() {
        let mut start = indices[i];
        let mut end = indices[i + 1];
        if i > 0 && start < indices[i - 1] {
            start = indices[i - 1];
        }
        if end < start {
            end = start;
        }
        if i + 1 == weights.len() {
            end = units.len();
        }
        if start >= units.len() {
            continue;
        }
        let end = end.min(units.len());
        if !out.is_empty() {
            out.push('\n');
        }
        for ch in &units[start..end] {
            out.push(*ch);
        }
    }

    if out.is_empty() {
        normalized
    } else {
        out
    }
}

fn wrap_cjk_line(line: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= max_chars {
        return vec![line.to_string()];
    }

    let break_chars = ['。', '、', '！', '？', '：', '；', ')', '）'];
    let mut out = Vec::new();
    let mut start = 0usize;

    while start < chars.len() {
        let mut end = (start + max_chars).min(chars.len());
        if end < chars.len() {
            let search_start = start.saturating_add(max_chars / 2);
            let mut candidate = None;
            for i in (search_start..end).rev() {
                if break_chars.contains(&chars[i]) {
                    candidate = Some(i + 1);
                    break;
                }
            }
            if let Some(c) = candidate {
                end = c;
            }
        }
        out.push(chars[start..end].iter().collect());
        start = end;
    }

    out
}

fn wrap_space_line(line: &str, max_chars: usize) -> Vec<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    for word in words {
        let candidate_len = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if candidate_len > max_chars && !current.is_empty() {
            out.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn smart_wrap_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if has_any_line_break(trimmed) {
        return trimmed.to_string();
    }

    let has_space = trimmed.contains(' ');
    let units = trimmed.chars().count();
    let limit = if has_space { 72 } else { 42 };
    if units <= limit {
        return trimmed.to_string();
    }

    let wrapped = if has_space {
        wrap_space_line(trimmed, limit)
    } else {
        wrap_cjk_line(trimmed, limit)
    };

    wrapped.join("\n")
}

/// HTTP ステータスから利用者向けのヒントを作る
fn google_status_hint(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        429 => "Google に自動リクエストとして一時ブロックされています。時間をおくか、config.json の engine を \"deepl\" に切り替えてください",
        403 => "Google にアクセスを拒否されました。ネットワーク（プロキシ/VPN）を確認してください",
        _ => "Google 翻訳エンドポイントがエラーを返しました",
    }
}

/// エラー表示用に本文の先頭だけを 1 行へ切り詰める
fn body_snippet(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(120).collect()
}

/// Google 翻訳のレスポンス JSON から訳文を取り出す
///
/// `[["訳文", "en"]]` 形式が基本だが、長文が分割されて
/// `[["前半", "en"], ["後半", "en"]]` になる場合もあるため全要素を連結する。
fn extract_google_translation(json: &serde_json::Value) -> String {
    let Some(items) = json.as_array() else {
        return String::new();
    };

    let mut result = String::new();
    for item in items {
        match item {
            serde_json::Value::String(s) => result.push_str(s),
            serde_json::Value::Array(inner) => {
                if let Some(s) = inner.first().and_then(|v| v.as_str()) {
                    result.push_str(s);
                }
            }
            _ => {}
        }
    }
    result
}

/// Google翻訳の無料APIを使って翻訳する
///
/// 旧エンドポイント `translate.googleapis.com/translate_a/single?client=gtx` は
/// bot 判定で HTTP 429 + HTML（"automated queries"）を返すようになり、
/// JSON パースが reqwest の "error decoding response body" で失敗していた。
/// Chrome の翻訳拡張が使う `clients5.google.com/translate_a/t?client=dict-chrome-ex`
/// は同じ条件で 200 + JSON を返すため、そちらを使う。
/// 長文で URL 長の上限に当たらないよう、本文は POST ボディで送る。
fn google_translate(
    text: &str,
    target: &str,
    source: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .post("https://clients5.google.com/translate_a/t")
        .query(&[
            ("client", "dict-chrome-ex"),
            ("sl", source),
            ("tl", target),
        ])
        .form(&[("q", text)])
        .send()?;

    let status = response.status();
    let body = response.text()?;

    if !status.is_success() {
        return Err(format!("Google翻訳エラー ({}): {}", status, google_status_hint(status)).into());
    }

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        format!(
            "Google翻訳: 応答を解析できませんでした ({}): {}",
            e,
            body_snippet(&body)
        )
    })?;

    let result = extract_google_translation(&json);

    if result.is_empty() {
        Err("翻訳結果を取得できませんでした".into())
    } else {
        Ok(result)
    }
}

/// DeepL API を使って翻訳する
fn deepl_translate(
    text: &str,
    target: &str,
    api_key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::new();

    let target_upper = target.to_uppercase();

    let base_url = if api_key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };

    let response = client
        .post(base_url)
        .header("Authorization", format!("DeepL-Auth-Key {}", api_key))
        .form(&[
            ("text", text),
            ("target_lang", &target_upper),
        ])
        .send()?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("DeepL API エラー ({}): {}", status, body).into());
    }

    let json: serde_json::Value = response.json()?;

    let translated = json
        .get("translations")
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
        .ok_or("DeepL: 翻訳結果のパースに失敗")?;

    Ok(translated.to_string())
}

/// テキストを翻訳する（メイン関数）
pub fn translate(text: &str, config: &Config) -> Result<TranslationResult, Box<dyn std::error::Error>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(TranslationResult {
            translated: String::new(),
            target_lang: String::new(),
        });
    }

    let target = detect_target_lang(text, config);
    let source = &config.source_lang;

    let translated_raw = match config.engine.as_str() {
        "deepl" => {
            if config.deepl_api_key.is_empty() {
                return Err("DeepL APIキーが未設定です。~/.quick-tools/config.json を編集してください".into());
            }
            deepl_translate(text, &target, &config.deepl_api_key)?
        }
        _ => {
            google_translate(text, &target, source)?
        }
    };
    let translated = smart_wrap_text(&reflow_by_source_lines(text, &translated_raw));

    Ok(TranslationResult {
        translated,
        target_lang: target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_google_translation_single_entry() {
        // clients5 の基本形: [["訳文", "検出元言語"]]
        let json: serde_json::Value =
            serde_json::from_str(r#"[["こんにちは世界","en"]]"#).unwrap();
        assert_eq!(extract_google_translation(&json), "こんにちは世界");
    }

    #[test]
    fn test_extract_google_translation_multiple_entries() {
        let json: serde_json::Value =
            serde_json::from_str(r#"[["前半。","en"],["後半。","en"]]"#).unwrap();
        assert_eq!(extract_google_translation(&json), "前半。後半。");
    }

    #[test]
    fn test_extract_google_translation_flat_strings() {
        let json: serde_json::Value = serde_json::from_str(r#"["訳文","en"]"#).unwrap();
        assert_eq!(extract_google_translation(&json), "訳文en");
    }

    #[test]
    fn test_extract_google_translation_unexpected_shape() {
        let json: serde_json::Value = serde_json::from_str(r#"{"error":"blocked"}"#).unwrap();
        assert_eq!(extract_google_translation(&json), "");
    }

    #[test]
    fn test_google_status_hint_mentions_deepl_on_429() {
        let hint = google_status_hint(reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert!(hint.contains("deepl"));
    }

    #[test]
    fn test_body_snippet_flattens_and_truncates() {
        let body = "<html>\n  <title>Sorry...</title>\n</html>";
        let snippet = body_snippet(body);
        assert!(!snippet.contains('\n'));
        assert!(snippet.starts_with("<html> <title>Sorry..."));

        let long = "a".repeat(500);
        assert_eq!(body_snippet(&long).chars().count(), 120);
    }

    /// 実ネットワークを使う疎通確認（既定ではスキップ）
    ///
    /// `cargo test -- --ignored` で実行し、Google 側のエンドポイント仕様変更を検知する。
    #[test]
    #[ignore = "ネットワーク接続が必要"]
    fn test_google_translate_live() {
        let out = google_translate("Hello world", "ja", "auto").expect("翻訳に失敗");
        assert!(!out.is_empty());
        assert!(crate::lang::is_japanese(&out), "訳文が日本語ではない: {}", out);
    }

    #[test]
    fn test_reflow_keeps_existing_line_breaks() {
        // 新エンドポイントは改行を保持して返すため、再配分せずそのまま使う
        let out = reflow_by_source_lines("Hello.\nWorld.", "こんにちは。\n世界。");
        assert_eq!(out, "こんにちは。\n世界。");
    }
}
