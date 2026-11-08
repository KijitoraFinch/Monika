# Monika

LLM エージェントのための知識管理ツールチェインです。設計意図・仕様・根拠とコードやデータの具体的な部分を Link で結び、確認した内容を Capture に保存します。次のセッションで関係を辿り、確認時点からの変化を検査できます。

## インストール

[GitHub Releases](https://github.com/KijitoraFinch/Monika/releases) と Homebrew から導入できます。

### GitHub Releases

環境に合うアーカイブと、同じ名前の `.sha256` ファイルをダウンロードします。

| 環境 | アーカイブ |
| --- | --- |
| macOS・Apple Silicon | `monika-aarch64-apple-darwin.tar.gz` |
| macOS・Intel | `monika-x86_64-apple-darwin.tar.gz` |
| Linux・arm64 | `monika-aarch64-unknown-linux-gnu.tar.gz` |
| Linux・x86_64 | `monika-x86_64-unknown-linux-gnu.tar.gz` |

macOS は 13 以降、Linux は glibc 2.35 以降を対象にビルドします。Linux の動作確認環境は Ubuntu 22.04 です。

次は macOS・Apple Silicon の例です。

```sh
shasum -a 256 -c monika-aarch64-apple-darwin.tar.gz.sha256
tar -xzf monika-aarch64-apple-darwin.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 monika-aarch64-apple-darwin/monika "$HOME/.local/bin/monika"
export PATH="$HOME/.local/bin:$PATH"
monika --version
```

Linux ではチェックサムの確認に `sha256sum -c FILE.sha256` を使えます。`PATH` の設定は利用するシェルの設定にも追加してください。アーカイブ内の `documentation/` に利用ガイド・拡張 API・実行可能な操作例があります。

更新は新しい版のチェックサムを確認し、同じ導入先の実行ファイルを置き換えます。削除するときは導入した実行ファイルを削除します。各プロジェクトの `monika.json`・Sidecar・`.monika/` は、そのプロジェクトの保存データとして管理します。

### Homebrew

```sh
brew tap KijitoraFinch/monika https://github.com/KijitoraFinch/Monika
brew install KijitoraFinch/monika/monika
monika --version
```

更新は `brew upgrade monika`、削除は `brew uninstall monika` です。文書は `$(brew --prefix monika)/share/monika/documentation/` に入ります。

## 使い始める

```sh
cd /path/to/project
monika init
monika --help
```

`monika init` は15種類の同梱拡張を登録します。文書（テキスト・Markdown・HTML）、データ（JSON・JSON Lines・YAML・TOML・CSV・TSV）、11言語・構文のコードを選択でき、ファイル・HTTP/HTTPS・Git の観測と組み合わせられます。引用元を表す `anchor` と注釈検査の `assert` もあります。[同梱拡張の使い方](https://github.com/KijitoraFinch/Monika/blob/main/BUNDLED_EXTENSIONS.md)、[Agent 向け利用ガイド](https://github.com/KijitoraFinch/Monika/blob/main/docs/AGENT_GUIDE.md)、[拡張 API](https://github.com/KijitoraFinch/Monika/blob/main/EXTENSIONS.md) を参照してください。同じ文書は配布物の `documentation/` にもあります。JSON の選択・集計には jq を使い、パイプでは `set -o pipefail` を有効にします。Homebrew では jq も依存として導入します。

アーカイブ内の操作例は、Python 3.9 以降と jq を使って試せます。以下のパスはアーカイブを展開したディレクトリを基準としています。

```sh
python3 documentation/examples/bundled_demo.py --monika ./monika
```

同梱拡張を使う一時プロジェクトで、文書・コード・データの選択、比較、Capture、変更検出、GC を実行します。外部拡張と jq パイプの例は `documentation/examples/demo.py` にあります。同梱拡張そのものの利用には Python は不要です。`git:` の観測には Git が必要です。

## ソースからビルド

```sh
cargo install --path . --locked
```

必要な Rust の版は `rust-toolchain.toml` に指定しています。

## ライセンス

Monika 本体は [MIT License](https://github.com/KijitoraFinch/Monika/blob/main/LICENSE) です。依存するソフトウェアと素材の条件は [Third-party notices](https://github.com/KijitoraFinch/Monika/blob/main/THIRD_PARTY_NOTICES.md) にまとめています。配布物にも `documentation/THIRD_PARTY_NOTICES.md` とライセンス全文を同梱します。
