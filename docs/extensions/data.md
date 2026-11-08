# データ形式の同梱拡張

`json`・`jsonl`・`yaml`・`toml`・`csv`・`tsv` は、Core が取得した Resource の固定済み bytes を UTF-8 として解釈し、Region を返します。いずれも revision は `1`、操作は `resolve` と `compare` です。拡張自身は元のファイルを読み直しません。Resource の取得方法と登録は [外部拡張 API](../../EXTENSIONS.md) を参照してください。

RegionSelector の例です。

```json
{"extension":"json","resource":"data.json","query":{"pointer":"/items/0/name"}}
```

`resource` には対象の Resource を指定します。`query` の省略・`null`・`{}` は全体を選びます。各 schema にないフィールド、不正な型、範囲外の index はエラーです。整数を指定する場所では負数・真偽値・小数・数値文字列を受け付けません。入力全体を検証するため、選択外の構文エラーも報告します。

## JSON・YAML・TOML の query

```json
{}
```

```json
{"pointer":"/items/0/name"}
```

フィールドは `pointer` だけです。省略時は空文字列です。RFC 6901 の JSON Pointer を各形式の object/table/mapping と array/sequence に適用します。

| pointer | 選択 |
| --- | --- |
| `""` | 根の値全体 |
| `"/items/0"` | `items` 配列の先頭要素 |
| `"/a~1b"` | `a/b` というキー |
| `"/~0key"` | `~key` というキー |
| `"/"` | 空文字列のキー |

`~0` と `~1` を一度だけ復号します。`~2` や末尾の `~` はエラーです。URI fragment の `#/...` は受け付けず、`%2F` などを percent decode しません。配列 index は `0` または先頭に `0` のない十進数字です。`01`・`+1`・`-1`・`-` は配列 index に使えません。object のキー `"01"` には `"/01"` でアクセスできます。

存在する `null` は正常な値です。存在しないキー、範囲外の index、scalar の内部への指定はエラーです。空の object・array は、その container を選んだ Region になります。

### JSON と YAML の content

```json
{"values":[{"name":"雪","enabled":true}]}
```

`values` は選択した部分木の配列です。pointer 選択では常に一要素です。`null` を選ぶと `{"values":[null]}` になります。元のコメント、空白、キー順、pointer、source offset は含みません。

JSON は整数・小数・指数の数値表現を任意精度で保持します。例えば `123456789012345678901234567890` を浮動小数点数へ変換しません。復号後に同じになる重複キーはエラーです。JSON の入れ子は 128 段未満に制限します。

YAML は一つの document を扱います。mapping のキーは文字列、mapping 内のキーは一意です。sequence・mapping・文字列・真偽値・有限の数値・null と、これらへの通常の anchor/alias を扱います。空の YAML は null です。

YAML の scalar は `serde_yaml_ng 0.10` の型の解釈を基礎にし、通常の整数・浮動小数点数の範囲を超える数値も任意精度で扱います。例えば `true` は真偽値、`yes` と `0123` は文字列です。数値は元の表記から JSON の任意精度数値に変換し、長い小数・大きな整数・大きな指数を丸めません。`0xFF`・`0o77`・`0b11` は十進数、`.5` は `0.5` に正規化します。引用符や block scalar 内の数値に見える文字列は文字列のまま保持します。

文字列以外のキー、重複キー、merge key `<<`、明示的な tag、複数 document、非有限の `.nan`・`.inf` はエラーです。標準 tag の `!!str` なども明示的な tag として扱います。引用符の外で空白・構造記号の直後に現れる `!` は tag 用の記法として予約しています。`hello !world` などの文字列は引用符で囲みます。引用符内や block scalar 内の `!` は文字列の一部です。

JSON 通信で意味が変わることを避けるため、選んだ mapping に `$serde_json::private::Number` または `$serde_json::private::RawValue` というキーが含まれる場合は明示的なエラーになります。この制約は JSON・JSONL・YAML・TOML に共通です。そのキーの値自体を pointer で選ぶことはできます。

