//! UTF-8 text, CommonMark, and HTML Regions. Public query syntax is documented in
//! `docs/extensions/documents.md`.

use crate::extensions::process::Manifest;
use crate::model::{Observation, Region, RegionSelector, Relation};
use anyhow::{Context, Result, bail, ensure};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

const HTML_DOMAIN: &str = "monika.html.v1";

pub fn manifest(name: &str) -> Option<Manifest> {
    let (domain, description) = match name {
        "text" => (
            "monika.bytes.v1",
            "UTF-8 text. query: null (whole), {\"lines\":[1,3]} (inclusive), or \
             {\"text\":\"literal\",\"occurrence\":2}; occurrence may be \"all\", otherwise \
             multiple matches need an explicit occurrence. content: {\"text\":\"original slices\"}.",
        ),
        "markdown" => (
            "monika.bytes.v1",
            "CommonMark. query: null (whole), {\"section\":\"Title\"}, \
             {\"section\":[\"Parent\",{\"title\":\"Child\",\"occurrence\":2}]}, \
             {\"heading\":\"Title\"}, or {\"code\":{\"language\":\"rust\"}}. \
             Named matches must be unique unless occurrence is specified; sections include \
             subheadings. content: {\"text\":\"original slices\"}.",
        ),
        "html" => (
            HTML_DOMAIN,
            "UTF-8 HTML5 DOM. query: null (document), {\"css\":\"article > p\"} (all \
             matches), or {\"css\":\"p\",\"occurrence\":2} (DOM order, 1-based). \
             HTML5 parsing includes its specified error recovery. content: \
             {\"html\":[\"serialized outer HTML\"]}; ancestor selections absorb descendants.",
        ),
        _ => return None,
    };
    Some(Manifest {
        name: name.into(),
        revision: "1".into(),
        operations: vec!["resolve".into(), "compare".into()],
        domains: vec![domain.into()],
        description: description.into(),
    })
}

pub fn resolve(
    name: &str,
    selector: &RegionSelector,
    observation: &Observation,
    bytes: &[u8],
) -> Result<Value> {
    ensure!(
        selector.extension == name,
        "selector extension must be {name}"
    );
    let source = std::str::from_utf8(bytes).context("document Resource must be UTF-8")?;
    match name {
        "text" => super::span_region(observation, bytes, text_ranges(&selector.query, source)?),
        "markdown" => super::span_region(
            observation,
            bytes,
            markdown_ranges(&selector.query, source)?,
        ),
        "html" => html_region(&selector.query, observation, source),
        _ => bail!("unknown document extension: {name}"),
    }
}

pub fn compare(name: &str, left: &Region, right: &Region) -> Result<Relation> {
    match name {
        "text" | "markdown" => super::compare_spans(left, right),
        "html" => compare_html(left, right),
        _ => bail!("unknown document extension: {name}"),
    }
}

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .with_context(|| format!("{label} must be an object"))
}

fn fields(object: &Map<String, Value>, allowed: &[&str], label: &str) -> Result<()> {
    for key in object.keys() {
        ensure!(
            allowed.contains(&key.as_str()),
            "unknown {label} field: {key}"
        );
    }
    Ok(())
}

fn string<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    value
        .as_str()
        .with_context(|| format!("{label} must be a string"))
}

fn positive_index(value: &Value, label: &str) -> Result<usize> {
    let n = value
        .as_u64()
        .with_context(|| format!("{label} must be a positive integer"))?;
    ensure!(n > 0, "{label} is 1-based; zero is invalid");
    usize::try_from(n).with_context(|| format!("{label} is too large"))
}

#[derive(Clone, Copy)]
enum Occurrence {
    Unique,
    All,
    Index(usize),
}

impl Occurrence {
    fn parse(value: Option<&Value>, default: Self, allow_all: bool) -> Result<Self> {
        match value {
            None => Ok(default),
            Some(Value::String(s)) if allow_all && s == "all" => Ok(Self::All),
            Some(value) => Ok(Self::Index(positive_index(value, "occurrence")?)),
        }
    }

    fn select<T>(self, matches: Vec<T>, label: &str) -> Result<Vec<T>> {
        match self {
            Self::All => Ok(matches),
            Self::Unique => {
                ensure!(
                    matches.len() <= 1,
                    "ambiguous {label}: {} matches; specify occurrence (1-based)",
                    matches.len()
                );
                Ok(matches)
            }
            Self::Index(n) => {
                ensure!(
                    n <= matches.len(),
                    "{label} occurrence {n} is out of range ({} matches)",
                    matches.len()
                );
                Ok(matches.into_iter().skip(n - 1).take(1).collect())
            }
        }
    }
}

