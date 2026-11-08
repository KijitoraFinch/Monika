# Changelog

## 0.1.0

Monika の初回リリースです。LLM エージェントがセッションをまたいで使う知識・設計意図・根拠を、コードやデータの具体的な部分と Link で結び付けて管理できます。

- RegionSelector で指定した内容を読み取り、Region の同一性・包含関係を比較できます。
- Capture に確認時点の内容を保存し、現在との差を検査できます。保存した Capture は Sidecar の Link から参照して保持します。
- 15種類の拡張を同梱します。ファイル・HTTP/HTTPS・Git の観測、文書・構造化データ・11言語／構文のコードの選択、引用元を表す `anchor`、注釈検査の `assert` を利用できます。
- 独自の外部拡張を JSON Lines のプロトコルで追加できます。CLI の JSON 出力は jq と組み合わせられます。
- macOS 13 以降と Linux（glibc 2.35 以降）の arm64／x86_64 向けバイナリを配布します。
- 本体は MIT ライセンスです。配布物には利用ガイド、操作例、依存ライセンスの全文と必要なソースを同梱します。

### インストール

```sh
brew tap KijitoraFinch/monika https://github.com/KijitoraFinch/Monika
brew install KijitoraFinch/monika/monika
monika --version
```

Homebrew では jq も導入されます。`git:` の観測には Git が必要です。

アーカイブから導入する場合は、環境に合う `.tar.gz` と同名の `.sha256` をダウンロードし、チェックサムを確認して展開してください。

| 環境 | アーカイブ |
| --- | --- |
| macOS・Apple Silicon | `monika-aarch64-apple-darwin.tar.gz` |
| macOS・Intel | `monika-x86_64-apple-darwin.tar.gz` |
| Linux・arm64 | `monika-aarch64-unknown-linux-gnu.tar.gz` |
| Linux・x86_64 | `monika-x86_64-unknown-linux-gnu.tar.gz` |

[導入と使い始め方](https://github.com/KijitoraFinch/Monika/blob/v0.1.0/README.md) · [同梱拡張](https://github.com/KijitoraFinch/Monika/blob/v0.1.0/BUNDLED_EXTENSIONS.md) · [Agent 向け利用ガイド](https://github.com/KijitoraFinch/Monika/blob/v0.1.0/docs/AGENT_GUIDE.md)
