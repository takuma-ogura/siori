use ratatui::style::Color;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Global config (~/.config/siori/config.toml)
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub colors: ColorConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub diff: DiffConfig,
}

#[derive(Debug, Default, Deserialize)]
pub struct DiffConfig {
    /// Skip confirmation popup and copy directly (default: false)
    #[serde(default)]
    pub skip_confirm: bool,
}

/// Repository-specific config (.siori.toml)
#[derive(Debug, Default, Deserialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub version: VersionConfig,
}

#[derive(Debug, Deserialize)]
pub struct VersionConfig {
    /// Show confirmation dialog before updating version (default: true)
    #[serde(default = "default_true")]
    pub confirm: bool,

    /// Commit message template (default: "chore: bump version to {version}")
    #[serde(default = "default_commit_message")]
    pub commit_message: String,

    /// Tag format (default: "v{version}")
    #[serde(default = "default_tag_format")]
    pub tag_format: String,

    /// Additional version files to update
    #[serde(default)]
    pub additional_files: Vec<VersionFileConfig>,

    /// Files to ignore from auto-detection
    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VersionFileConfig {
    pub path: String,
    pub pattern: String,
}

fn default_commit_message() -> String {
    "chore: bump version to {version}".to_string()
}

fn default_tag_format() -> String {
    "v{version}".to_string()
}

impl Default for VersionConfig {
    fn default() -> Self {
        Self {
            confirm: true,
            commit_message: default_commit_message(),
            tag_format: default_tag_format(),
            additional_files: Vec::new(),
            ignore: Vec::new(),
        }
    }
}

impl RepoConfig {
    pub fn load(repo_path: &Path) -> Self {
        let config_path = repo_path.join(".siori.toml");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str(&content) {
                    return config;
                }
            }
        }
        RepoConfig::default()
    }
}

#[derive(Debug, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub show_hints: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { show_hints: true }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
pub struct ColorConfig {
    pub staged: Option<String>,
    pub modified: Option<String>,
    pub untracked: Option<String>,
    pub selected_bg: Option<String>,
    pub text: Option<String>,
    pub text_bright: Option<String>,
    pub dim: Option<String>,
    pub info: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }
}

fn config_path() -> Option<PathBuf> {
    // 1. XDG準拠: ~/.config/siori/config.toml (Linux/macOS共通)
    if let Some(home) = std::env::var_os("HOME") {
        let xdg_path = PathBuf::from(home).join(".config/siori/config.toml");
        if xdg_path.exists() {
            return Some(xdg_path);
        }
    }
    // 2. OS標準: ~/Library/Application Support/siori/ (macOS)
    let proj_dirs = directories::ProjectDirs::from("", "", "siori")?;
    let path = proj_dirs.config_dir().join("config.toml");
    if path.exists() { Some(path) } else { None }
}

/// 文字列からColorに変換
pub fn parse_color(s: &str, default: Color) -> Color {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" | "dark_gray" | "darkgray" => Color::DarkGray,
        "light_red" | "lightred" => Color::LightRed,
        "light_green" | "lightgreen" => Color::LightGreen,
        "light_yellow" | "lightyellow" => Color::LightYellow,
        "light_blue" | "lightblue" => Color::LightBlue,
        "light_magenta" | "lightmagenta" => Color::LightMagenta,
        "light_cyan" | "lightcyan" => Color::LightCyan,
        "reset" | "default" => Color::Reset,
        hex if hex.starts_with('#') && hex.len() == 7 => {
            let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
            Color::Rgb(r, g, b)
        }
        _ => default,
    }
}

/// 設定から色を取得、なければデフォルト
pub fn get_color(opt: &Option<String>, default: Color) -> Color {
    opt.as_ref()
        .map(|s| parse_color(s, default))
        .unwrap_or(default)
}