fn text_ranges(query: &Value, source: &str) -> Result<Vec<(usize, usize)>> {
    if query.is_null() {
        return Ok(vec![(0, source.len())]);
    }
    let query = object(query, "text query")?;
    if let Some(lines) = query.get("lines") {
        fields(query, &["lines"], "text query")?;
        let pair = lines.as_array().context("lines must be [start, end]")?;
        ensure!(pair.len() == 2, "lines must be [start, end]");
        let start = positive_index(&pair[0], "line start")?;
        let end = positive_index(&pair[1], "line end")?;
        ensure!(start <= end, "line start must not exceed line end");
        let lines = line_ranges(source);
        ensure!(
            end <= lines.len(),
            "line {end} is out of range ({} lines)",
            lines.len()
        );
        return Ok(vec![(lines[start - 1].0, lines[end - 1].1)]);
    }
    fields(query, &["text", "occurrence"], "text query")?;
    let needle = string(
        query
            .get("text")
            .context("text query needs lines or text")?,
        "text",
    )?;
    ensure!(!needle.is_empty(), "text must not be empty");
    let occurrence = Occurrence::parse(query.get("occurrence"), Occurrence::Unique, true)?;
    let mut matches = Vec::new();
    let mut cursor = 0;
    // Advancing by one character preserves overlapping matches and UTF-8 boundaries.
    while let Some(offset) = source[cursor..].find(needle) {
        let start = cursor + offset;
        matches.push((start, start + needle.len()));
        cursor = start + needle.chars().next().expect("nonempty literal").len_utf8();
    }
    occurrence.select(matches, "text")
}

fn line_ranges(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\r' | b'\n' => {
                if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                    cursor += 1;
                }
                cursor += 1;
                result.push((start, cursor));
                start = cursor;
            }
            _ => cursor += 1,
        }
    }
    if start < bytes.len() {
        result.push((start, bytes.len()));
    }
    result
}

struct Heading {
    title: String,
    level: u8,
    parent: Option<usize>,
    start: usize,
    end: usize,
    section_end: usize,
}

struct Fence {
    language: String,
    start: usize,
    end: usize,
}

struct Markdown {
    headings: Vec<Heading>,
    fences: Vec<Fence>,
}

