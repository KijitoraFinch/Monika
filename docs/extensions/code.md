# code：構文に基づくコードの選択

`code` は Tree-sitter 0.25 で UTF-8 の Resource を解析し、宣言や任意の構文を Region として選択します。Core が渡す Observation の bytes を読み、Resource のファイルを直接開きません。`revision` は `1`、操作は `resolve`・`compare` です。

`monika init` の既定構成に含まれます。既存の構成には `monika extension enable file text code` で追加できます。`file` はローカル Resource の観測、`text` は共有 domain の既定の比較を担当します。

```sh
printf '%s\n' '{"extension":"code","resource":"src/main.rs","query":{"symbol":"main"}}' | monika read
```

## 対応言語

`language` を省略すると Resource の拡張子から grammar を選びます。URI は path の拡張子を使います。拡張子のない Resource、未知の拡張子、`.h` は `language` を指定してください。内容や shebang からの推測は行いません。

| `language` | 自動判定する拡張子 | 主な宣言と `kind` |
| --- | --- | --- |
| `rust` | `.rs` | 関数 `function`、impl・trait 内の関数 `method`、`module`、`struct`、`enum`、`trait`、`impl`、`type`、`constant`、`static`、`macro`、`union` |
| `python` | `.py`、`.pyi` | 関数 `function`、class 内の関数 `method`、`class`。async・decorator に対応 |
| `javascript` | `.js`、`.jsx`、`.mjs`、`.cjs` | `function`、`class`、`method`。generator、変数に直接束縛した arrow function・function expression・class expression、関数を直接束縛した class field に対応 |
| `typescript` | `.ts`、`.mts`、`.cts` | JavaScript と同じ形式、および `interface`、`enum`、`type`、namespace・module の `module`、関数・method の signature |
| `tsx` | `.tsx` | TypeScript と JSX |
| `go` | `.go` | `function`、receiver を持つ `method`、`struct`、`interface`、`type`、interface 内の `method` |
| `java` | `.java` | class・record の `class`、interface・annotation type の `interface`、`enum`、`method`、`constructor` |
| `c` | `.c` | 関数定義・prototype の `function`、本体を持つ `struct`・`union`・`enum` |
| `cpp` | `.cc`、`.cpp`、`.cxx`、`.c++`、`.C`、`.hh`、`.hpp`、`.hxx`、`.h++` | C の形式、および `class`、class 内の `method`、`namespace`、using alias の `type`。template の境界に対応 |
| `ruby` | `.rb`、`.rake`、`.gemspec` | `method`、`class`、`module`。singleton method・singleton class に対応 |
| `bash` | `.sh`、`.bash` | `function`。`name() { …; }` と `function name { …; }` |

`.C` は C++ です。その他の拡張子は大文字・小文字を区別しません。`.h` は C / C++ の両方で使われるため明示指定を求めます。`.sh` は Bash grammar を使います。Zsh・Fish・PowerShell 用の grammar は含みません。

## ファイル全体

`query: null` または `{}` は、language を推定してファイル全体を選びます。明示指定だけのオブジェクトも使えます。

```json
{"extension":"code","resource":"include/worker.h","query":{"language":"cpp"}}
```

全体選択でも構文を解析します。空ファイルは空 Region になります。

## 宣言を名前で選択する

```json
{"extension":"code","resource":"src/worker.py","query":{"symbol":"run","scope":["Worker"],"kind":"method"}}
```

| 項目 | 意味 |
| --- | --- |
| `language` | 任意。上表の language 名を正確に指定 |
| `symbol` | 必須。宣言名の完全一致。コメント・文字列中の名前は対象外 |
| `scope` | 任意。外側から順に並べた、親宣言名の配列。指定すると全要素が一致する宣言だけを選択。`[]` は親宣言がない位置 |
| `kind` | 任意。上表の kind で絞り込み |
| `occurrence` | 任意。絞り込み後の候補を原文順に数えた **1 始まり**の正の整数。`1` が最初の候補 |

`scope` を省略すると全 scope を検索します。候補が一つならそれを選びます。候補が複数ある場合はエラーにし、scope・kind・行番号を最大 8 件示します。`scope`、`kind`、`occurrence` で絞り込んでください。候補がない場合と、`occurrence: 0` や候補数を超える指定もエラーです。`occurrence` は宣言の並べ替えによって別の対象を指し得ます。

Rust の module と impl を経由する例です。

```json
{"extension":"code","resource":"src/worker.rs","query":{"symbol":"run","scope":["app","Worker"],"kind":"method"}}
```

同じ型の inherent impl と trait impl に同名宣言がある場合、`scope` も同じになり得ます。`occurrence` または raw query で選択します。impl の名前は型の構文から得て、最外側の generic 引数・pointer・reference は取り除きます。型 alias の解決はしません。

Go の receiver は scope に入ります。`func (w *Worker[T]) Run()` の scope は `["Worker"]` です。Ruby の `class Worker` 内の `def self.run` と `class << self` 内の `def run` は、ともに scope `["Worker","self"]` になります。C++ の `namespace A::B`、TypeScript の `namespace A.B` は scope の複数の要素として扱います。