### TOML の content

TOML の各値は `{"type":型名,"value":値}` です。日時と文字列、整数と浮動小数点数、通常の table と型表現を明確に区別します。

| `type` | `value` |
| --- | --- |
| `string` | 文字列 |
| `boolean` | 真偽値 |
| `integer` | 符号付き 64-bit 整数 |
| `float` | TOML parser が解釈した有限の binary64 数値 |
| `datetime` | TOML parser が正規化した日時・日付・時刻の文字列 |
| `array` | 型付きの値の配列 |
| `table` | キーから型付きの値への object |

例えば `stamp = 1979-05-27T07:32:00Z` の `/stamp` を選ぶと次を返します。

```json
{"values":[{"type":"datetime","value":"1979-05-27T07:32:00Z"}]}
```

offset datetime、local datetime、local date、local time の各値は、それぞれの要素と offset の有無を文字列表現に保ちます。日時の表記の正規化に TOML parser の `Display` を使います。日時を string や table として読み替えません。pointer は元の TOML の構造をたどります。content 内の `type`・`value` は pointer の対象になりません。

空の TOML は空の table です。構文エラー・重複定義・整数の範囲外・`nan`・`inf` はエラーです。

## JSONL の query

JSONL は物理的な各行を一つの JSON 値として読み、レコード配列を作ります。object だけでなく array・数値・文字列・真偽値・null もレコードです。LF と CRLF、最終行の改行の有無を扱います。途中・末尾の空行や空白だけの行、壊れた行、行内の重複キーはエラーです。正常な最終レコードの後の改行一つは空行を作りません。

利用できる query は次のいずれかです。

```json
{}
```

```json
{"pointer":"/0/name"}
```

```json
{"lines":[1,3,4]}
```

`pointer` はレコード配列を根として扱い、array index は **0 始まり**です。`lines` は物理行番号の配列で、**1 始まり**です。同時指定はエラーです。行番号は重複を除いて昇順に扱い、結果の `values` は元の行順に並びます。`{"lines":[1]}` と `{"pointer":"/0"}` は同じ identity と content を返します。

```json
{"values":[{"name":"a"},{"name":"c"}]}
```

`{"lines":[]}` は空の Region で、content は `{"values":[]}` です。空の Resource にはレコードがありません。空の Resource の全体選択は、空のレコード配列という根を選ぶため `{"values":[[]]}` です。範囲外の行指定は空結果として扱わず、エラーにします。

## CSV・TSV の query

CSV の delimiter は comma、TSV は tab です。どちらも二重引用符による quoting と `""` による引用符の escape、引用符内の delimiter・改行を扱います。全行でフィールド数がそろっている必要があります。改行は LF または CRLF です。

```json
{
  "headers": true,
  "rows": [0,2],
  "columns": ["name","score"],
  "where": [{"column":"status","equals":"ok"}]
}
```

| フィールド | 型・省略値 | 意味 |
| --- | --- | --- |
| `headers` | boolean、既定 `true` | 最初の論理レコードをヘッダーとして扱う |
| `rows` | 非負整数の配列、省略時は全データ行 | ヘッダーを除いた **0 始まり**の論理行番号 |
| `columns` | 非負整数または列名の配列、省略時は全列 | **0 始まり**の列番号、またはヘッダーの完全一致名 |
| `where` | 条件 object の配列、省略時は条件なし | `rows` の候補にすべての条件を適用する |

`headers:true` ではヘッダーはデータ行に数えません。空でない一意のヘッダー名が必要です。ヘッダーのない表は `headers:false` を明示します。この場合は先頭レコードも row `0` に入り、列は番号で指定します。空の Resource はヘッダーを持たない空の表として扱います。

