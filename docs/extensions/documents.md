# text・markdown・html

三つの拡張は、Core が渡す Observation の UTF-8 bytes から Region を解決します。Resource の取得方法は提供元が担当するため、ローカルファイル以外にも同じ query を使えます。元の Resource を読み直すことはありません。

RegionSelector の例です。`resource` は必須で、`query` を省略するか `null` にすると全体を選択します。

```json
{"extension":"text","resource":"notes.txt","query":{"lines":[2,5]}}
{"extension":"markdown","resource":"design.md","query":{"section":["設計","検証"]}}
{"extension":"html","resource":"page.html","query":{"css":"main > article"}}
```

`revision` はいずれも `"1"`、操作は `resolve` と `compare` です。`monika extension show text` などで設定済みの拡張の概要を確認できます。以下の例は `query` 部分です。未知のフィールド、異なる選択方式の混在、誤った型をエラーにします。UTF-8 以外の入力もエラーです。

既存のプロジェクトへの登録と読み取りの例です。file の提供元をすでに別の拡張で登録している場合は、登録コマンドから `file` を省きます。

```sh
monika extension enable file text markdown html
monika read <<'JSON'
{"extension":"markdown","resource":"design.md","query":{"section":["設計","検証"]}}
JSON
```

## text

| query | 選択 |
| --- | --- |
| `null` | 全文 |
| `{"lines":[2,5]}` | 2 行目から 5 行目まで。両端を含む |
| `{"text":"重要な記述"}` | 文字列の一致箇所。一致が複数なら曖昧さを報告 |
| `{"text":"重要な記述","occurrence":2}` | 文書の先頭から数えて 2 番目の一致 |
| `{"text":"重要な記述","occurrence":"all"}` | すべての一致箇所の和集合 |

行番号は 1 から数えます。LF、CRLF、単独の CR を改行として扱い、選択した最終行の改行も含めます。末尾の改行の後に空の行を追加しません。空の Resource は 0 行です。開始番号が終了番号を超える指定や、存在しない行の指定はエラーです。

`text` は大文字・小文字を区別するリテラルです。正規表現や Unicode 正規化は適用しません。空文字列はエラーです。一致が重なる場合も別の出現として数えます。例えば `あああ` で `ああ` を探すと 2 箇所が一致し、`occurrence:2` は後ろの 2 文字、`occurrence:"all"` は 3 文字全体を選びます。

一致が 0 件なら、`occurrence` の省略または `"all"` は正常な空の Region を返します。番号を明示したときは、その番号の一致が存在することを要求します。`occurrence:0`、負数、小数、`null`、範囲外の番号はエラーです。

## markdown

CommonMark parser が認識する見出しとコードフェンスを扱います。ATX 見出し（`# タイトル`）と Setext 見出し（下線が `===` または `---`）に対応します。コードブロックや HTML block 内の見出しらしい文字列は見出しになりません。

| query | 選択 |
| --- | --- |
| `null` | 元の Markdown 全文 |
| `{"section":"検証"}` | 文書中の「検証」という見出しから始まる節 |
| `{"section":"検証","occurrence":2}` | 文書中で 2 番目の「検証」の節 |
| `{"section":"検証","occurrence":"all"}` | 同名の節すべての和集合 |
| `{"section":["設計","検証"]}` | 最上位の「設計」の直下にある「検証」の節 |
| `{"heading":"検証"}` | 見出し自体の元の Markdown |
| `{"heading":["設計","検証"]}` | path で指定した見出し自体 |
| `{"code":{}}` | すべての fenced code block |
| `{"code":{"language":"rust"}}` | info string の先頭 token が `rust` の fenced code block すべて |
| `{"code":{"language":"rust","occurrence":2}}` | 該当する 2 番目の fenced code block |

節は見出しの開始位置から、同じか小さい見出しレベルの次の見出しの直前までです。下位の見出しと本文を含み、次の見出しがなければ文書末尾までになります。見出しのない前書きは全文の選択に含まれます。

### 見出し名と path

見出し名には、parser が解釈した text と inline code を使います。強調やリンクの構文は取り除き、画像は代替テキスト、文字参照とエスケープは解釈後の文字として扱います。inline HTML tag 自体は名前に含めず、改行は空白一つにします。例えば ``## *設計* &amp; `検証` `` の名前は `設計 & 検証` です。元の記法は返される `content` に残ります。名前の比較は大文字・小文字を区別し、空の見出しは `""` で指定できます。

文字列による指定は文書全体を検索します。配列による path は見出しの階層を先頭からたどります。最初の要素は最上位の見出し、その後は直下の見出しを指します。レベルを飛ばした `# 親` → `### 子` も親子です。引用やリスト内で parser が認識した見出しも、文書の出現順とレベルによるこの階層に含めます。

同名の見出しは、path の各段階で出現番号を指定できます。番号はその親の直下にある、同名の見出しの中で 1 から数えます。

```json
{
  "section": [
    {"title":"設計","occurrence":2},
    {"title":"検証","occurrence":1}
  ]
}
```

文字列の要素は `{"title":"名前"}` と同じです。同名候補が複数ある段階では番号が必要です。path の要素では `occurrence:"all"` を使えません。path 全体に `occurrence` を追加することもできません。

見出しが見つからなければ正常な空の Region です。出現番号を指定した段階でその番号の候補が存在しない場合はエラーです。先行する path 要素が見つからない場合は空の Region になりますが、後続要素の型や未知フィールドも検査します。空の path はエラーです。

### コードフェンスと範囲

backtick と tilde の fenced code block を選択できます。`language` は info string の最初の空白区切り token と完全一致で比較します。`"rust ignore"` の token は `rust` です。`language:""` は info string のないフェンス、`language` の省略はすべてのフェンスを選びます。`occurrence` は省略すると `"all"` と同じです。番号の規則は text と共通です。インデントによる code block はこの選択に含みません。