名前は構文中の表記を使います。たとえば JavaScript の `#private` は `"symbol":"#private"`、文字列で書いた method 名は引用符も含めて指定します。Unicode の識別子も元の表記で一致させます。

### 宣言の境界

宣言ノード全体を選び、構文上付随する要素も含めます。

- Python の decorator、Rust の外側の attribute・doc comment、Java の annotation、JavaScript / TypeScript の decorator を含みます。
- `export`、TypeScript の `declare`、C++ の template 宣言は、その宣言を直接包む場合に含みます。
- JavaScript / TypeScript の `const run = () => …;` は、その文が一つの変数宣言だけなら `const` と末尾の `;` を含みます。一文に複数の変数宣言がある場合は、該当する declarator だけを選びます。
- Go の type 宣言は、その宣言に type が一つだけなら `type` を含みます。
- C / C++ の関数 prototype は宣言文全体です。一文で複数の関数を宣言すると、それぞれの選択が同じ Region になります。関数ポインタ変数は関数として選びません。
- class / struct の末尾の `;` など、grammar が宣言ノードの外に置く区切りは含まれない場合があります。一般の前置コメントと前後の空白は宣言に含めません。含めた attribute・decorator と宣言の間の原文は保持します。

## Tree-sitter query と capture

任意の構文を選ぶ場合は、`query` に Tree-sitter query、`capture` に返す capture 名を指定します。`symbol` の指定とは組み合わせません。

```json
{
  "extension": "code",
  "resource": "src/worker.rs",
  "query": {
    "query": "((function_item name: (identifier) @name) @decl (#eq? @name \"run\"))",
    "capture": "decl"
  }
}
```

`capture` は `@` を付けない名前です。同じ名前の capture が複数あれば、すべての区間の集合を返します。重複・包含・隣接する区間は共有の byte span 処理で統合し、原文順に並べます。capture のノードそのものを選ぶため、宣言選択で行う decorator 等の境界拡張は行いません。

宣言の名前だけを選ぶ例です。

```json
{"extension":"code","resource":"src/worker.py","query":{"query":"(function_definition name: (identifier) @name)","capture":"name"}}
```

grammar のノード名・field 名を使います。[Tree-sitter query の構文](https://tree-sitter.github.io/tree-sitter/using-parsers/queries/1-syntax.html)と、各 language crate に含まれる `node-types.json` を参照してください。Rust binding が評価する `#eq?`・`#match?`・`#any-of?` とその否定・`any-` 変種を利用できます。任意の独自 predicate、`#is?`、`#is-not?`、`#set!` は明示的なエラーにします。

有効な query と capture で一致がない場合は、正常な空 Region を返します。query の構文エラー、grammar にないノードや field、存在しない capture 名、query の match limit 超過はエラーです。

## Region と比較

domain は text・markdown と共通の `monika.bytes.v1` です。

```json
{
  "domain": "monika.bytes.v1",
  "identity": {"observation": "Observation の ID", "ranges": [[12, 40]]},
  "content": {"text": "選択した原文"}
}
```

区間は UTF-8 の byte offset による半開区間です。複数区間の `content.text` は区切りを追加せず、選択した原文を順につなぎます。空 Region の `ranges` は `[]`、`text` は空文字列です。Core が使った Observation の ID を `observations` に付けます。

同じ Observation 上の byte 集合で `equal`・`contains`・`within`・`incomparable` を判定します。一部だけ重なる集合は `incomparable` です。異なる Observation は `incomparable` となります。code で選択した関数と text で選択した同じ区間は `equal` になります。

Capture の変化検査に用いる `content` は原文だけです。構文エラーを導入しない選択外の編集や、宣言の内容を保った移動では、byte offset が変わっても `content` は変わりません。attribute・decorator など選択に含む内容の変更は検出します。

## 解析の範囲とエラー

これは **構文に基づく選択**です。LS を起動せず、型検査、名前解決、import 先の探索、動的な method 定義の追跡、macro 展開、C/C++ の preprocessing は行いません。条件付きコンパイルの分岐は、原文に存在する構文として扱います。C++ の class 外に記述した関数定義・prototype は、修飾名があっても `kind: "function"` とします。C++ の型や namespace を意味解析して区別することはしません。

宣言選択は上表の名前を持つ構文を対象とします。C/C++ の型の前方宣言・typedef、無名宣言、JavaScript の destructuring・計算された property 名、任意の代入式から作る関数など、宣言選択が扱わない構文は raw query で指定できます。オブジェクト変数の名前や無名 block は `scope` の要素にはなりません。別々の無名 scope で同じ名前になった場合も、候補を勝手に一つに決めず曖昧さを報告します。

ファイル全体に Tree-sitter の `ERROR` または欠けた構文要素がある場合は、選択外でも失敗します。診断には Resource、1 始まりの行番号・byte 列番号を含みます。grammar が受け付けることは、コンパイラで有効なコードであることまでは保証しません。

`query` は `null` または記載した形のオブジェクトです。未知の項目、誤った型、空の文字列、0・負数・小数の `occurrence`、選択方式の混在はエラーになります。任意項目は省略で表し、`null` を代用しません。
