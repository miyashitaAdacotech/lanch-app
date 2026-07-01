// config.rs - 設定ファイルの読み書き
//
// ~/.quick-tools/config.json にJSON形式で設定を保存する。
// ファイルが存在しない場合はデフォルト値で自動生成する。
//
// quick-translate から拡張:
// - Claude API キー / モデル設定を追加
// - Markdown整形ホットキーを追加

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// アプリケーション設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // === 翻訳設定 ===
    /// 翻訳エンジン: "google" または "deepl"
    pub engine: String,

    /// DeepL API キー（空文字列 = 未設定）
    pub deepl_api_key: String,

    /// ソース言語: "auto" で自動判定
    pub source_lang: String,

    /// 日本語テキストの翻訳先
    pub target_lang_ja: String,

    /// 英語テキストの翻訳先
    pub target_lang_en: String,

    // === Claude 設定（Markdown整形用） ===
    /// Claude API キー（レガシー: 現在は Claude CLI 経由のため不要）
    #[serde(default)]
    pub claude_api_key: String,

    /// Claude モデル名（Claude CLI に渡すモデル指定）
    pub claude_model: String,

    // === UI 設定 ===
    /// フォントサイズ
    pub font_size: f32,

    /// ウィンドウの透明度 (0.0 - 1.0)
    pub opacity: f32,

    /// 翻訳ログを有効にするか
    pub log_enabled: bool,

    // === ホットキー設定 ===
    /// ポップアップ起動のホットキー
    pub hotkey_popup: String,

    /// 選択テキスト翻訳のホットキー
    pub hotkey_selected: String,

    /// Markdown整形のホットキー
    pub hotkey_format: String,

    /// クリップボード履歴のホットキー
    pub hotkey_clipboard_history: String,

    // === ランチャー設定 ===
    /// ランチャーのホットキー
    pub hotkey_launcher: String,

    /// ランチャーショートカット定義
    pub launcher_shortcuts: Vec<LauncherShortcut>,
}

/// ランチャーのショートカット定義
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherShortcut {
    /// ショートカットキー（例: "cc"）
    pub key: String,
    /// 表示名
    pub name: String,
    /// 実行コマンド
    pub command: String,
    /// コマンド引数
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: "google".to_string(),
            deepl_api_key: String::new(),
            source_lang: "auto".to_string(),
            target_lang_ja: "en".to_string(),
            target_lang_en: "ja".to_string(),
            claude_api_key: String::new(),
            claude_model: "claude-haiku-4-5-20251001".to_string(),
            font_size: 16.0,
            opacity: 0.95,
            log_enabled: true,
            hotkey_popup: "ctrl+shift+t".to_string(),
            hotkey_selected: "ctrl+shift+y".to_string(),
            hotkey_format: "ctrl+alt+f".to_string(),
            hotkey_clipboard_history: "ctrl+shift+v".to_string(),
            hotkey_launcher: "ctrl+alt+space".to_string(),
            launcher_shortcuts: vec![
                LauncherShortcut {
                    key: "cc".into(),
                    name: "Claude Code".into(),
                    command: "cmd".into(),
                    args: vec![
                        "/C".into(),
                        "start".into(),
                        "".into(),
                        "cmd".into(),
                        "/K".into(),
                        "claude".into(),
                    ],
                },
                LauncherShortcut {
                    key: "cd".into(),
                    name: "Codex".into(),
                    command: "cmd".into(),
                    args: vec![
                        "/C".into(),
                        "start".into(),
                        "".into(),
                        "cmd".into(),
                        "/K".into(),
                        "codex".into(),
                    ],
                },
            ],
        }
    }
}

/// 設定ディレクトリのパスを返す
/// Windows: C:\Users\<ユーザー名>\.lanch-app
fn config_dir() -> PathBuf {
    dirs::home_dir()
        .expect("ホームディレクトリが見つかりません")
        .join(".lanch-app")
}

/// 設定ファイルのパスを返す
fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// 設定ファイルを読み込む
///
/// ファイルが存在しない場合はデフォルト設定を生成して保存する。
/// JSONにないフィールドは `#[serde(default)]` によりデフォルト値が使われる。
pub fn load_config() -> Config {
    let path = config_file();

    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(config) => return config,
                Err(e) => {
                    eprintln!("設定ファイルのパースに失敗: {}", e);
                }
            },
            Err(e) => {
                eprintln!("設定ファイルの読み込みに失敗: {}", e);
            }
        }
    }

    // 旧 quick-translate の設定ファイルからマイグレーション
    let old_config = migrate_from_quick_translate();
    let config = old_config.unwrap_or_default();
    let _ = save_config(&config);
    config
}

