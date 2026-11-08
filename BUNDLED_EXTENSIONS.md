# 同梱拡張

`monika init` は15種類の同梱拡張を登録します。パーサーと HTTP クライアントは実行ファイルに含まれ、Python・Node.js・Language Server の追加導入は不要です。`git:` の取得には Git を使います。

| 拡張 | 扱うもの | 指定例（`query`） |
| --- | --- | --- |
| `file` | ローカルの通常ファイルの観測、全体の hash・サイズ | `null` |
| `http` | HTTP/HTTPS の観測 | `html`・`json` などと組み合わせる |
| `git` | ローカル Git のコミット内のファイルの観測 | `resource: "git:src/main.rs?rev=HEAD"` |
| `text` | UTF-8 の全文・行・文字列 | `{"lines":[2,8]}` |
| `markdown` | CommonMark の節・見出し・コードブロック | `{"section":["設計","保存"]}` |
| `html` | HTML5 の DOM と CSS selector | `{"css":"article > p"}` |
| `json` | JSON の値・部分木 | `{"pointer":"/items/0"}` |
| `jsonl` | JSON Lines のレコード・部分木 | `{"lines":[1,3]}` |
| `yaml` | YAML の値・部分木 | `{"pointer":"/jobs/test"}` |
| `toml` | TOML の型付きの値・部分木 | `{"pointer":"/package/name"}` |
| `csv`・`tsv` | 表の行・列・値で絞ったセル | `{"columns":["name"],"where":[{"column":"status","equals":"ok"}]}` |
| `code` | 11言語・構文の宣言、Tree-sitter query | `{"symbol":"run","scope":["Worker"]}` |
| `anchor` | 引用元を表す、観測不要の名前付きの起点 | `"report:conclusion"`（`resource` は省略） |
| `assert` | 注釈で指定した Region content の期待値検査 | `annotation.expect` に条件を記述 |

指定・空の選択・比較・エラーの詳細は [文書](docs/extensions/documents.md)、[構造化データと表](docs/extensions/data.md)、[コード](docs/extensions/code.md) を参照してください。コードは Rust・Python・JavaScript/JSX・TypeScript・TSX・Go・Java・C・C++・Ruby・Bash を構文として解析します。Language Server による名前解決は行いません。

## 登録

```sh
monika init
monika extension list
monika extension show markdown
```

既存の設定への追加は `monika extension enable text markdown code`、一括登録は `monika extension enable --all` です。登録済みの同じ設定を保ち、名前・観測 scheme の競合は理由を示して終了します。独自の `file` 提供元を使っている場合は、その登録を保って解釈拡張だけを追加できます。空の設定から始める場合は `monika init --bare` を使います。

同梱拡張も [外部拡張 API](EXTENSIONS.md) で動作します。設定に入る `command` の先頭の `@monika` は、起動中の実行ファイルを示します。設定を別マシンに共有したり実行ファイルを移動したりしても、固定したインストール先や `PATH` に依存しません。

```json
{"command":["@monika","extension","serve","text"],"revision":"1","schemes":[],"timeout_ms":30000}
```

## Resource と比較

ローカルの `resource` はプロジェクトルート相対です。`file` が bytes を観測し、各拡張が同じ Observation を解釈します。`text`・`markdown`・`code` は共通の domain `monika.bytes.v1` を持ち、初期設定は `text` を比較担当にします。

```json
{"left":{"extension":"text","resource":"src/main.rs"},"right":{"extension":"code","resource":"src/main.rs","query":{"symbol":"main"}}}
```

この入力を `monika compare` に渡すと、全文と関数の包含を調べられます。Capture の変更検査では選んだ `content` を比べ、選択外への追記による行番号の移動だけでは変更になりません。行番号で選んだ場合は、同じ行番号が指す現在の内容を検査します。

`file` の解決結果は `{"sha256":"…","size":123}` です。バイナリ成果物も参照・変更検査できます。本文を引用する Capture には、その形式を解釈する拡張を使います。不在と、権限不足・ディレクトリなどの読み取り失敗は区別します。

### HTTP/HTTPS

```json
{"extension":"html","resource":"https://example.org/report","query":{"css":"main"}}
```

`http` は GET の応答本文を観測し、TLS 証明書を検証します。リダイレクトは最大10回、404・410は不在、その他の非成功ステータスはエラーです。`Accept-Encoding: identity` を要求し、圧縮された応答はエラーにします。独自の認証・ヘッダーが必要なサービスは外部の提供元を登録してください。内容を固定する場合は Resource を `refreshed` に設定し、`monika refresh URL` で採用できます。

### Git

```json
{"extension":"code","resource":"git:src/main.rs?rev=HEAD","query":{"symbol":"main"}}
```

ファイル名はプロジェクトルート相対の URI path、`rev` は Git のコミット指定です。省略値は `HEAD`。空白や `?`・`#` は percent-encoding します。先にコミットを確定し、その通常ファイルの blob を読みます。作業ツリーと index を変更しません。存在しないパスと、不正な revision・取得失敗を区別します。symlink・submodule・directory は通常ファイルとして解釈しません。

## 引用用 Capture の保持

引用元を表す `anchor` から、元の RegionSelector と Capture ID を持つ端点へ Link を張れます。`anchor` 自体は外部の内容を観測しません。

```json
{
  "id":"report-quote",
  "source":{"extension":"anchor","query":"report:conclusion"},
  "target":{"extension":"markdown","resource":"design.md","query":{"section":"根拠"}}
}
```

これを Sidecar に保存して `monika capture --link report-quote --endpoint target` を実行すると、引用内容を Capture に固定して Link に記録します。Sidecar から参照している Capture は GC で保持されます。`anchor` の名前は、それ自体で文書の存在や内容を保証するものではありません。

## 期待値の検査

`assert` を使うときは、設定に `"checks":[{"extension":"assert"}]` を追加します。初期設定では注釈検査を有効にしていません。次は `annotation` の例です。

```json
{"expect":[{"endpoint":"target","pointer":"/values/0","equals":42}]}
```

`endpoint` は `source` または `target`、`pointer` は Region の `content` を基準とする JSON Pointer です。等しければ `pass`、異なるか path が存在しなければ `fail`。JSON の `null` と path の不在を区別します。条件の形式が誤っていればエラーです。`expect` がなければ `not_applicable` になります。項目名の変更は `checks` の `options` に `{"field":"項目名"}` を指定します。

## 実行できる操作例

```sh
python3 documentation/examples/bundled_demo.py --monika ./monika
```

Release を展開した場所で実行します。Python 3.9 以降が必要です。一時プロジェクトで選択・比較法則・Capture・注釈検査・GC・refresh・プロジェクト移動を試します。ソース checkout では `python3 examples/bundled_demo.py --monika target/debug/monika` を使えます。