見出し・フェンスの範囲は parser の元文書上の位置を使います。フェンスは開始・終了の区切りと info string を含み、終了フェンスの後の改行は含みません。終了フェンスがなければ CommonMark に従って文書末尾までを code block として扱います。引用・リストなどの外側の構文については parser が返す位置を使うため、先頭の `> ` やリスト記号が範囲の外になる場合があります。

## text・markdown の identity と変化

両者は `monika.bytes.v1` domain を使います。例えば同じ Observation の Markdown の節を選ぶ指定と、その節に一致する text の行指定は `equal` になります。同じ domain を扱う code 拡張とも範囲で比較できます。

```json
{
  "domain":"monika.bytes.v1",
  "identity":{"observation":"Observation-ID","ranges":[[10,25],[40,50]]},
  "content":{"text":"選択された元の文字列"}
}
```

`ranges` は UTF-8 bytes の半開区間です。開始位置は含み、終了位置は含みません。文書順に並べ、重複・重なり・隣接を統合します。空の選択は `ranges:[]` と `content:{"text":""}` です。`content.text` は統合した範囲の元の文字列を、区切りを挿入せず連結します。改行、空白、Markdown の構文も選択範囲内のまま残ります。

同じ Observation では範囲の集合が同じなら `equal`、左が右を真に含めば `contains`、逆なら `within` です。一部分だけが重なる場合と、離れている場合はどちらも `incomparable` です。空の Region は同じ Observation のすべての Region に含まれます。異なる Observation 間は `incomparable` です。

Capture の変化検査は `content` を比較します。選択箇所の外側に文章を足して位置がずれても、選択された元の文字列が同じなら変化しません。選択内部の空白や記法の変更は変化になります。複数範囲の連結結果が同じなら、範囲の配置が変わっても content は同じです。

## html

HTML5 document parser で作った DOM に CSS selector を適用します。`html`・`head`・`body`・`tbody` の補完や省略終了タグの処理、table 内の要素の移動など、HTML5 の定める補正も解釈に含めます。整形式性を検査する拡張ではありません。全文と部分選択は常に同じ document モードで解析します。

| query | 選択 |
| --- | --- |
| `null` | Document node 全体。DOCTYPE や文書直下のコメントも含む |
| `{"css":"article > p"}` | 一致するすべての要素の subtree の和集合 |
| `{"css":"article > p","occurrence":2}` | DOM 順序で 2 番目に一致する要素の subtree |
| `{"css":"#a, #b","occurrence":"all"}` | 一致する subtree すべて。省略時と同じ |
| `{"css":"#a\\:b"}` | `id="a:b"` の要素 |
| `{"css":"[data-label=\"日 本\"]"}` | 指定した属性値を持つ要素 |

CSS は scraper が扱う selector を使えます。要素名、ID、class、属性、子・子孫・兄弟関係、`:nth-child`、`:not`、`:is`、`:has` などで指定します。CSS group の同じ要素への重複一致は一件として数えます。同じ ID を持つ要素が複数あれば、それぞれが一致します。番号は group の記述順やソースの要素作成順ではなく、解析後の DOM 順序です。出現番号は subtree の重複統合より前に適用します。

空の CSS 文字列、構文エラー、未対応の selector（例えば `:hover` や `::before`）、未知のフィールドはエラーです。一致がない有効な CSS は正常な空の Region です。`occurrence` に明示した番号が存在しない場合はエラーです。JavaScript は実行せず、外部 stylesheet や iframe の Resource は取得しません。

### DOM の identity と包含

domain は `monika.html.v1` です。選択した要素は、その要素自身と子孫の DOM nodes 全体を表します。祖先と子孫の両方に一致した場合は祖先の subtree にまとめます。例えば `article` と `article, article p` は同じ Region です。子要素を全部選んでも、親要素自身を含む subtree と同一にはなりません。

```json
{
  "domain":"monika.html.v1",
  "identity":{"observation":"Observation-ID","roots":[[1,1,0],[1,1,2]]},
  "content":{"html":["<article>...</article>","<aside>...</aside>"]}
}
```

`roots` の各配列は Document node からたどる子 node の番号の path です。番号は 0 から始まり、text、comment、DOCTYPE なども数えます。配列は DOM 順序に並び、同じ node と祖先・子孫の組を重複して保持しません。空の path `[]` は Document node 自身なので、全文の `roots` は `[[]]` です。空の選択は `roots:[]` です。

同じ Observation では、subtree の和集合が同じなら `equal`、左が右の全 subtree を真に含めば `contains`、逆なら `within` です。親と子で root の path が異なっていても包含を判定できます。互いに一部だけを共有する集合は `incomparable`、空の Region はすべての Region に含まれます。異なる Observation は `incomparable`、異なる domain の比較はエラーです。

`null` による Document node の選択は `html` 要素よりも外側を含みます。空の入力も HTML5 document として解析するため、`null` は補完された `<html><head></head><body></body></html>` を含む Region になります。

### content と Capture

`content.html` は統合後の subtree を DOM 順序で並べた outer HTML の配列です。空の選択は空配列です。選択した要素自身のタグ・属性、子孫、text、コメントを含みます。全文では Document のシリアライズ結果一つを入れます。元の HTML bytes ではなく、parser が構成した DOM をシリアライズします。

選択外に兄弟要素を追加して path が変わっても、選択された outer HTML が同じなら Capture の内容は変化しません。選択された要素の属性や子孫の内容の変更は変化になります。タグ名の大文字・小文字、属性値の引用符、文字参照の書き方など、同じ DOM に正規化される記法の違いは content に現れない場合があります。元の bytes の違いを検査したい場合は text を使います。