行番号は物理行番号ではありません。引用符内に改行を持つレコードも一行と数えます。列名と値の大文字・小文字や空白はそのまま扱い、trim しません。行・列の重複指定は除き、列名と番号が同じ列を指す場合も一列になります。結果は指定配列の順序によらず元の行順・列順です。

条件の schema は次のとおりです。両方のフィールドが必須です。

```json
{"column":"status","equals":"ok"}
```

`column` は `columns` と同じ列名または番号、`equals` は文字列です。セル値の文字列と完全一致で比較します。`"01"` と `"1"` は異なります。複数条件は AND、空の条件配列は条件なしです。結果に含めない列でも条件に使えます。存在しない列や誤った条件は、候補行が空でもエラーです。

例えば、次の表から `{"columns":["name"],"where":[{"column":"status","equals":"ok"}]}` で選択できます。

```csv
name,status
a,ok
b,pending
c,ok
```

```json
{"columns":["name"],"rows":[["a"],["c"]]}
```

全セルの値は文字列です。`headers:false` の非空結果は `{"columns":null,"rows":[["a","ok"]]}` の形になります。content に元の行番号・列番号・byte offset は含めません。filter に合わない行を途中に挿入しても、選択結果の値と順序が同じなら content は同じです。行番号で指定した場合は、その番号が別のデータを指す変更が content に反映されます。

`rows:[]`、`columns:[]`、条件に合う行がゼロの場合は、いずれも空のセル集合です。content は共通して `{"columns":[],"rows":[]}` です。空の Resource やヘッダーだけの表も同じ空結果になります。ヘッダーだけを選ぶ操作はなく、ヘッダー名は選択されたデータセルのラベルとして content に含めます。

空行はエラーです。一列だけの空文字列レコードは `""` と記述します。複数列の空セルは `,`、`a,` などで表せます。閉じていない引用符、引用符の外での不正な `"`、閉じ引用符の後の余計な文字、引用符の外での単独 CR、UTF-8 BOM、行ごとの列数の不一致はエラーです。引用符内の改行は内容をそのまま保持します。

## identity と compare

| 拡張 | domain |
| --- | --- |
| `json` | `monika.json.v1` |
| `jsonl` | `monika.jsonl.v1` |
| `yaml` | `monika.yaml.v1` |
| `toml` | `monika.toml.v1` |
| `csv` | `monika.csv.v1` |
| `tsv` | `monika.tsv.v1` |

構造化データの identity は `{"observation":"ID","paths":[["items","0"]]}` です。各 path は復号済みの token 配列です。根は `[]`、根だけを選ぶと `paths:[[]]`、空の選択は `paths:[]` です。複数 path は辞書順に正規化します。祖先の部分木は子孫の部分木を含みます。`/a` と `/ab` のように文字列の接頭辞だけが一致するキーは包含になりません。JSONL の複数行選択は複数の部分木として比較し、選択したレコード内への pointer を含みます。

表の identity は `{"observation":"ID","headers":true,"cells":[[0,0],[2,0]]}` です。`cells` はデータ行番号と列番号の組を昇順で重複なく並べた集合です。同じセルを選ぶ query は、列名・列番号・filter などの指定方法によらず同じ Region になります。空のセル集合は `cells:[]` にそろいます。

比較には同じ domain と Observation を使います。表では `headers` も同じ解釈である必要があります。同じ集合は `equal`、左が右を真に含むと `contains`、左が右に真に含まれると `within`、それ以外は `incomparable` です。部分的な重なりも、どちらも他方を含まない場合は `incomparable` です。空の選択は同じ Observation・解釈の全選択に含まれ、空同士は `equal` です。異なる Observation または表の headers mode は `incomparable`、異なる domain はエラーです。

Capture の変化判定は `content` に対して行います。同じ値を持つ異なる位置は、content が同じでも別の Region です。全体選択では入力値全体、部分選択ではその部分の値と、表なら選択列のヘッダー名が変化判定の対象になります。
