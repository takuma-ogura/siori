# siori

A simple Git TUI for vibe coders.

![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

English | [日本語](README.ja.md)

## Features

- **Compact UI** - Designed for narrow terminal panes
- **Files Tab** - Stage/unstage files with diff stats
- **Log Tab** - Commit history with graph visualization
- **Keyboard-driven** - Vim-style navigation (j/k)
- **Auto-refresh** - Detects file changes automatically
- **Repository Switcher** - Quick switch between repos

## Installation

### Homebrew (macOS/Linux)

```bash
brew tap takuma-ogura/siori
brew install siori
```

### GitHub Releases

Download the latest binary from [Releases](https://github.com/takuma-ogura/siori/releases).

```bash
# macOS (Apple Silicon)
curl -sL https://github.com/takuma-ogura/siori/releases/latest/download/siori-aarch64-apple-darwin.tar.gz | tar xz
sudo mv siori /usr/local/bin/

# Linux (x86_64)
curl -sL https://github.com/takuma-ogura/siori/releases/latest/download/siori-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv siori /usr/local/bin/
```

### Cargo

```bash
cargo install siori
```

### From Source

```bash
git clone https://github.com/takuma-ogura/siori.git
cd siori
cargo install --path .
```

## Usage

```bash
# Run in any git repository
siori
```

## Key Bindings

### Files Tab

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate up/down |
| `Space` | Stage/unstage file |
| `c` | Enter commit message |
| `Enter` | Commit (in input mode) |
| `P` | Push |
| `Tab` | Switch to Log tab |
| `r` | Switch repository |
| `q` | Quit |

### Log Tab

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate commits |
| `t` | Create tag |
| `T` | Push tags |
| `d` | Delete tag |
| `P` | Push |
| `p` | Pull |
| `Tab` | Switch to Files tab |
| `r` | Switch repository |
| `q` | Quit |

## Configuration

Config file location: `~/.config/siori/config.toml`

```toml
[ui]
show_hints = true

[colors]
# ANSI color names: black, red, green, yellow, blue, magenta, cyan, white
# Or RGB hex: "#ff0000"
text = "white"
staged = "green"
modified = "yellow"
```

## Requirements

- Git repository
- Terminal with 256-color support

## License

MIT License - see [LICENSE](LICENSE) for details.
