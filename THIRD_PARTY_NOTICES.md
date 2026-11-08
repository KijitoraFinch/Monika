# Third-party notices

Monika 本体は [MIT License](LICENSE) で提供します。第三者のコードや素材には、それぞれのライセンスが適用されます。

- [Cargo 依存の一覧・ライセンス全文・著作権表示](licenses/CARGO_DEPENDENCIES.txt)：`Cargo.lock` の直接・間接依存を対象に、開発用・ビルド用・対象 OS 固有・無効な optional branch の依存も含めています。MIT を選べるものは MIT を選び、Unicode-3.0、Apache-2.0、ISC、BSD-3-Clause、MPL-2.0、CDLA-Permissive-2.0 の必要な表示も保持しています。
- [Rust 標準ライブラリとランタイムの通知](licenses/RUST_STDLIB_NOTICES.txt)：Rust 本体の表示に加え、内部の依存とソース内の個別表示を収録しています。Apache-2.0 WITH LLVM-exception などの追加条件も含みます。
- [ビルドツールの表示](licenses/BUILD_TOOLS.txt)：cargo-dist が生成した workflow の MIT 表示と、別途インストールするロゴ生成ツールのライセンス情報です。

ソースリポジトリ内の IBM Plex フォントは SIL Open Font License 1.1 です。元のライセンスと取得元は [フォントのディレクトリ](https://github.com/KijitoraFinch/Monika/tree/main/assets/logo/fonts) に収録しています。フォント、ロゴ生成ツール、jq、Python は CLI の配布アーカイブに含まれません。

Release のアーカイブには `LICENSE` と `documentation/` 以下のこの文書・通知全文を同梱します。Homebrew では `$(brew --prefix monika)/share/monika/documentation/` に同じ通知が入ります。

HTML/CSS の依存のうち MPL-2.0 が適用される `cssparser`・`cssparser-macros`・`dtoa-short`・`selectors` は、未変更のソースを `licenses/sources/*.crate` に同梱しています。`.crate` は gzip 圧縮された tar アーカイブです。これらのソースは MPL-2.0 の条件で利用できます。バージョン・取得元・checksum とライセンス全文は Cargo 依存の通知に記載しています。Monika 自身が作成したコードは MIT です。
