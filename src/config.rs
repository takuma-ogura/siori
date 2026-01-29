use ratatui::style::Color;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub colors: ColorConfig,
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
    if path.exists() {
        Some(path)
    } else {
        None
    }
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
    opt.as_ref().map(|s| parse_color(s, default)).unwrap_or(default)
}
