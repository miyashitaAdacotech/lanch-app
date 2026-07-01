// launcher_calc.rs - ランチャー内蔵電卓
//
// 再帰降下パーサーで四則演算 (+, -, *, /, %) と括弧をサポート。
// 演算子が1つもなければ None を返す（数字だけの入力を誤検知しない）。

/// 入力文字列を数式として評価し、結果を文字列で返す。
/// 演算子を含まない場合や構文エラーの場合は None。
pub fn try_evaluate(input: &str) -> Option<String> {
    let tokens = tokenize(input)?;
    // 演算子が1つもなければ電卓として扱わない
    if !tokens
        .iter()
        .any(|t| matches!(t, Token::Op(_) | Token::LParen))
    {
        return None;
    }
    let mut pos = 0;
    let result = parse_expr(&tokens, &mut pos)?;
    // 全トークンを消費していなければ構文エラー
    if pos != tokens.len() {
        return None;
    }
    // 整数なら小数点なし、それ以外は適度な桁数
    if result.fract() == 0.0 && result.abs() < 1e15 {
        Some(format!("{}", result as i64))
    } else {
        // 不要な末尾ゼロを除去
        let s = format!("{:.10}", result);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        Some(s.to_string())
    }
}

// === トークナイザ ===

#[derive(Debug, Clone)]
enum Token {
    Num(f64),
    Op(char), // +, -, *, /, %
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' => i += 1,
            '+' | '*' | '/' | '%' => {
                tokens.push(Token::Op(chars[i]));
                i += 1;
            }
            '-' => {
                // 単項マイナス: 先頭、または直前が演算子か左括弧
                let is_unary = tokens.is_empty()
                    || matches!(tokens.last(), Some(Token::Op(_)) | Some(Token::LParen));
                if is_unary {
                    // 数値の一部として読む
                    let start = i;
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                        i += 1;
                    }
                    if i == start + 1 {
                        // '-' だけで数字がない → 演算子として扱う
                        tokens.push(Token::Op('-'));
                    } else {
                        let s: String = chars[start..i].iter().collect();
                        tokens.push(Token::Num(s.parse().ok()?));
                    }
                } else {
                    tokens.push(Token::Op('-'));
                    i += 1;
                }
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                tokens.push(Token::Num(s.parse().ok()?));
            }
            _ => return None, // 不明な文字
        }
    }

    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

// === 再帰降下パーサー ===
// expr   = term (('+' | '-') term)*
// term   = factor (('*' | '/' | '%') factor)*
// factor = Num | '(' expr ')'

fn parse_expr(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let mut left = parse_term(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Op('+') => {
                *pos += 1;
                left += parse_term(tokens, pos)?;
            }
            Token::Op('-') => {
                *pos += 1;
                left -= parse_term(tokens, pos)?;
            }
            _ => break,
        }
    }
    Some(left)
}

fn parse_term(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let mut left = parse_factor(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Op('*') => {
                *pos += 1;
                left *= parse_factor(tokens, pos)?;
            }
            Token::Op('/') => {
                *pos += 1;
                let right = parse_factor(tokens, pos)?;
                if right == 0.0 {
                    return None;
                } // ゼロ除算
                left /= right;
            }
            Token::Op('%') => {
                *pos += 1;
                let right = parse_factor(tokens, pos)?;
                if right == 0.0 {
                    return None;
                }
                left %= right;
            }
            _ => break,
        }
    }
    Some(left)
}

fn parse_factor(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    if *pos >= tokens.len() {
        return None;
    }
    match &tokens[*pos] {
        Token::Num(n) => {
            let v = *n;
            *pos += 1;
            Some(v)
        }
        Token::LParen => {
            *pos += 1; // skip '('
            let v = parse_expr(tokens, pos)?;
            // expect ')'
            if *pos < tokens.len() && matches!(tokens[*pos], Token::RParen) {
                *pos += 1;
                Some(v)
            } else {
                None // 括弧が閉じていない
            }
        }
        _ => None,
    }
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_addition() {
        assert_eq!(try_evaluate("3+5"), Some("8".into()));
    }

    #[test]
    fn test_basic_subtraction() {
        assert_eq!(try_evaluate("10-3"), Some("7".into()));
    }

    #[test]
    fn test_multiplication() {
        assert_eq!(try_evaluate("4*5"), Some("20".into()));
    }

    #[test]
    fn test_division() {
        assert_eq!(try_evaluate("15/3"), Some("5".into()));
    }

    #[test]
    fn test_modulo() {
        assert_eq!(try_evaluate("10%3"), Some("1".into()));
    }

    #[test]
    fn test_precedence() {
        assert_eq!(try_evaluate("2+3*4"), Some("14".into()));
    }

    #[test]
    fn test_parentheses() {
        assert_eq!(try_evaluate("(2+3)*4"), Some("20".into()));
    }

    #[test]
    fn test_nested_parentheses() {
        assert_eq!(try_evaluate("((2+3)*4)+1"), Some("21".into()));
    }

    #[test]
    fn test_decimal() {
        assert_eq!(try_evaluate("1.5+2.5"), Some("4".into()));
    }

    #[test]
    fn test_negative_number() {
        assert_eq!(try_evaluate("-3+5"), Some("2".into()));
    }

    #[test]
    fn test_spaces() {
        assert_eq!(try_evaluate(" 3 + 5 "), Some("8".into()));
    }

    #[test]
    fn test_plain_number_returns_none() {
        assert_eq!(try_evaluate("42"), None);
    }

    #[test]
    fn test_empty_returns_none() {
        assert_eq!(try_evaluate(""), None);
    }

    #[test]
    fn test_invalid_returns_none() {
        assert_eq!(try_evaluate("abc"), None);
    }

    #[test]
    fn test_division_by_zero() {
        assert_eq!(try_evaluate("5/0"), None);
    }

    #[test]
    fn test_complex_expression() {
        assert_eq!(try_evaluate("(10+5)*2/3"), Some("10".into()));
    }
}
