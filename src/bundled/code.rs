//! Syntax-based selection from the immutable bytes supplied by Core.

use crate::extensions::process::Manifest;
use crate::model::{Observation, Region, RegionSelector, Relation};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

pub fn manifest(name: &str) -> Option<Manifest> {
    (name == "code").then(|| Manifest {
        name: name.into(),
        revision: "1".into(),
        operations: vec!["resolve".into(), "compare".into()],
        domains: vec!["monika.bytes.v1".into()],
        description: concat!(
            "Tree-sitter syntax selection. UTF-8 Resource required. query: null or ",
            "{language?} for whole file; {language?,symbol,scope?:[name],kind?,occurrence?:1} ",
            "for one declaration (occurrence is a one-based positive integer); ",
            "{language?,query,capture} for all matching captures. ",
            "Example: {\"symbol\":\"run\",\"scope\":[\"Worker\"]}. ",
            "Languages: rust, python, javascript, typescript, tsx, go, java, c, cpp, ruby, bash; ",
            "inferred from known extensions when omitted. Syntax errors and ambiguous symbols fail. ",
            "content: {text: original selected slices joined without separators}. See docs/extensions/code.md."
        ).into(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Syntax {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Java,
    C,
    Cpp,
    Ruby,
    Bash,
}

impl Syntax {
    fn from_name(name: &str) -> Result<Self> {
        Ok(match name {
            "rust" => Self::Rust,
            "python" => Self::Python,
            "javascript" => Self::JavaScript,
            "typescript" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "go" => Self::Go,
            "java" => Self::Java,
            "c" => Self::C,
            "cpp" => Self::Cpp,
            "ruby" => Self::Ruby,
            "bash" => Self::Bash,
            _ => bail!(
                "code: unknown language {name:?}; expected rust, python, javascript, typescript, tsx, go, java, c, cpp, ruby, or bash"
            ),
        })
    }

    fn infer(resource: &str) -> Result<Self> {
        let url = url::Url::parse(resource).ok();
        let path = url.as_ref().map_or(resource, |url| url.path());
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if extension == "C" {
            return Ok(Self::Cpp);
        }
        Ok(match extension.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "go" => Self::Go,
            "java" => Self::Java,
            "c" => Self::C,
            "cc" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" => Self::Cpp,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "sh" | "bash" => Self::Bash,
            "h" => bail!("code: .h is ambiguous between C and C++; specify language as c or cpp"),
            _ => bail!(
                "code: cannot infer language from Resource {resource:?}; specify language explicitly"
            ),
        })
    }

    fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
        }
    }
}

enum Selection<'a> {
    Whole,
    Symbol {
        name: &'a str,
        scope: Option<Vec<&'a str>>,
        kind: Option<&'a str>,
        occurrence: Option<usize>,
    },
    Captures {
        query: &'a str,
        capture: &'a str,
    },
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>> {
    object
        .get(key)
        .map(|value| {
            let text = value
                .as_str()
                .with_context(|| format!("code: {key} must be a string"))?;
            ensure!(!text.trim().is_empty(), "code: {key} must not be empty");
            Ok(text)
        })
        .transpose()
}

fn parse_query<'a>(query: &'a Value, resource: &str) -> Result<(Syntax, Selection<'a>)> {
    if query.is_null() {
        return Ok((Syntax::infer(resource)?, Selection::Whole));
    }
    let object = query
        .as_object()
        .context("code: query must be null or an object")?;
    for key in object.keys() {
        ensure!(
            matches!(
                key.as_str(),
                "language" | "symbol" | "scope" | "kind" | "occurrence" | "query" | "capture"
            ),
            "code: unknown query field {key:?}"
        );
    }
    let syntax = string(object, "language")?
        .map(Syntax::from_name)
        .unwrap_or_else(|| Syntax::infer(resource))?;
    if let Some(name) = string(object, "symbol")? {
        ensure!(
            !object.contains_key("query") && !object.contains_key("capture"),
            "code: symbol selection cannot contain query or capture"
        );
        let kind = string(object, "kind")?;
        if let Some(kind) = kind {
            ensure!(
                matches!(
                    kind,
                    "function"
                        | "method"
                        | "class"
                        | "struct"
                        | "enum"
                        | "trait"
                        | "interface"
                        | "module"
                        | "namespace"
                        | "impl"
                        | "type"
                        | "constant"
                        | "static"
                        | "macro"
                        | "union"
                        | "constructor"
                ),
                "code: unknown declaration kind {kind:?}"
            );
        }
        let scope = object
            .get("scope")
            .map(|value| {
                value
                    .as_array()
                    .context("code: scope must be an array of declaration names")?
                    .iter()
                    .map(|part| {
                        let part = part
                            .as_str()
                            .context("code: each scope entry must be a string")?;
                        ensure!(
                            !part.trim().is_empty(),
                            "code: scope entries must not be empty"
                        );
                        Ok(part)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let occurrence = object
            .get("occurrence")
            .map(|value| {
                let index = value
                    .as_u64()
                    .context("code: occurrence must be a positive integer (one-based)")?;
                ensure!(
                    index > 0,
                    "code: occurrence must be a positive integer (one-based)"
                );
                usize::try_from(index).context("code: occurrence is too large")
            })
            .transpose()?;
        Ok((
            syntax,
            Selection::Symbol {
                name,
                scope,
                kind,
                occurrence,
            },
        ))
    } else if let Some(query) = string(object, "query")? {
        ensure!(
            !object.contains_key("scope")
                && !object.contains_key("kind")
                && !object.contains_key("occurrence"),
            "code: scope, kind, and occurrence require symbol selection"
        );
        let capture =
            string(object, "capture")?.context("code: raw query requires capture (without @)")?;
        ensure!(
            !capture.starts_with('@'),
            "code: capture name must omit the @ prefix"
        );
        Ok((syntax, Selection::Captures { query, capture }))
    } else {
        ensure!(
            object.keys().all(|key| key == "language"),
            "code: scope, kind, occurrence, and capture require a selection"
        );
        Ok((syntax, Selection::Whole))
    }
}

pub fn resolve(
    name: &str,
    selector: &RegionSelector,
    observation: &Observation,
    bytes: &[u8],
) -> Result<Value> {
    ensure!(name == "code", "code: unknown extension {name:?}");
    let resource = selector
        .resource
        .as_deref()
        .context("code: Resource is required")?;
    let source = std::str::from_utf8(bytes).context("code: Resource must be valid UTF-8")?;
    let (syntax, selection) = parse_query(&selector.query, resource)?;
    let language = syntax.language();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .context("code: cannot load Tree-sitter grammar")?;
    let tree = parser
        .parse(bytes, None)
        .context("code: Tree-sitter parsing did not complete")?;
    let root = tree.root_node();
    if root.has_error() {
        let error = first_error(root);
        let point = error.start_position();
        bail!(
            "code: syntax error in {resource} at line {}, byte column {} ({} {})",
            point.row + 1,
            point.column + 1,
            if error.is_missing() {
                "missing"
            } else {
                "unexpected"
            },
            error.kind()
        );
    }
    let ranges = match selection {
        Selection::Whole => vec![(0, bytes.len())],
        Selection::Symbol {
            name,
            scope,
            kind,
            occurrence,
        } => {
            let mut declarations = Vec::new();
            collect_declarations(root, syntax, source, &mut Vec::new(), &mut declarations);
            let candidates: Vec<_> = declarations
                .iter()
                .filter(|declaration| {
                    declaration.name == name
                        && kind.is_none_or(|kind| kind == declaration.kind)
                        && scope.as_ref().is_none_or(|scope| {
                            scope
                                .iter()
                                .copied()
                                .eq(declaration.scope.iter().map(String::as_str))
                        })
                })
                .collect();
            ensure!(
                !candidates.is_empty(),
                "code: no declaration matches symbol {name:?}, scope {scope:?}, kind {kind:?}"
            );
            let selected = if let Some(index) = occurrence {
                candidates.get(index - 1).with_context(|| format!("code: occurrence {index} out of range for symbol {name:?}; {} matching declarations", candidates.len()))?
            } else {
                ensure!(
                    candidates.len() == 1,
                    "code: ambiguous symbol {name:?}: {} declarations match; specify scope, kind, or occurrence. Matches: {}",
                    candidates.len(),
                    candidates
                        .iter()
                        .take(8)
                        .map(|decl| format!(
                            "{} {:?} at line {}",
                            decl.kind,
                            decl.scope
                                .iter()
                                .map(String::as_str)
                                .chain([decl.name.as_str()])
                                .collect::<Vec<_>>(),
                            decl.line
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                &candidates[0]
            };
            vec![(selected.start, selected.end)]
        }
        Selection::Captures { query, capture } => {
            query_ranges(&language, root, bytes, query, capture)?
        }
    };
    super::span_region(observation, bytes, ranges)
}

pub fn compare(name: &str, left: &Region, right: &Region) -> Result<Relation> {
    ensure!(name == "code", "code: unknown extension {name:?}");
    super::compare_spans(left, right)
}

fn first_error(mut node: Node<'_>) -> Node<'_> {
    while !node.is_error() && !node.is_missing() {
        let mut cursor = node.walk();
        let child = node
            .children(&mut cursor)
            .find(|child| child.has_error() || child.is_missing());
        match child {
            Some(child) => node = child,
            None => break,
        }
    }
    node
}

fn query_ranges(
    language: &Language,
    root: Node<'_>,
    bytes: &[u8],
    source: &str,
    capture: &str,
) -> Result<Vec<(usize, usize)>> {
    let query = Query::new(language, source).context("code: invalid Tree-sitter query")?;
    let capture_index = query
        .capture_index_for_name(capture)
        .with_context(|| format!("code: query has no capture named {capture:?}"))?;
    for pattern in 0..query.pattern_count() {
        ensure!(
            query.general_predicates(pattern).is_empty()
                && query.property_predicates(pattern).is_empty()
                && query.property_settings(pattern).is_empty(),
            "code: unsupported Tree-sitter predicate or directive; only built-in text predicates (eq?, match?, any-of? and their variants) are supported"
        );
    }
    let mut cursor = QueryCursor::new();
    let mut ranges = Vec::new();
    {
        let mut matches = cursor.matches(&query, root, bytes);
        while let Some(matched) = matches.next() {
            ranges.extend(
                matched
                    .captures
                    .iter()
                    .filter(|capture| capture.index == capture_index)
                    .map(|capture| (capture.node.start_byte(), capture.node.end_byte())),
            );
        }
    }
    ensure!(
        !cursor.did_exceed_match_limit(),
        "code: Tree-sitter query exceeded its match limit; simplify the query"
    );
    Ok(ranges)
}

struct Declaration {
    name: String,
    kind: &'static str,
    scope: Vec<String>,
    start: usize,
    end: usize,
    line: usize,
}

struct NamedScope {
    name: String,
    kind: &'static str,
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn named_children(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    // A range iterator avoids borrowing a TreeCursor across recursive traversal.
    (0..node.named_child_count()).filter_map(move |index| node.named_child(index))
}

fn collect_declarations(
    node: Node<'_>,
    syntax: Syntax,
    source: &str,
    scope: &mut Vec<NamedScope>,
    output: &mut Vec<Declaration>,
) {
    let original_depth = scope.len();
    if let Some((kind, names)) = declaration_name(node, syntax, source, scope) {
        let (name, qualifiers) = names.split_last().expect("a declaration has a name");
        scope.extend(qualifiers.iter().map(|name| NamedScope {
            name: name.clone(),
            kind: "namespace",
        }));
        let (start, end) = declaration_range(node, syntax);
        output.push(Declaration {
            name: name.clone(),
            kind,
            scope: scope.iter().map(|scope| scope.name.clone()).collect(),
            start,
            end,
            line: node.start_position().row + 1,
        });
        scope.push(NamedScope {
            name: name.clone(),
            kind,
        });
    }
    for child in named_children(node) {
        collect_declarations(child, syntax, source, scope, output);
    }
    scope.truncate(original_depth);
}

fn declaration_name(
    node: Node<'_>,
    syntax: Syntax,
    source: &str,
    scope: &[NamedScope],
) -> Option<(&'static str, Vec<String>)> {
    let tag = node.kind();
    let in_type = scope.last().is_some_and(|scope| {
        matches!(
            scope.kind,
            "class" | "struct" | "union" | "trait" | "interface" | "impl" | "enum"
        )
    });
    let mut name = node.child_by_field_name("name");
    let kind = match syntax {
        Syntax::Rust => match tag {
            "function_item" | "function_signature_item" => {
                if in_type {
                    "method"
                } else {
                    "function"
                }
            }
            "mod_item" => "module",
            "struct_item" => "struct",
            "enum_item" => "enum",
            "trait_item" => "trait",
            "type_item" | "associated_type" => "type",
            "const_item" => "constant",
            "static_item" => "static",
            "union_item" => "union",
            "macro_definition" => "macro",
            "impl_item" => {
                name = node.child_by_field_name("type").map(base_type);
                "impl"
            }
            _ => return None,
        },
        Syntax::Python => match tag {
            "function_definition" => {
                if in_type {
                    "method"
                } else {
                    "function"
                }
            }
            "class_definition" => "class",
            _ => return None,
        },
        Syntax::JavaScript | Syntax::TypeScript | Syntax::Tsx => match tag {
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                "function"
            }
            "class_declaration" | "abstract_class_declaration" => "class",
            "method_definition" | "method_signature" | "abstract_method_signature" => "method",
            "interface_declaration" => "interface",
            "enum_declaration" => "enum",
            "type_alias_declaration" => "type",
            "internal_module" | "module" => "module",
            "variable_declarator" => {
                let value = node.child_by_field_name("value")?;
                match value.kind() {
                    "arrow_function" | "function_expression" | "generator_function" => "function",
                    "class" => "class",
                    _ => return None,
                }
            }
            "field_definition" | "public_field_definition" => {
                name = name.or_else(|| node.child_by_field_name("property"));
                let value = node.child_by_field_name("value")?;
                match value.kind() {
                    "arrow_function" | "function_expression" | "generator_function" => "method",
                    _ => return None,
                }
            }
            _ => return None,
        },
        Syntax::Go => match tag {
            "function_declaration" => "function",
            "method_declaration" => {
                let receiver = node.child_by_field_name("receiver")?;
                let parameter = named_children(receiver)
                    .find(|child| child.kind() == "parameter_declaration")?;
                let receiver_type = base_type(parameter.child_by_field_name("type")?);
                return Some((
                    "method",
                    vec![
                        text(receiver_type, source).into(),
                        text(name?, source).into(),
                    ],
                ));
            }
            "type_spec" | "type_alias" => match node.child_by_field_name("type")?.kind() {
                "struct_type" => "struct",
                "interface_type" => "interface",
                _ => "type",
            },
            "method_elem" => "method",
            _ => return None,
        },
        Syntax::Java => match tag {
            "class_declaration" | "record_declaration" => "class",
            "interface_declaration" | "annotation_type_declaration" => "interface",
            "enum_declaration" => "enum",
            "method_declaration" | "annotation_type_element_declaration" => "method",
            "constructor_declaration" | "compact_constructor_declaration" => "constructor",
            _ => return None,
        },
        Syntax::C | Syntax::Cpp => match tag {
            "function_definition" => {
                name = declarator_name(node.child_by_field_name("declarator")?);
                if in_type { "method" } else { "function" }
            }
            "function_declarator" => {
                if !declares_function(node) {
                    return None;
                }
                // Only declarations, never parameter types or calls. The outer declarator
                // represents functions returning pointers; inner ones belong to its name.
                let mut parent = node.parent()?;
                while matches!(
                    parent.kind(),
                    "pointer_declarator"
                        | "reference_declarator"
                        | "parenthesized_declarator"
                        | "attributed_declarator"
                ) {
                    parent = parent.parent()?;
                }
                if !matches!(parent.kind(), "declaration" | "field_declaration") {
                    return None;
                }
                name = declarator_name(node.child_by_field_name("declarator")?);
                if in_type { "method" } else { "function" }
            }
            "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
                node.child_by_field_name("body")?;
                match tag {
                    "class_specifier" => "class",
                    "struct_specifier" => "struct",
                    "union_specifier" => "union",
                    _ => "enum",
                }
            }
            "namespace_definition" => "namespace",
            "alias_declaration" => "type",
            _ => return None,
        },
        Syntax::Ruby => match tag {
            "method" => "method",
            "singleton_method" => {
                let object = node.child_by_field_name("object")?;
                return Some((
                    "method",
                    vec![text(object, source).into(), text(name?, source).into()],
                ));
            }
            "class" => "class",
            "singleton_class" => {
                name = node.child_by_field_name("value");
                "class"
            }
            "module" => "module",
            _ => return None,
        },
        Syntax::Bash => match tag {
            "function_definition" => "function",
            _ => return None,
        },
    };
    let name = name?;
    // Destructuring and computed property expressions have no literal declaration name.
    if matches!(
        name.kind(),
        "object_pattern" | "array_pattern" | "computed_property_name"
    ) {
        return None;
    }
    Some((kind, qualified_name(name, source)))
}

fn qualified_name(node: Node<'_>, source: &str) -> Vec<String> {
    if node.kind() == "nested_namespace_specifier" {
        return named_children(node)
            .flat_map(|child| qualified_name(child, source))
            .collect();
    }
    if matches!(node.kind(), "nested_identifier" | "member_expression") {
        let mut names = node
            .child_by_field_name("object")
            .map(|object| qualified_name(object, source))
            .unwrap_or_default();
        if let Some(property) = node.child_by_field_name("property") {
            names.push(text(property, source).into());
            return names;
        }
    }
    if matches!(
        node.kind(),
        "qualified_identifier" | "scoped_type_identifier" | "scope_resolution"
    ) {
        let mut names = node
            .child_by_field_name("scope")
            .or_else(|| node.child_by_field_name("path"))
            .map(|scope| qualified_name(scope, source))
            .unwrap_or_default();
        if let Some(name) = node.child_by_field_name("name") {
            names.extend(qualified_name(name, source));
            return names;
        }
    }
    vec![text(node, source).into()]
}

fn base_type(mut node: Node<'_>) -> Node<'_> {
    loop {
        match node.kind() {
            "generic_type" | "pointer_type" | "reference_type" => {
                let next = node.child_by_field_name("type").or_else(|| {
                    named_children(node)
                        .find(|child| !matches!(child.kind(), "lifetime" | "mutable_specifier"))
                });
                if let Some(next) = next {
                    node = next;
                } else {
                    return node;
                }
            }
            _ => return node,
        }
    }
}

fn declarator_name(mut node: Node<'_>) -> Option<Node<'_>> {
    loop {
        if matches!(
            node.kind(),
            "identifier"
                | "field_identifier"
                | "qualified_identifier"
                | "destructor_name"
                | "operator_name"
                | "type_identifier"
        ) {
            return Some(node);
        }
        node = inner_declarator(node)?;
    }
}

fn inner_declarator(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("declarator").or_else(|| {
        matches!(
            node.kind(),
            "parenthesized_declarator" | "attributed_declarator"
        )
        .then(|| named_children(node).find(|child| child.kind() != "attribute_declaration"))
        .flatten()
    })
}

fn declares_function(mut node: Node<'_>) -> bool {
    // The declarator operation nearest the name determines whether this is a
    // function or a variable holding a function pointer/reference.
    let mut function = false;
    loop {
        match node.kind() {
            "function_declarator" => function = true,
            "pointer_declarator" | "reference_declarator" | "array_declarator" => function = false,
            _ => {}
        }
        match inner_declarator(node) {
            Some(inner) => node = inner,
            None => return function,
        }
    }
}

fn declaration_range(mut node: Node<'_>, syntax: Syntax) -> (usize, usize) {
    if syntax == Syntax::Go
        && matches!(node.kind(), "type_spec" | "type_alias")
        && let Some(parent) = node.parent()
        && parent.kind() == "type_declaration"
        && named_children(parent)
            .filter(|child| matches!(child.kind(), "type_spec" | "type_alias"))
            .count()
            == 1
    {
        node = parent;
    }
    if matches!(syntax, Syntax::C | Syntax::Cpp) && node.kind() == "function_declarator" {
        while !matches!(node.kind(), "declaration" | "field_declaration") {
            node = node
                .parent()
                .expect("function declarator has a declaration parent");
        }
    }
    if matches!(
        syntax,
        Syntax::JavaScript | Syntax::TypeScript | Syntax::Tsx
    ) && node.kind() == "variable_declarator"
        && let Some(parent) = node.parent()
        && matches!(
            parent.kind(),
            "lexical_declaration" | "variable_declaration"
        )
        && named_children(parent)
            .filter(|child| child.kind() == "variable_declarator")
            .count()
            == 1
    {
        node = parent;
    }
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "decorated_definition"
                | "export_statement"
                | "ambient_declaration"
                | "template_declaration"
        ) {
            node = parent;
        } else {
            break;
        }
    }
    let mut start = node.start_byte();
    if matches!(syntax, Syntax::TypeScript | Syntax::Tsx) && node.kind() == "method_definition" {
        // In the TypeScript grammar method decorators are class_body siblings.
        let mut previous = node.prev_named_sibling();
        while let Some(sibling) = previous {
            if sibling.kind() == "decorator" {
                start = sibling.start_byte();
            } else if sibling.kind() != "comment" {
                break;
            }
            previous = sibling.prev_named_sibling();
        }
    }
    if syntax == Syntax::Rust {
        // Rust outer attributes are siblings. Inner attributes belong to the container.
        let mut previous = node.prev_named_sibling();
        while let Some(sibling) = previous {
            if sibling.kind() == "attribute_item" || sibling.child_by_field_name("outer").is_some()
            {
                start = sibling.start_byte();
            } else if !matches!(sibling.kind(), "line_comment" | "block_comment") {
                break;
            }
            previous = sibling.prev_named_sibling();
        }
    }
    (start, node.end_byte())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ResourceState;
    use serde_json::json;

    fn observation(resource: &str, source: &[u8]) -> Observation {
        Observation::new(
            resource.into(),
            source,
            "1".into(),
            "file".into(),
            ResourceState {
                exists: true,
                revision: Value::Null,
            },
        )
    }

    fn selection(resource: &str, source: &str, query: Value) -> Result<Value> {
        resolve(
            "code",
            &RegionSelector {
                extension: "code".into(),
                resource: Some(resource.into()),
                query,
            },
            &observation(resource, source.as_bytes()),
            source.as_bytes(),
        )
    }

    fn selected(resource: &str, source: &str, query: Value) -> String {
        selection(resource, source, query).unwrap()["content"]["text"]
            .as_str()
            .unwrap()
            .into()
    }

    fn error(resource: &str, source: &str, query: Value) -> String {
        format!("{:#}", selection(resource, source, query).unwrap_err())
    }

    fn region(value: Value, observation: &Observation) -> Region {
        Region {
            domain: value["domain"].as_str().unwrap().into(),
            identity: value["identity"].clone(),
            content: value["content"].clone(),
            observations: vec![observation.id.clone()],
        }
    }

    #[test]
    fn declarations_from_every_grammar() {
        let cases = [
            ("x.rs", "fn run() {}", "function"),
            ("x.py", "def run():\n    pass", "function"),
            ("x.js", "function run() {}", "function"),
            ("x.ts", "function run(): void {}", "function"),
            ("x.tsx", "function run() { return <div/>; }", "function"),
            ("x.go", "package sample\nfunc run() {}", "function"),
            ("x.java", "class Main { void run() {} }", "method"),
            ("x.c", "int run(void) { return 0; }", "function"),
            ("x.cpp", "int run() { return 0; }", "function"),
            ("x.rb", "def run\n  nil\nend", "method"),
            ("x.sh", "run() { :; }", "function"),
        ];
        for (resource, source, kind) in cases {
            let result = selection(resource, source, json!({"symbol":"run","kind":kind}))
                .unwrap_or_else(|error| panic!("{resource}: {error:#}"));
            let expected = match resource {
                "x.go" => "func run() {}",
                "x.java" => "void run() {}",
                _ => source,
            };
            assert_eq!(result["content"]["text"], expected, "{resource}");
            assert_eq!(result["domain"], "monika.bytes.v1");
            assert!(result.get("observations").is_none());
        }
    }

    #[test]
    fn whole_file_still_parses_and_infers_url_paths() {
        let source = "// 日本語\nfn run() {}\n";
        assert_eq!(
            selected("https://example.test/x.rs?download=1", source, Value::Null),
            source
        );
        assert_eq!(
            selected("source", source, json!({"language":"rust"})),
            source
        );
        assert_eq!(selected("x.rs", "", Value::Null), "");
        assert!(error("x.rs", "fn broken( {", Value::Null).contains("syntax error"));
        assert!(error("unknown", source, Value::Null).contains("cannot infer language"));
        assert!(error("x.h", "int run(void);", Value::Null).contains("ambiguous"));
        assert_eq!(
            selected("x.h", "int run(void);", json!({"language":"c"})),
            "int run(void);"
        );
    }

    #[test]
    fn rust_attributes_doc_comments_and_impl_scopes() {
        let source = "//! Module docs\n/// Function docs\n#[inline]\npub fn run() {}\nstruct Worker;\nimpl<T> Worker {\n    #[must_use]\n    fn task() -> bool { true }\n}\n";
        assert_eq!(
            selected("x.rs", source, json!({"symbol":"run"})),
            "/// Function docs\n#[inline]\npub fn run() {}"
        );
        assert_eq!(
            selected(
                "x.rs",
                source,
                json!({"symbol":"task","kind":"method","scope":["Worker"]})
            ),
            "#[must_use]\n    fn task() -> bool { true }"
        );
        assert_eq!(
            selected("x.rs", source, json!({"symbol":"Worker","kind":"struct"})),
            "struct Worker;"
        );
        assert!(
            selected("x.rs", source, json!({"symbol":"Worker","kind":"impl"}))
                .starts_with("impl<T> Worker")
        );
    }

    #[test]
    fn python_decorators_nested_names_and_occurrences() {
        let source = "# def run(): pass\nclass First:\n    @property\n    def run(self):\n        return 1\n\nclass Second:\n    @wrap(1)\n    async def run(self):\n        return 2\n";
        assert!(error("x.py", source, json!({"symbol":"run"})).contains("ambiguous"));
        let first = selected("x.py", source, json!({"symbol":"run","scope":["First"]}));
        assert_eq!(first, "@property\n    def run(self):\n        return 1");
        assert_eq!(
            first,
            selected("x.py", source, json!({"symbol":"run","occurrence":1}))
        );
        assert!(
            selected("x.py", source, json!({"symbol":"run","occurrence":2}))
                .starts_with("@wrap(1)")
        );
        assert!(
            error("x.py", source, json!({"symbol":"run","occurrence":3}))
                .contains("occurrence 3 out of range")
        );
        assert_eq!(
            selected(
                "x.py",
                source,
                json!({"symbol":"run","scope":["Second"],"kind":"method","occurrence":1})
            ),
            selected("x.py", source, json!({"symbol":"run","occurrence":2}))
        );
        assert!(
            error(
                "x.py",
                source,
                json!({"symbol":"run","scope":["First"],"occurrence":2})
            )
            .contains("occurrence 2 out of range")
        );
        assert!(
            error("x.py", source, json!({"symbol":"run","scope":[]})).contains("no declaration")
        );
        let nested = "def outer():\n    def inner():\n        pass\n    return inner\n";
        assert_eq!(
            selected(
                "x.py",
                nested,
                json!({"symbol":"inner","scope":["outer"],"kind":"function"})
            ),
            "def inner():\n        pass"
        );
    }

    #[test]
    fn javascript_and_typescript_exports_bound_functions_and_members() {
        let source =
            "export const run = (x) => x + 1;\nclass Worker { async task() { return 1; } }";
        assert_eq!(
            selected("x.js", source, json!({"symbol":"run"})),
            "export const run = (x) => x + 1;"
        );
        assert_eq!(
            selected(
                "x.js",
                source,
                json!({"symbol":"task","scope":["Worker"],"kind":"method"})
            ),
            "async task() { return 1; }"
        );
        let source = "export namespace App { export interface Worker { run(): void; } }";
        assert_eq!(
            selected(
                "x.ts",
                source,
                json!({"symbol":"run","scope":["App","Worker"]})
            ),
            "run(): void"
        );
        assert_eq!(
            selected(
                "x.ts",
                source,
                json!({"symbol":"Worker","kind":"interface"})
            ),
            "export interface Worker { run(): void; }"
        );
        let source = "const a = () => 1, b = () => 2;";
        assert_eq!(
            selected("x.js", source, json!({"symbol":"b"})),
            "b = () => 2"
        );
        assert_eq!(
            selected(
                "x.js",
                "const Worker = class { run() {} };",
                json!({"symbol":"run","scope":["Worker"]})
            ),
            "run() {}"
        );
    }

    #[test]
    fn typescript_decorators_and_nested_namespaces() {
        let source = "@sealed\nexport class Worker {\n  @trace\n  run() {}\n}\nnamespace A.B.C { export function task() {} }";
        assert_eq!(
            selected("x.ts", source, json!({"symbol":"Worker"})),
            "@sealed\nexport class Worker {\n  @trace\n  run() {}\n}"
        );
        assert_eq!(
            selected("x.ts", source, json!({"symbol":"run"})),
            "@trace\n  run() {}"
        );
        assert_eq!(
            selected(
                "x.ts",
                source,
                json!({"symbol":"task","scope":["A","B","C"]})
            ),
            "export function task() {}"
        );
    }

    #[test]
    fn bound_class_fields_and_ruby_singleton_class_have_scopes() {
        for resource in ["x.js", "x.ts", "x.tsx"] {
            let source = "class Worker { run = () => 1; }";
            assert_eq!(
                selected(
                    resource,
                    source,
                    json!({"symbol":"run","kind":"method","scope":["Worker"]})
                ),
                "run = () => 1"
            );
        }
        let source = "class Worker\n  class << self\n    def run; 1; end\n  end\nend";
        assert_eq!(
            selected(
                "x.rb",
                source,
                json!({"symbol":"run","scope":["Worker","self"]})
            ),
            "def run; 1; end"
        );
    }

    #[test]
    fn go_receivers_and_interface_methods() {
        let source = "package sample\ntype Worker[T any] struct {}\nfunc (w *Worker[T]) Run() {}\ntype Runner interface { Run() }\n";
        assert!(error("x.go", source, json!({"symbol":"Run"})).contains("ambiguous"));
        assert_eq!(
            selected("x.go", source, json!({"symbol":"Run","scope":["Worker"]})),
            "func (w *Worker[T]) Run() {}"
        );
        assert_eq!(
            selected("x.go", source, json!({"symbol":"Run","scope":["Runner"]})),
            "Run()"
        );
        assert_eq!(
            selected("x.go", source, json!({"symbol":"Worker","kind":"struct"})),
            "type Worker[T any] struct {}"
        );
    }

    #[test]
    fn java_annotations_constructors_and_overloads() {
        let source =
            "class Worker { Worker() {} @Override public void run() {} public void run(int x) {} }";
        assert_eq!(
            selected(
                "x.java",
                source,
                json!({"symbol":"Worker","kind":"constructor","scope":["Worker"]})
            ),
            "Worker() {}"
        );
        assert!(error("x.java", source, json!({"symbol":"run"})).contains("ambiguous"));
        assert_eq!(
            selected("x.java", source, json!({"symbol":"run","occurrence":1})),
            "@Override public void run() {}"
        );
    }

    #[test]
    fn c_declarators_do_not_confuse_function_pointer_variables_and_types() {
        let source = "struct Worker { int field; };\nstruct Worker *reference;\nint (*callback)(int);\nint *run(void);\nint (*factory(void))(int);\nint execute(int (*cb)(int)) { return cb(1); }";
        assert_eq!(
            selected("x.c", source, json!({"symbol":"Worker"})),
            "struct Worker { int field; }"
        );
        assert_eq!(
            selected("x.c", source, json!({"symbol":"run"})),
            "int *run(void);"
        );
        assert_eq!(
            selected("x.c", source, json!({"symbol":"factory"})),
            "int (*factory(void))(int);"
        );
        assert!(error("x.c", source, json!({"symbol":"callback"})).contains("no declaration"));
        assert!(error("x.c", source, json!({"symbol":"cb"})).contains("no declaration"));
        assert!(selected("x.c", source, json!({"symbol":"execute"})).starts_with("int execute"));
    }

    #[test]
    fn cpp_namespaces_templates_and_out_of_line_definitions() {
        let source = "namespace A::B { template<class T> class Worker { public: void run() {} }; }\nvoid A::B::task() {}";
        assert_eq!(
            selected(
                "x.cpp",
                source,
                json!({"symbol":"run","scope":["A","B","Worker"]})
            ),
            "void run() {}"
        );
        assert!(
            selected("x.cpp", source, json!({"symbol":"Worker"})).starts_with("template<class T>")
        );
        assert_eq!(
            selected("x.cpp", source, json!({"symbol":"task","scope":["A","B"]})),
            "void A::B::task() {}"
        );
    }

    #[test]
    fn c_and_cpp_attributes_templates_and_overloads_keep_declaration_boundaries() {
        let source = "__attribute__((unused)) static int run(void) { return 0; }";
        assert_eq!(selected("x.c", source, json!({"symbol":"run"})), source);
        let source = "template <typename T>\n[[nodiscard]] T run(T x) { return x; }";
        assert_eq!(selected("x.cpp", source, json!({"symbol":"run"})), source);
        let source = "class Worker { public: Worker() {} void run() {} void run(int x) {} };";
        assert_eq!(
            selected(
                "x.cpp",
                source,
                json!({"symbol":"Worker","kind":"method","scope":["Worker"]})
            ),
            "Worker() {}"
        );
        assert!(error("x.cpp", source, json!({"symbol":"run"})).contains("ambiguous"));
        assert_eq!(
            selected("x.cpp", source, json!({"symbol":"run","occurrence":2})),
            "void run(int x) {}"
        );
    }

    #[test]
    fn ruby_modules_and_singleton_methods() {
        let source =
            "module App\n  class Worker\n    def run; 1; end\n    def self.run; 2; end\n  end\nend";
        assert!(error("x.rb", source, json!({"symbol":"run"})).contains("ambiguous"));
        assert_eq!(
            selected(
                "x.rb",
                source,
                json!({"symbol":"run","scope":["App","Worker"]})
            ),
            "def run; 1; end"
        );
        assert_eq!(
            selected(
                "x.rb",
                source,
                json!({"symbol":"run","scope":["App","Worker","self"]})
            ),
            "def self.run; 2; end"
        );
    }

    #[test]
    fn unicode_and_comments_select_only_syntax_declarations() {
        let source = "# def 計算(): pass\ndef 計算():\n    return '雪'\n";
        let result = selection("x.py", source, json!({"symbol":"計算"})).unwrap();
        let start = source.find("\ndef").unwrap() + 1;
        assert_eq!(
            result["identity"]["ranges"],
            json!([[start, source.len() - 1]])
        );
        assert_eq!(result["content"]["text"], "def 計算():\n    return '雪'");
        assert!(
            error(
                "x.rs",
                "// fn phantom() {}\nconst X: &str = \"fn phantom() {}\";",
                json!({"symbol":"phantom"})
            )
            .contains("no declaration")
        );
    }

    #[test]
    fn raw_query_filters_captures_and_normalizes_duplicate_or_nested_spans() {
        let source = "fn one() {}\nfn two() {}\n";
        let query = json!({"query":"((function_item name: (identifier) @name) @decl (#eq? @name \"two\"))", "capture":"decl"});
        assert_eq!(selected("x.rs", source, query), "fn two() {}");
        let query = json!({"query":"(function_item) @decl\n(function_item name: (identifier) @decl) @decl", "capture":"decl"});
        assert_eq!(selected("x.rs", source, query), "fn one() {}fn two() {}");
        let query = json!({"query":"(struct_item) @decl", "capture":"decl"});
        let result = selection("x.rs", source, query).unwrap();
        assert_eq!(result["identity"]["ranges"], json!([]));
        assert_eq!(result["content"]["text"], "");
    }

    #[test]
    fn rejects_malformed_queries_missing_captures_and_unevaluated_predicates() {
        let cases = [
            (json!("run"), "null or an object"),
            (json!({"symbol":"run","extra":true}), "unknown query field"),
            (json!({"language":"unknown"}), "unknown language"),
            (json!({"language":null}), "language must be a string"),
            (json!({"symbol":null}), "symbol must be a string"),
            (json!({"symbol":""}), "symbol must not be empty"),
            (
                json!({"symbol":"run","scope":"Worker"}),
                "scope must be an array",
            ),
            (
                json!({"symbol":"run","scope":[2]}),
                "scope entry must be a string",
            ),
            (
                json!({"symbol":"run","kind":"variable"}),
                "unknown declaration kind",
            ),
            (
                json!({"symbol":"run","occurrence":0}),
                "positive integer (one-based)",
            ),
            (
                json!({"symbol":"run","occurrence":-1}),
                "positive integer (one-based)",
            ),
            (
                json!({"symbol":"run","occurrence":1.5}),
                "positive integer (one-based)",
            ),
            (
                json!({"symbol":"run","query":"(identifier) @x"}),
                "cannot contain query",
            ),
            (
                json!({"query":"(identifier) @x","kind":"function","capture":"x"}),
                "require symbol",
            ),
            (json!({"capture":"x"}), "require a selection"),
            (json!({"query":"(identifier) @x"}), "requires capture"),
            (
                json!({"query":"(identifier) @x","capture":"@x"}),
                "omit the @",
            ),
            (
                json!({"query":"(identifier) @x","capture":"missing"}),
                "no capture named",
            ),
            (
                json!({"query":"(nonexistent_node) @x","capture":"x"}),
                "invalid Tree-sitter query",
            ),
            (
                json!({"query":"(","capture":"x"}),
                "invalid Tree-sitter query",
            ),
            (
                json!({"query":"((identifier) @x (#custom? @x))","capture":"x"}),
                "unsupported Tree-sitter predicate",
            ),
            (
                json!({"query":"((identifier) @x (#is? local))","capture":"x"}),
                "unsupported Tree-sitter predicate",
            ),
            (
                json!({"query":"((identifier) @x (#set! role \"test\"))","capture":"x"}),
                "unsupported Tree-sitter predicate",
            ),
        ];
        for (query, expected) in cases {
            let message = error("x.rs", "fn run() {}", query.clone());
            assert!(
                message.contains(expected),
                "{query}: {message}; wanted {expected}"
            );
        }
    }

    #[test]
    fn rejects_invalid_source_even_outside_selection() {
        let message = error("x.rs", "fn run() {}\nfn broken( {", json!({"symbol":"run"}));
        assert!(
            message.contains("syntax error") && message.contains("line 2"),
            "{message}"
        );
        let bytes = b"fn run() {}\xff";
        let selector = RegionSelector {
            extension: "code".into(),
            resource: Some("x.rs".into()),
            query: Value::Null,
        };
        let message = resolve("code", &selector, &observation("x.rs", bytes), bytes)
            .unwrap_err()
            .to_string();
        assert!(message.contains("valid UTF-8"));
        let selector = RegionSelector {
            resource: None,
            ..selector
        };
        assert!(resolve("code", &selector, &observation("x.rs", b""), b"").is_err());
    }

    #[test]
    fn comparison_shares_byte_domain_and_observation_identity() {
        let source = "mod app { fn one() {} fn two() {} }";
        let observation = observation("x.rs", source.as_bytes());
        let resolve_region =
            |query| region(selection("x.rs", source, query).unwrap(), &observation);
        let module = resolve_region(json!({"symbol":"app"}));
        let first = resolve_region(json!({"symbol":"one"}));
        let second = resolve_region(json!({"symbol":"two"}));
        assert_eq!(
            compare("code", &module, &first).unwrap(),
            Relation::Contains
        );
        assert_eq!(compare("code", &first, &module).unwrap(), Relation::Within);
        assert_eq!(compare("code", &first, &first).unwrap(), Relation::Equal);
        assert_eq!(
            compare("code", &first, &second).unwrap(),
            Relation::Incomparable
        );
        let spans = first.identity["ranges"][0].as_array().unwrap();
        let start = spans[0].as_u64().unwrap() as usize;
        let end = spans[1].as_u64().unwrap() as usize;
        let byte_region = |ranges| {
            region(
                super::super::span_region(&observation, source.as_bytes(), ranges).unwrap(),
                &observation,
            )
        };
        assert_eq!(
            compare("code", &first, &byte_region(vec![(start, end)])).unwrap(),
            Relation::Equal
        );
        assert_eq!(
            compare("code", &first, &byte_region(vec![(start + 1, end + 1)])).unwrap(),
            Relation::Incomparable
        );
        let empty = byte_region(vec![]);
        assert_eq!(compare("code", &first, &empty).unwrap(), Relation::Contains);
        let mut other_observation = first.clone();
        other_observation.identity["observation"] = json!("different");
        assert_eq!(
            compare("code", &first, &other_observation).unwrap(),
            Relation::Incomparable
        );
    }

    #[test]
    fn capture_content_ignores_changes_before_and_after_declaration() {
        let before = "fn run() { println!(\"雪\"); }\n";
        let after = "// added\nfn unrelated() {}\nfn run() { println!(\"雪\"); }\nfn tail() {}";
        let query = json!({"symbol":"run"});
        let left = selection("x.rs", before, query.clone()).unwrap();
        let right = selection("x.rs", after, query).unwrap();
        assert_eq!(left["content"], right["content"]);
        assert_ne!(left["identity"], right["identity"]);
    }

    #[test]
    fn manifest_declares_shared_domain_and_rejects_other_names() {
        let manifest = manifest("code").unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.revision, "1");
        assert_eq!(manifest.domains, ["monika.bytes.v1"]);
        assert!(super::manifest("rust").is_none());
    }
}
