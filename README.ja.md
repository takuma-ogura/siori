# siori

バイブコーダーのためのシンプルな Git TUI。

![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

[English](README.md) | 日本語

## 特徴

- **コンパクトな UI** - 狭いターミナルペイン向けに設計
- **Files タブ** - diff 統計付きでファイルをステージ/アンステージ
- **Log タブ** - グラフ表示付きのコミット履歴
- **キーボード駆動** - Vim スタイルのナビゲーション (j/k)
- **自動更新** - ファイル変更を自動検出
- **リポジトリ切り替え** - リポジトリ間をすばやく切り替え

## インストール

### Homebrew (macOS/Linux)

```bash
brew tap takuma-ogura/siori
brew install siori
```

### GitHub Releases

[Releases](https://github.com/takuma-ogura/siori/releases) から最新のバイナリをダウンロードできます。

```bash
# macOS (Apple Silicon)
curl -sL https://github.com/takuma-ogura/siori/releases/latest/download/siori-aarch64-apple-darwin.tar.gz | tar xz
sudo mv siori /usr/local/bin/

# macOS (Intel)
curl -sL https://github.com/takuma-ogura/siori/releases/latest/download/siori-x86_64-apple-darwin.tar.gz | tar xz
sudo mv siori /usr/local/bin/

# Linux (x86_64)
curl -sL https://github.com/takuma-ogura/siori/releases/latest/download/siori-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv siori /usr/local/bin/
```

### Cargo

```bash
cargo install siori
```

### ソースからビルド

```bash
git clone https://github.com/takuma-ogura/siori.git
cd siori
cargo install --path .
```

## 使い方

```bash
# 任意の git リポジトリで実行
siori
```

## キーバインド

### Files タブ

| キー | アクション |
|------|------------|
| `j` / `k` | 上下に移動 |
| `Space` | ファイルをステージ/アンステージ |
| `c` | コミットメッセージを入力 |
| `Enter` | コミット（入力モード時） |
| `P` | Push |
| `Tab` | Log タブに切り替え |
| `r` | リポジトリを切り替え |
| `q` | 終了 |

### Log タブ

| キー | アクション |
|------|------------|
| `j` / `k` | コミットを移動 |
| `t` | タグを作成 |
| `T` | タグを Push |
| `d` | タグを削除 |
| `P` | Push |
| `p` | Pull |
| `Tab` | Files タブに切り替え |
| `r` | リポジトリを切り替え |
| `q` | 終了 |

## 設定

設定ファイルの場所: `~/.config/siori/config.toml`

```toml
[ui]
show_hints = true

[colors]
# ANSI カラー名: black, red, green, yellow, blue, magenta, cyan, white
# または RGB hex: "#ff0000"
text = "white"
staged = "green"
modified = "yellow"
```

## 必要条件

- Git リポジトリ
- 256 色対応のターミナル

## ライセンス

MIT License - 詳細は [LICENSE](LICENSE) をご覧ください。
