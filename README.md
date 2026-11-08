# Monika

LLM エージェントのための知識管理ツールチェイン
## インストール

[GitHub Releases](https://github.com/KijitoraFinch/Monika/releases) または Homebrew から導入できます。

### GitHub Releases


| 環境 | アーカイブ |
| --- | --- |
| macOS・Apple Silicon | `monika-aarch64-apple-darwin.tar.gz` |
| macOS・Intel | `monika-x86_64-apple-darwin.tar.gz` |
| Linux・arm64 | `monika-aarch64-unknown-linux-gnu.tar.gz` |
| Linux・x86_64 | `monika-x86_64-unknown-linux-gnu.tar.gz` |

macOS は 13 以降、Linux は glibc 2.35 以降を対象にビルドします。Linux の動作確認環境は Ubuntu 22.04 です。

macOS・Apple Silicon の場合

```sh
shasum -a 256 -c monika-aarch64-apple-darwin.tar.gz.sha256
tar -xzf monika-aarch64-apple-darwin.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 monika-aarch64-apple-darwin/monika "$HOME/.local/bin/monika"
export PATH="$HOME/.local/bin:$PATH"
monika --version
```



### Homebrew

```sh
brew tap KijitoraFinch/monika https://github.com/KijitoraFinch/Monika
brew install KijitoraFinch/monika/monika
monika --version
```
なお、ドキュメント類は `$(brew --prefix monika)/share/monika/documentation/` にインストールされます。

## 使い始める

```sh
cd /path/to/project
monika init
monika --help
```

`monika init` は15種類の同梱拡張を登録します。文書（テキスト・Markdown・HTML）、データ（JSON・JSON Lines・YAML・TOML・CSV・TSV）、11言語・構文のコードを選択でき、ファイル・HTTP/HTTPS・Git の観測と組み合わせられます。引用元を表す `anchor` と注釈検査の `assert` もあります。[同梱拡張の使い方](https://github.com/KijitoraFinch/Monika/blob/main/BUNDLED_EXTENSIONS.md)、[Agent 向け利用ガイド](https://github.com/KijitoraFinch/Monika/blob/main/docs/AGENT_GUIDE.md)、[拡張 API](https://github.com/KijitoraFinch/Monika/blob/main/EXTENSIONS.md) を参照してください。同じ文書は配布物の `documentation/` にもあります。パイプで利用する際には `set -o pipefail` を有効にすることを推奨しています。

## ソースからビルド

```sh
cargo install --path . --locked
```

必要な Rust の版は `rust-toolchain.toml` に指定しています。

## ライセンス

MIT License](https://github.com/KijitoraFinch/Monika/blob/main/LICENSE) 
[Third-party notices](https://github.com/KijitoraFinch/Monika/blob/main/THIRD_PARTY_NOTICES.md)
