use regex::Regex;
use std::path::Path;

use crate::config::RepoConfig;

/// Detected version file with current version
#[derive(Debug, Clone)]
pub struct VersionFile {
    pub path: String,
    pub current_version: String,
    pub pattern: String,
}

/// Auto-detect version files in the repository
pub fn detect_version_files(repo_path: &Path, config: &RepoConfig) -> Vec<VersionFile> {
    let mut files = Vec::new();

    // Auto-detect standard files
    for (filename, pattern) in [
        ("Cargo.toml", r#"version = "{version}""#),
        ("package.json", r#""version": "{version}""#),
        ("pyproject.toml", r#"version = "{version}""#),
        ("VERSION", "{version}"),
    ] {
        if config.version.ignore.contains(&filename.to_string()) {
            continue;
        }
        let file_path = repo_path.join(filename);
        if file_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                if let Some(version) = extract_version(&content, filename) {
                    files.push(VersionFile {
                        path: filename.to_string(),
                        current_version: version,
                        pattern: pattern.to_string(),
                    });
                }
            }
        }
    }

    // Add additional files from config
    for file_config in &config.version.additional_files {
        let file_path = repo_path.join(&file_config.path);
        if file_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                if let Some(version) = extract_with_pattern(&content, &file_config.pattern) {
                    files.push(VersionFile {
                        path: file_config.path.clone(),
                        current_version: version,
                        pattern: file_config.pattern.clone(),
                    });
                }
            }
        }
    }

    files
}

/// Generate tag name from version using tag_format
pub fn generate_tag_name(version: &str, tag_format: &str) -> String {
    tag_format.replace("{version}", version)
}

/// Check if input is a valid version format (e.g., 0.1.6, 1.0.0-beta.1)
pub fn is_valid_version(input: &str) -> bool {
    Regex::new(r"^\d+\.\d+\.\d+")
        .map(|re| re.is_match(input))
        .unwrap_or(false)
}

/// Update version file content with new version
pub fn update_version_content(content: &str, pattern: &str, new_version: &str) -> String {
    let old_pattern = pattern.replace("{version}", r"[0-9]+\.[0-9]+\.[0-9]+[a-zA-Z0-9\.\-]*");
    let new_text = pattern.replace("{version}", new_version);
    if let Ok(re) = Regex::new(&old_pattern) {
        re.replace(content, new_text.as_str()).to_string()
    } else {
        content.to_string()
    }
}

// === Extractors ===

fn extract_version(content: &str, filename: &str) -> Option<String> {
    match filename {
        "Cargo.toml" | "pyproject.toml" => extract_toml_version(content),
        "package.json" => extract_package_json_version(content),
        "VERSION" => extract_plain_version(content),
        _ => None,
    }
}

fn extract_toml_version(content: &str) -> Option<String> {
    let re = Regex::new(r#"^\s*version\s*=\s*"([^"]+)""#).ok()?;
    for line in content.lines() {
        if let Some(caps) = re.captures(line) {
            return Some(caps[1].to_string());
        }
    }
    None
}

fn extract_package_json_version(content: &str) -> Option<String> {
    let re = Regex::new(r#""version"\s*:\s*"([^"]+)""#).ok()?;
    re.captures(content).map(|caps| caps[1].to_string())
}

fn extract_plain_version(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+")
        .ok()?
        .is_match(trimmed)
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn extract_with_pattern(content: &str, pattern: &str) -> Option<String> {
    let regex_pattern =
        regex::escape(pattern).replace(r"\{version\}", r"([0-9]+\.[0-9]+\.[0-9]+[a-zA-Z0-9\.\-]*)");
    let re = Regex::new(&regex_pattern).ok()?;
    re.captures(content).map(|caps| caps[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_toml_version() {
        let content = r#"
[package]
name = "siori"
version = "0.1.5"
"#;
        assert_eq!(extract_toml_version(content), Some("0.1.5".to_string()));
    }

    #[test]
    fn test_extract_package_json_version() {
        let content = r#"{"name": "app", "version": "1.2.3"}"#;
        assert_eq!(
            extract_package_json_version(content),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn test_generate_tag_name() {
        assert_eq!(generate_tag_name("0.1.6", "v{version}"), "v0.1.6");
        assert_eq!(
            generate_tag_name("1.0.0-beta.1", "v{version}"),
            "v1.0.0-beta.1"
        );
        assert_eq!(generate_tag_name("0.1.6", "{version}"), "0.1.6");
    }

    #[test]
    fn test_is_valid_version() {
        assert!(is_valid_version("0.1.6"));
        assert!(is_valid_version("1.0.0-beta.1"));
        assert!(is_valid_version("10.20.30"));
        assert!(!is_valid_version("v0.1.6")); // v prefix is invalid
        assert!(!is_valid_version("abc"));
        assert!(!is_valid_version(""));
    }

    #[test]
    fn test_update_version_content() {
        let content = r#"version = "0.1.5""#;
        let updated = update_version_content(content, r#"version = "{version}""#, "0.1.6");
        assert_eq!(updated, r#"version = "0.1.6""#);
    }
}