impl Markdown {
    fn parse(source: &str) -> Self {
        let mut headings: Vec<Heading> = Vec::new();
        let mut fences = Vec::new();
        let mut outline: Vec<usize> = Vec::new();
        let mut current_heading = None;
        let mut current_fence: Option<usize> = None;
        // CommonMark only: optional dialect features do not alter heading text or offsets.
        for (event, span) in Parser::new(source).into_offset_iter() {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    let level = level as u8;
                    while let Some(&previous) = outline.last() {
                        if headings[previous].level < level {
                            break;
                        }
                        headings[previous].section_end = span.start;
                        outline.pop();
                    }
                    let index = headings.len();
                    headings.push(Heading {
                        title: String::new(),
                        level,
                        parent: outline.last().copied(),
                        start: span.start,
                        end: span.end,
                        section_end: source.len(),
                    });
                    outline.push(index);
                    current_heading = Some(index);
                }
                Event::End(TagEnd::Heading(_)) => {
                    let index = current_heading.take().expect("balanced CommonMark heading");
                    headings[index].end = span.end;
                }
                Event::Text(text) | Event::Code(text) => {
                    if let Some(index) = current_heading {
                        headings[index].title.push_str(&text);
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if let Some(index) = current_heading {
                        headings[index].title.push(' ');
                    }
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                    current_fence = Some(fences.len());
                    fences.push(Fence {
                        language: info.split_whitespace().next().unwrap_or("").to_owned(),
                        start: span.start,
                        end: span.end,
                    });
                }
                Event::End(TagEnd::CodeBlock) => {
                    if let Some(index) = current_fence.take() {
                        fences[index].end = span.end;
                    }
                }
                _ => {}
            }
        }
        Self { headings, fences }
    }

    fn select_headings(&self, query: &Map<String, Value>, kind: &str) -> Result<Vec<usize>> {
        let value = &query[kind];
        if let Some(title) = value.as_str() {
            fields(query, &[kind, "occurrence"], "markdown query")?;
            let occurrence = Occurrence::parse(query.get("occurrence"), Occurrence::Unique, true)?;
            let matches = self
                .headings
                .iter()
                .enumerate()
                .filter(|(_, heading)| heading.title == title)
                .map(|(i, _)| i)
                .collect();
            return occurrence.select(matches, &format!("heading {title:?}"));
        }
        fields(query, &[kind], "markdown path query")?;
        let path = value
            .as_array()
            .with_context(|| format!("{kind} must be a title string or heading path"))?;
        ensure!(!path.is_empty(), "heading path must not be empty");
        // Validate the entire query even if an earlier path component matches nothing.
        let parts = path
            .iter()
            .map(|part| {
                if let Some(title) = part.as_str() {
                    return Ok((title, Occurrence::Unique));
                }
                let part = object(part, "heading path component")?;
                fields(part, &["title", "occurrence"], "heading path component")?;
                let title = string(
                    part.get("title")
                        .context("heading path component needs title")?,
                    "heading title",
                )?;
                Ok((
                    title,
                    Occurrence::parse(part.get("occurrence"), Occurrence::Unique, false)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut parent = None;
        for (title, occurrence) in parts {
            let matches = self
                .headings
                .iter()
                .enumerate()
                .filter(|(_, heading)| heading.parent == parent && heading.title == title)
                .map(|(i, _)| i)
                .collect();
            let selected =
                occurrence.select(matches, &format!("heading path component {title:?}"))?;
            match selected.first() {
                Some(&index) => parent = Some(index),
                None => return Ok(Vec::new()),
            }
        }
        Ok(parent.into_iter().collect())
    }
}

fn markdown_ranges(query: &Value, source: &str) -> Result<Vec<(usize, usize)>> {
    if query.is_null() {
        return Ok(vec![(0, source.len())]);
    }
    let query = object(query, "markdown query")?;
    let markdown = Markdown::parse(source);
    for kind in ["section", "heading"] {
        if query.contains_key(kind) {
            return Ok(markdown
                .select_headings(query, kind)?
                .into_iter()
                .map(|index| {
                    let heading = &markdown.headings[index];
                    (
                        heading.start,
                        if kind == "section" {
                            heading.section_end
                        } else {
                            heading.end
                        },
                    )
                })
                .collect());
        }
    }
    fields(query, &["code"], "markdown query")?;
    let code = object(
        query
            .get("code")
            .context("markdown query needs section, heading, or code")?,
        "code",
    )?;
    fields(code, &["language", "occurrence"], "code")?;
    let language = code
        .get("language")
        .map(|value| string(value, "code language"))
        .transpose()?;
    if let Some(language) = language {
        ensure!(
            !language.chars().any(char::is_whitespace),
            "code language must be a single info-string token (or empty for unlabelled fences)"
        );
    }
    let occurrence = Occurrence::parse(code.get("occurrence"), Occurrence::All, true)?;
    let matches = markdown
        .fences
        .iter()
        .filter(|fence| language.is_none_or(|language| fence.language == language))
        .map(|fence| (fence.start, fence.end))
        .collect();
    occurrence.select(matches, "fenced code block")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HtmlIdentity {
    observation: String,
    // Each path counts all child nodes, including text and comments, from the Document.
    // Paths are in document order and contain no ancestor/descendant pairs.
    roots: Vec<Vec<usize>>,
}

fn html_region(query: &Value, observation: &Observation, source: &str) -> Result<Value> {
    let selection = if query.is_null() {
        None
    } else {
        let query = object(query, "html query")?;
        fields(query, &["css", "occurrence"], "html query")?;
        let css = string(query.get("css").context("html query needs css")?, "css")?;
        ensure!(!css.trim().is_empty(), "css must not be empty");
        let css = Selector::parse(css)
            .map_err(|error| anyhow::anyhow!("invalid CSS selector: {error}"))?;
        let occurrence = Occurrence::parse(query.get("occurrence"), Occurrence::All, true)?;
        Some((css, occurrence))
    };
    // HTML5's defined tree-building recovery is part of the interpretation, including
    // implied elements and foster parenting; this is always document-mode parsing.
    let document = Html::parse_document(source);
    let mut roots = Vec::new();
    let mut html = Vec::new();
    if let Some((css, occurrence)) = selection {
        let root = document.root_element();
        // Html::select iterates allocation order, which can differ from DOM order after
        // HTML5 tree repairs. ElementRef::select traverses the actual tree.
        let matches = std::iter::once(root)
            .filter(|element| css.matches(element))
            .chain(root.select(&css))
            .collect();
        let selected: HashSet<_> = occurrence
            .select(matches, "CSS selector")?
            .into_iter()
            .map(|element| element.id())
            .collect();
        let mut frames = vec![document.tree.root().children().enumerate()];
        let mut path = Vec::new();
        while let Some(children) = frames.last_mut() {
            match children.next() {
                Some((index, node)) => {
                    path.push(index);
                    if selected.contains(&node.id()) {
                        roots.push(path.clone());
                        html.push(
                            ElementRef::wrap(node)
                                .expect("CSS matches only elements")
                                .html(),
                        );
                        // Selecting a subtree already includes every selected descendant.
                        path.pop();
                    } else {
                        frames.push(node.children().enumerate());
                    }
                }
                None => {
                    frames.pop();
                    path.pop();
                }
            }
        }
    } else {
        roots.push(Vec::new());
        html.push(document.html());
    }
    Ok(json!({
        "domain": HTML_DOMAIN,
        "identity": HtmlIdentity { observation: observation.id.clone(), roots },
        "content": { "html": html }
    }))
}

fn html_identity(region: &Region) -> Result<HtmlIdentity> {
    ensure!(
        region.domain == HTML_DOMAIN,
        "expected {HTML_DOMAIN} Region"
    );
    let identity: HtmlIdentity =
        serde_json::from_value(region.identity.clone()).context("invalid HTML identity")?;
    ensure!(
        !identity.observation.is_empty(),
        "HTML identity needs an Observation ID"
    );
    for pair in identity.roots.windows(2) {
        ensure!(
            pair[0] < pair[1] && !pair[1].starts_with(&pair[0]),
            "HTML identity roots must be in DOM order, unique, and free of ancestor/descendant pairs"
        );
    }
    Ok(identity)
}

fn compare_html(left: &Region, right: &Region) -> Result<Relation> {
    let left = html_identity(left)?;
    let right = html_identity(right)?;
    if left.observation != right.observation {
        return Ok(Relation::Incomparable);
    }
    Ok(
        match (
            html_subset(&left.roots, &right.roots),
            html_subset(&right.roots, &left.roots),
        ) {
            (true, true) => Relation::Equal,
            (true, false) => Relation::Within,
            (false, true) => Relation::Contains,
            (false, false) => Relation::Incomparable,
        },
    )
}

fn html_subset(a: &[Vec<usize>], b: &[Vec<usize>]) -> bool {
    let mut candidates = b.iter().peekable();
    let mut ancestor = None;
    for path in a {
        while candidates
            .peek()
            .is_some_and(|candidate| *candidate <= path)
        {
            ancestor = candidates.next();
        }
        if !ancestor.is_some_and(|ancestor| path.starts_with(ancestor)) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ResourceState;

    fn observation(bytes: &[u8]) -> Observation {
        Observation::new(
            "document".into(),
            bytes,
            "0".into(),
            "file".into(),
            ResourceState {
                exists: true,
                revision: Value::Null,
            },
        )
    }

    fn resolved(name: &str, query: Value, bytes: &[u8]) -> Result<Value> {
        resolve(
            name,
            &RegionSelector {
                extension: name.into(),
                resource: Some("document".into()),
                query,
            },
            &observation(bytes),
            bytes,
        )
    }

    fn region(name: &str, query: Value, source: &str) -> Region {
        let mut value = resolved(name, query, source.as_bytes()).unwrap();
        value["observations"] = json!([observation(source.as_bytes()).id]);
        serde_json::from_value(value).unwrap()
    }

    fn text(name: &str, query: Value, source: &str) -> String {
        region(name, query, source).content["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn error(name: &str, query: Value, source: &str, expected: &str) {
        let error = resolved(name, query.clone(), source.as_bytes()).unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "{name} {query}: {error:#}"
        );
    }

    fn related(name: &str, left: &Region, right: &Region, expected: Relation) {
        assert_eq!(compare(name, left, right).unwrap(), expected);
        let reverse = match expected {
            Relation::Contains => Relation::Within,
            Relation::Within => Relation::Contains,
            other => other,
        };
        assert_eq!(compare(name, right, left).unwrap(), reverse);
    }

    #[test]
    fn manifests_and_protocol_result_are_complete() {
        for name in ["text", "markdown", "html"] {
            let manifest = manifest(name).unwrap();
            manifest.validate().unwrap();
            assert_eq!(manifest.revision, "1");
            assert_eq!(manifest.operations, ["resolve", "compare"]);
            assert_eq!(
                manifest.domains,
                [if name == "html" {
                    HTML_DOMAIN
                } else {
                    "monika.bytes.v1"
                }]
            );
            let value = resolved(name, Value::Null, b"hello").unwrap();
            assert_eq!(value.as_object().unwrap().len(), 3);
            assert!(value.get("observations").is_none());
        }
        assert!(manifest("pdf").is_none());
        assert!(resolved("pdf", Value::Null, b"").is_err());
        assert!(
            compare(
                "pdf",
                &region("text", Value::Null, ""),
                &region("text", Value::Null, "")
            )
            .is_err()
        );
    }

    #[test]
    fn all_documents_require_utf8() {
        for name in ["text", "markdown", "html"] {
            let err = resolved(name, Value::Null, &[0xff]).unwrap_err();
            assert!(err.to_string().contains("UTF-8"));
        }
    }

    #[test]
    fn text_whole_and_lines_preserve_original_bytes() {
        let source = "一行\r\n二行\n\r四行\r最後";
        assert_eq!(text("text", Value::Null, source), source);
        assert_eq!(
            text("text", json!({"lines":[1, 2]}), source),
            "一行\r\n二行\n"
        );
        assert_eq!(text("text", json!({"lines":[3, 4]}), source), "\r四行\r");
        assert_eq!(text("text", json!({"lines":[5, 5]}), source), "最後");
        assert_eq!(text("text", json!({"lines":[1, 1]}), "\n"), "\n");
        error("text", json!({"lines":[2, 2]}), "x\n", "out of range");
        error("text", json!({"lines":[1, 1]}), "", "0 lines");
        let empty = region("text", Value::Null, "");
        assert_eq!(empty.identity["ranges"], json!([]));
        assert_eq!(empty.content, json!({"text":""}));
    }

    #[test]
    fn text_literal_occurrences_include_overlapping_unicode_matches() {
        let source = "前あああ後";
        error("text", json!({"text":"ああ"}), source, "ambiguous");
        let second = region("text", json!({"text":"ああ", "occurrence":2}), source);
        assert_eq!(second.identity["ranges"], json!([[6, 12]]));
        assert_eq!(second.content, json!({"text":"ああ"}));
        let all = region("text", json!({"text":"ああ", "occurrence":"all"}), source);
        assert_eq!(all.identity["ranges"], json!([[3, 12]]));
        assert_eq!(all.content, json!({"text":"あああ"}));
        assert_eq!(text("text", json!({"text":"あ後"}), source), "あ後");
        assert_eq!(text("text", json!({"text":"a.b"}), "a.b axb"), "a.b");
        assert_eq!(text("text", json!({"text":"\\n"}), "\\n\n"), "\\n");
    }

    #[test]
    fn text_empty_results_and_invalid_occurrences_are_distinct() {
        for query in [
            json!({"text":"missing"}),
            json!({"text":"missing","occurrence":"all"}),
        ] {
            let result = region("text", query, "data");
            assert_eq!(result.identity["ranges"], json!([]));
            assert_eq!(result.content, json!({"text":""}));
        }
        error(
            "text",
            json!({"text":"missing","occurrence":1}),
            "data",
            "out of range",
        );
        error(
            "text",
            json!({"text":"a","occurrence":2}),
            "a",
            "out of range",
        );
    }

    #[test]
    fn text_rejects_invalid_query_shapes_and_indices() {
        for query in [
            json!({}),
            json!("whole"),
            json!([]),
            json!({"line":[1,2]}),
            json!({"lines":[1]}),
            json!({"lines":[1,2,3]}),
            json!({"lines":[0,1]}),
            json!({"lines":[2,1]}),
            json!({"lines":[-1,1]}),
            json!({"lines":[1.0,1]}),
            json!({"lines":[1,"1"]}),
            json!({"lines":[1,1],"text":"a"}),
            json!({"lines":[1,1],"occurrence":1}),
            json!({"text":""}),
            json!({"text":null}),
            json!({"text":"a","occurrence":0}),
            json!({"text":"a","occurrence":null}),
            json!({"text":"a","occurrence":"first"}),
            json!({"text":"a","occurrence":1.5}),
            json!({"text":"a","case_sensitive":false}),
        ] {
            assert!(
                resolved("text", query.clone(), b"a\nb\n").is_err(),
                "{query}"
            );
        }
    }

    #[test]
    fn text_comparison_uses_byte_sets_and_observation_ids() {
        let source = "ababa";
        let whole = region("text", Value::Null, source);
        let left = region("text", json!({"text":"aba","occurrence":1}), source);
        let right = region("text", json!({"text":"aba","occurrence":2}), source);
        let all = region("text", json!({"text":"aba","occurrence":"all"}), source);
        let empty = region("text", json!({"text":"missing"}), source);
        related("text", &whole, &all, Relation::Equal);
        related("text", &whole, &left, Relation::Contains);
        related("text", &left, &right, Relation::Incomparable);
        related("text", &left, &empty, Relation::Contains);
        related(
            "text",
            &whole,
            &region("text", Value::Null, "other"),
            Relation::Incomparable,
        );
    }

    const MARKDOWN: &str =
        "intro\n\n# A\nbody\n## B\nchild\n### C\nleaf\n## D\nsibling\n# E\nend\n";

    #[test]
    fn markdown_sections_include_descendants_and_stop_before_peers() {
        assert_eq!(text("markdown", Value::Null, MARKDOWN), MARKDOWN);
        let a = region("markdown", json!({"section":"A"}), MARKDOWN);
        let b = region("markdown", json!({"section":["A","B"]}), MARKDOWN);
        let c = region("markdown", json!({"section":["A","B","C"]}), MARKDOWN);
        assert_eq!(
            a.content["text"],
            "# A\nbody\n## B\nchild\n### C\nleaf\n## D\nsibling\n"
        );
        assert_eq!(b.content["text"], "## B\nchild\n### C\nleaf\n");
        assert_eq!(c.content["text"], "### C\nleaf\n");
        related("markdown", &a, &b, Relation::Contains);
        related("markdown", &b, &c, Relation::Contains);
        related("markdown", &a, &c, Relation::Contains);
        assert_eq!(
            text("markdown", json!({"heading":["A","B"]}), MARKDOWN),
            "## B\n"
        );
        assert_eq!(text("markdown", json!({"section":["B"]}), MARKDOWN), "");
    }

    #[test]
    fn markdown_uses_commonmark_heading_text_and_offsets() {
        let source = "序文\r\n\r\n見出し *強調* &amp; `コード`\r\n===\r\n本文\r\n\r\n## [link](https://example.com) ![画像](x) \\* ###\r\n後\r\n";
        let first = "見出し *強調* &amp; `コード`\r\n===\r\n";
        assert_eq!(
            text(
                "markdown",
                json!({"heading":"見出し 強調 & コード"}),
                source
            ),
            first
        );
        assert_eq!(
            text("markdown", json!({"heading":"link 画像 *"}), source),
            "## [link](https://example.com) ![画像](x) \\* ###\r\n"
        );
        let multi = "A\nB\n---\nbody\n";
        assert_eq!(
            text("markdown", json!({"heading":"A B"}), multi),
            "A\nB\n---\n"
        );
        assert_eq!(
            text(
                "markdown",
                json!({"heading":"inside"}),
                "# <em>inside</em>\n"
            ),
            "# <em>inside</em>\n"
        );
        assert_eq!(text("markdown", json!({"heading":""}), "#\n"), "#\n");
    }

    #[test]
    fn markdown_code_and_html_blocks_do_not_create_headings() {
        let source = "# real\n\n```md\n# fake\n```\n\n    # indented\n\n<div>\n# raw html\n</div>\n\n\\# escaped\n\n#no-space\n\n####### seven\n";
        for title in [
            "fake", "indented", "raw html", "escaped", "no-space", "seven",
        ] {
            assert_eq!(
                text("markdown", json!({"section":title}), source),
                "",
                "{title}"
            );
        }
        assert_eq!(text("markdown", json!({"section":"real"}), source), source);
    }

    #[test]
    fn markdown_paths_handle_skipped_levels_and_duplicate_siblings() {
        let source = "# A\n### Item\none\n### Item\ntwo\n# A\n### Item\nthree\n";
        error("markdown", json!({"section":"A"}), source, "ambiguous");
        error(
            "markdown",
            json!({"section":["A","Item"]}),
            source,
            "ambiguous",
        );
        error(
            "markdown",
            json!({"section":[{"title":"A","occurrence":1},"Item"]}),
            source,
            "ambiguous",
        );
        assert_eq!(
            text(
                "markdown",
                json!({"section":[{"title":"A","occurrence":1},{"title":"Item","occurrence":2}]}),
                source
            ),
            "### Item\ntwo\n"
        );
        assert_eq!(
            text(
                "markdown",
                json!({"section":[{"title":"A","occurrence":2},"Item"]}),
                source
            ),
            "### Item\nthree\n"
        );
        assert_eq!(
            text("markdown", json!({"section":"Item","occurrence":3}), source),
            "### Item\nthree\n"
        );
        let all = region(
            "markdown",
            json!({"section":"A","occurrence":"all"}),
            source,
        );
        related(
            "markdown",
            &all,
            &region("markdown", Value::Null, source),
            Relation::Equal,
        );
    }

    #[test]
    fn markdown_global_duplicates_and_overlap_are_normalized() {
        let source = "# A\nroot\n## A\nchild\n# Z\nend\n";
        let all = region(
            "markdown",
            json!({"section":"A","occurrence":"all"}),
            source,
        );
        let root = region("markdown", json!({"section":["A"]}), source);
        related("markdown", &all, &root, Relation::Equal);
        assert_eq!(all.content["text"], "# A\nroot\n## A\nchild\n");
        error(
            "markdown",
            json!({"section":"absent","occurrence":1}),
            source,
            "out of range",
        );
        error(
            "markdown",
            json!({"section":[{"title":"A","occurrence":3}]}),
            source,
            "out of range",
        );
    }

    #[test]
    fn markdown_fences_preserve_delimiters_info_and_unclosed_source() {
        let source = "```rust ignore\r\nlet x = 1;\r\n```\r\n\n~~~python\nprint(1)\n~~~\n\n    indented\n\n```rust\nlast";
        assert_eq!(
            text(
                "markdown",
                json!({"code":{"language":"rust","occurrence":1}}),
                source
            ),
            "```rust ignore\r\nlet x = 1;\r\n```"
        );
        assert_eq!(
            text(
                "markdown",
                json!({"code":{"language":"rust","occurrence":2}}),
                source
            ),
            "```rust\nlast"
        );
        let all = text("markdown", json!({"code":{}}), source);
        assert_eq!(
            all,
            "```rust ignore\r\nlet x = 1;\r\n```~~~python\nprint(1)\n~~~```rust\nlast"
        );
        assert_eq!(
            text("markdown", json!({"code":{"language":"go"}}), source),
            ""
        );
        assert_eq!(
            text("markdown", json!({"code":{"language":""}}), "```\nx\n```\n"),
            "```\nx\n```"
        );
        error(
            "markdown",
            json!({"code":{"language":"go","occurrence":1}}),
            source,
            "out of range",
        );
    }

    #[test]
    fn markdown_rejects_malformed_queries_even_after_missing_path_components() {
        for query in [
            json!({}),
            json!("A"),
            json!({"section":null}),
            json!({"section":[]}),
            json!({"section":1}),
            json!({"section":"A","heading":"A"}),
            json!({"section":"A","level":1}),
            json!({"section":"A","occurrence":null}),
            json!({"section":["A"],"occurrence":1}),
            json!({"section":[{"name":"A"}]}),
            json!({"section":[{"title":"A","occurrence":"all"}]}),
            json!({"section":["missing",{"title":"A","occurrence":0}]}),
            json!({"section":["missing",{"title":"A","typo":1}]}),
            json!({"section":["missing",null]}),
            json!({"code":true}),
            json!({"code":{"language":null}}),
            json!({"code":{"language":"rust extra"}}),
            json!({"code":{"language":"rust","index":1}}),
        ] {
            assert!(
                resolved("markdown", query.clone(), b"# A\n").is_err(),
                "{query}"
            );
        }
    }

    #[test]
    fn text_and_markdown_compare_across_extension_boundaries() {
        let source = "intro\n# A\nbody\n# B\nend\n";
        let lines = region("text", json!({"lines":[2,3]}), source);
        let section = region("markdown", json!({"section":"A"}), source);
        related("text", &lines, &section, Relation::Equal);
        related("markdown", &lines, &section, Relation::Equal);
        let heading = region("markdown", json!({"heading":"A"}), source);
        related("text", &lines, &heading, Relation::Contains);
    }

    #[test]
    fn source_positions_outside_text_and_markdown_selections_do_not_change_content() {
        for (name, query) in [
            ("text", json!({"text":"selected"})),
            ("markdown", json!({"section":"A"})),
        ] {
            let left = region(name, query.clone(), "# A\nselected\n# B\nold\n");
            let right = region(name, query, "new preface\n\n# A\nselected\n# B\nnew\n");
            assert_ne!(left.identity, right.identity);
            assert_eq!(left.content, right.content);
            related(name, &left, &right, Relation::Incomparable);
        }
    }

    const HTML: &str = "<!doctype html><!--before--><html><head><title>T</title></head><body><article id='a'><p id='x'>日本語 &amp; A</p><p id='y'><b>B</b></p></article><aside id='z'>Z</aside></body></html>";

    #[test]
    fn html_css_returns_outer_html_in_dom_order_and_preserves_unicode() {
        let selected = region("html", json!({"css":"#y, #x"}), HTML);
        assert_eq!(
            selected.content,
            json!({"html":["<p id=\"x\">日本語 &amp; A</p>","<p id=\"y\"><b>B</b></p>"]})
        );
        let second = region("html", json!({"css":"article > p","occurrence":2}), HTML);
        assert_eq!(second.content["html"], json!(["<p id=\"y\"><b>B</b></p>"]));
        related("html", &selected, &second, Relation::Contains);
    }

    #[test]
    fn html_deduplicates_css_groups_and_absorbs_descendant_subtrees() {
        let article = region("html", json!({"css":"article"}), HTML);
        let duplicate = region("html", json!({"css":"#a, article, article > p, b"}), HTML);
        assert_eq!(article, duplicate);
        let a = region("html", json!({"css":"#x, #y"}), HTML);
        let b = region("html", json!({"css":"article p"}), HTML);
        related("html", &a, &b, Relation::Equal);
        related("html", &article, &a, Relation::Contains);
    }

    #[test]
    fn html_union_containment_is_not_just_selected_root_equality() {
        let parents = region("html", json!({"css":"article, aside"}), HTML);
        let children = region("html", json!({"css":"#x, #y, #z"}), HTML);
        let deep = region("html", json!({"css":"b"}), HTML);
        related("html", &parents, &children, Relation::Contains);
        related("html", &children, &deep, Relation::Contains);
        related("html", &parents, &deep, Relation::Contains);
        let left = region("html", json!({"css":"#x, #y"}), HTML);
        let right = region("html", json!({"css":"#y, #z"}), HTML);
        related("html", &left, &right, Relation::Incomparable);
        related(
            "html",
            &left,
            &region("html", json!({"css":"#z"}), HTML),
            Relation::Incomparable,
        );
    }

    #[test]
    fn html_whole_document_contains_root_and_empty_is_a_valid_subset() {
        let whole = region("html", Value::Null, HTML);
        let root = region("html", json!({"css":":root"}), HTML);
        let all_elements = region("html", json!({"css":"*"}), HTML);
        assert_eq!(whole.identity["roots"], json!([[]]));
        assert!(
            whole.content["html"][0]
                .as_str()
                .unwrap()
                .starts_with("<!DOCTYPE html><!--before-->")
        );
        related("html", &whole, &root, Relation::Contains);
        related("html", &root, &all_elements, Relation::Equal);
        let empty = region("html", json!({"css":".missing"}), HTML);
        assert_eq!(empty.identity["roots"], json!([]));
        assert_eq!(empty.content, json!({"html":[]}));
        related("html", &root, &empty, Relation::Contains);
        related(
            "html",
            &empty,
            &region("html", json!({"css":"#missing"}), HTML),
            Relation::Equal,
        );
        let empty_document = region("html", Value::Null, "");
        assert_eq!(
            empty_document.content,
            json!({"html":["<html><head></head><body></body></html>"]})
        );
    }

    #[test]
    fn html_paths_count_text_comments_and_actual_dom_children() {
        let source = "<!doctype html><!--c--><p id=a>A<!--middle--><b>B</b></p>";
        let child = region("html", json!({"css":"b"}), source);
        // Document children: doctype, comment, html; html children: head, body;
        // body child: p; p children: text, comment, b.
        assert_eq!(child.identity["roots"], json!([[2, 1, 0, 2]]));
        related(
            "html",
            &region("html", json!({"css":"p"}), source),
            &child,
            Relation::Contains,
        );
    }

    #[test]
    fn html5_recovery_uses_dom_order_after_foster_parenting() {
        let source = "<!doctype html><table class=pick><p class=pick>first</p><tr><td>cell</table>";
        let first = region("html", json!({"css":".pick","occurrence":1}), source);
        assert_eq!(
            first.content,
            json!({"html":["<p class=\"pick\">first</p>"]})
        );
        let table = region("html", json!({"css":"table"}), source);
        related("html", &first, &table, Relation::Incomparable);
        let repaired = region("html", json!({"css":"tbody > tr > td"}), source);
        assert_eq!(repaired.content, json!({"html":["<td>cell</td>"]}));
        related("html", &table, &repaired, Relation::Contains);
        let paragraphs = region("html", json!({"css":"p"}), "<p>one<p>two");
        assert_eq!(
            paragraphs.content,
            json!({"html":["<p>one</p>","<p>two</p>"]})
        );
    }

    #[test]
    fn html_css_supports_escaping_attributes_and_duplicate_ids() {
        let source = "<p id='a:b' data-label='日 本'>一</p><p id='same'>二</p><p id='same'>三</p>";
        let colon = region("html", json!({"css":"#a\\:b"}), source);
        let attribute = region("html", json!({"css":"[data-label=\"日 本\"]"}), source);
        related("html", &colon, &attribute, Relation::Equal);
        let duplicate_ids = region("html", json!({"css":"#same"}), source);
        assert_eq!(duplicate_ids.content["html"].as_array().unwrap().len(), 2);
        let second = region("html", json!({"css":"#same","occurrence":2}), source);
        assert_eq!(second.content, json!({"html":["<p id=\"same\">三</p>"]}));
    }

    #[test]
    fn html_queries_reject_unknown_fields_bad_css_and_invalid_occurrences() {
        for query in [
            json!({}),
            json!("p"),
            json!({"css":null}),
            json!({"css":""}),
            json!({"css":"  "}),
            json!({"css":"["}),
            json!({"css":"p,"}),
            json!({"css":"p::before"}),
            json!({"css":"p:hover"}),
            json!({"css":"p","text":true}),
            json!({"css":"p","occurrence":0}),
            json!({"css":"p","occurrence":null}),
            json!({"css":"p","occurrence":1.0}),
            json!({"css":"p","occurrence":"first"}),
        ] {
            assert!(
                resolved("html", query.clone(), b"<p>ok</p>").is_err(),
                "{query}"
            );
        }
        error(
            "html",
            json!({"css":"missing","occurrence":1}),
            HTML,
            "out of range",
        );
        error(
            "html",
            json!({"css":"p","occurrence":3}),
            HTML,
            "out of range",
        );
    }

    #[test]
    fn html_content_ignores_outside_moves_but_includes_attributes_and_descendants() {
        let query = json!({"css":"#selected"});
        let original = region(
            "html",
            query.clone(),
            "<div id=selected><b>日本語</b></div>",
        );
        let moved = region(
            "html",
            query.clone(),
            "<aside>new</aside><!--gap--><div id=selected><b>日本語</b></div>",
        );
        assert_eq!(original.content, moved.content);
        assert_ne!(original.identity, moved.identity);
        related("html", &original, &moved, Relation::Incomparable);
        for source in [
            "<div id=selected title=new><b>日本語</b></div>",
            "<div id=selected><b>changed</b></div>",
        ] {
            assert_ne!(
                original.content,
                region("html", query.clone(), source).content
            );
        }
        let normalized = region("html", query, "<DIV id='selected'><b>日本語</b></DIV>");
        assert_eq!(original.content, normalized.content);
    }

    #[test]
    fn html_comparison_rejects_malformed_identity_and_foreign_domains() {
        let good = region("html", json!({"css":"p"}), HTML);
        for identity in [
            json!({"observation":"x","roots":[[1],[1]]}),
            json!({"observation":"x","roots":[[2],[1]]}),
            json!({"observation":"x","roots":[[1],[1,0]]}),
            json!({"observation":"x","roots":[[],[1]]}),
            json!({"observation":"x","roots":[[-1]]}),
            json!({"observation":"","roots":[]}),
            json!({"observation":"x","roots":[],"typo":true}),
        ] {
            let mut bad = good.clone();
            bad.identity = identity;
            assert!(compare("html", &good, &bad).is_err());
        }
        assert!(compare("html", &good, &region("text", Value::Null, HTML)).is_err());
        assert!(compare("text", &good, &good).is_err());
    }

    #[test]
    fn html_comparison_laws_hold_for_all_small_subtree_unions() {
        let source =
            "<main><section id=a><p id=b>B</p><p id=c>C</p></section><aside id=d>D</aside></main>";
        let mut regions = vec![region("html", Value::Null, source)];
        for mask in 0..16 {
            let css = ["#a", "#b", "#c", "#d"]
                .into_iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, id)| id)
                .collect::<Vec<_>>()
                .join(",");
            regions.push(region(
                "html",
                json!({"css":if css.is_empty() { ".missing" } else { &css }}),
                source,
            ));
        }
        let contains = |a: &Region, b: &Region| {
            matches!(
                compare("html", a, b).unwrap(),
                Relation::Equal | Relation::Contains
            )
        };
        for a in &regions {
            related("html", a, a, Relation::Equal);
            for b in &regions {
                let relation = compare("html", a, b).unwrap();
                related("html", a, b, relation);
                assert_eq!(
                    relation == Relation::Equal,
                    contains(a, b) && contains(b, a)
                );
                for c in &regions {
                    if contains(a, b) && contains(b, c) {
                        assert!(contains(a, c));
                    }
                }
            }
        }
    }
}