/// quick-translate の設定を読み込んでマイグレーションする
fn migrate_from_quick_translate() -> Option<Config> {
    let old_path = dirs::home_dir()?
        .join(".quick-translate")
        .join("config.json");

    if !old_path.exists() {
        return None;
    }

    let contents = fs::read_to_string(&old_path).ok()?;
    let old: serde_json::Value = serde_json::from_str(&contents).ok()?;

    let mut config = Config::default();

    if let Some(v) = old.get("engine").and_then(|v| v.as_str()) {
        config.engine = v.to_string();
    }
    if let Some(v) = old.get("deepl_api_key").and_then(|v| v.as_str()) {
        config.deepl_api_key = v.to_string();
    }
    if let Some(v) = old.get("source_lang").and_then(|v| v.as_str()) {
        config.source_lang = v.to_string();
    }
    if let Some(v) = old.get("target_lang_ja").and_then(|v| v.as_str()) {
        config.target_lang_ja = v.to_string();
    }
    if let Some(v) = old.get("target_lang_en").and_then(|v| v.as_str()) {
        config.target_lang_en = v.to_string();
    }
    if let Some(v) = old.get("font_size").and_then(|v| v.as_f64()) {
        config.font_size = v as f32;
    }
    if let Some(v) = old.get("opacity").and_then(|v| v.as_f64()) {
        config.opacity = v as f32;
    }
    if let Some(v) = old.get("hotkey_popup").and_then(|v| v.as_str()) {
        config.hotkey_popup = v.to_string();
    }
    if let Some(v) = old.get("hotkey_selected").and_then(|v| v.as_str()) {
        config.hotkey_selected = v.to_string();
    }

    eprintln!("quick-translate の設定をマイグレーションしました");
    Some(config)
}

/// 設定ファイルを保存する
pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;

    let path = config_file();
    let json = serde_json::to_string_pretty(config)?;
    fs::write(&path, json)?;

    Ok(())
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- デフォルト値のテスト ---

    #[test]
    fn test_default_config_values() {
        let config = Config::default();
        assert_eq!(config.engine, "google");
        assert_eq!(config.source_lang, "auto");
        assert_eq!(config.target_lang_ja, "en");
        assert_eq!(config.target_lang_en, "ja");
        assert_eq!(config.claude_model, "claude-haiku-4-5-20251001");
        assert_eq!(config.font_size, 16.0);
        assert_eq!(config.opacity, 0.95);
        assert!(config.log_enabled);
    }

    #[test]
    fn test_default_hotkeys() {
        let config = Config::default();
        assert_eq!(config.hotkey_popup, "ctrl+shift+t");
        assert_eq!(config.hotkey_selected, "ctrl+shift+y");
        assert_eq!(config.hotkey_format, "ctrl+alt+f");
        assert_eq!(config.hotkey_clipboard_history, "ctrl+shift+v");
    }

    #[test]
    fn test_default_api_keys_empty() {
        let config = Config::default();
        assert!(config.deepl_api_key.is_empty());
        assert!(config.claude_api_key.is_empty());
    }

    // --- シリアライズ/デシリアライズのテスト ---

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.engine, config.engine);
        assert_eq!(deserialized.source_lang, config.source_lang);
        assert_eq!(deserialized.hotkey_popup, config.hotkey_popup);
        assert_eq!(deserialized.claude_model, config.claude_model);
        assert_eq!(deserialized.font_size, config.font_size);
        assert_eq!(
            deserialized.hotkey_clipboard_history,
            config.hotkey_clipboard_history
        );
    }

    #[test]
    fn test_deserialize_partial_json_uses_defaults() {
        // 一部のフィールドだけのJSONでもデフォルト値で補完される
        let json = r#"{"engine": "deepl", "font_size": 20.0}"#;
        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.engine, "deepl");
        assert_eq!(config.font_size, 20.0);
        // 省略されたフィールドはデフォルト値
        assert_eq!(config.source_lang, "auto");
        assert_eq!(config.hotkey_popup, "ctrl+shift+t");
        assert_eq!(config.claude_model, "claude-haiku-4-5-20251001");
        assert_eq!(config.hotkey_clipboard_history, "ctrl+shift+v");
    }

    #[test]
    fn test_deserialize_unknown_fields_ignored() {
        // 未知のフィールドがあってもパースに失敗しない
        let json = r#"{"engine": "google", "unknown_field": "value", "another": 42}"#;
        let result: Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_deserialize_empty_json_uses_all_defaults() {
        let json = "{}";
        let config: Config = serde_json::from_str(json).unwrap();
        let default = Config::default();

        assert_eq!(config.engine, default.engine);
        assert_eq!(config.hotkey_popup, default.hotkey_popup);
        assert_eq!(config.claude_model, default.claude_model);
    }

    // --- 設定値のバリデーション的テスト ---

    #[test]
    fn test_opacity_in_valid_range() {
        let config = Config::default();
        assert!(config.opacity >= 0.0 && config.opacity <= 1.0);
    }

    #[test]
    fn test_font_size_positive() {
        let config = Config::default();
        assert!(config.font_size > 0.0);
    }

    #[test]
    fn test_custom_config_serialization() {
        let mut config = Config::default();
        config.engine = "deepl".to_string();
        config.deepl_api_key = "test-key-123".to_string();
        config.hotkey_clipboard_history = "alt+v".to_string();

        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("deepl"));
        assert!(json.contains("test-key-123"));
        assert!(json.contains("alt+v"));
    }
}
