//! Structured data and delimited tables. Public query schemas: docs/extensions/data.md.

use crate::extensions::process::Manifest;
use crate::model::{Observation, Region, RegionSelector, Relation};
use anyhow::{Context, Result, bail, ensure};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value, json, value::RawValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

fn domain(name: &str) -> Option<&'static str> {
    match name {
        "json" => Some("monika.json.v1"),
        "jsonl" => Some("monika.jsonl.v1"),
        "yaml" => Some("monika.yaml.v1"),
        "toml" => Some("monika.toml.v1"),
        "csv" => Some("monika.csv.v1"),
        "tsv" => Some("monika.tsv.v1"),
        _ => None,
    }
}

pub fn manifest(name: &str) -> Option<Manifest> {
    Some(Manifest {
        name: name.into(),
        revision: "1".into(),
        operations: vec!["resolve".into(), "compare".into()],
        domains: vec![domain(name)?.into()],
        description: match name {
            "csv" | "tsv" => "Select UTF-8 table cells; query null/{} selects all data, or {headers:true,rows:[0],columns:[\"name\"],where:[{column:\"status\",equals:\"ok\"}]}. Headers default true; indices are zero-based. Content {columns:[names] or null,rows:[[strings]]}; empty content {columns:[],rows:[]}. See docs/extensions/data.md.",
            "jsonl" => "Select a strict JSON Lines record array with null/{}, {pointer:\"/0/name\"}, or {lines:[1,3]} (one-based physical lines). Content {values:[selected subtrees]}; empty lines select values:[]. See docs/extensions/data.md.",
            "toml" => "Select TOML with null/{} or {pointer:\"/package/name\"} (RFC 6901). Content {values:[typed nodes]}, each {type,value}; datetime retains its TOML type. See docs/extensions/data.md.",
            _ => "Select a structured UTF-8 value with null/{} or {pointer:\"/items/0\"} (RFC 6901). Content {values:[selected subtree]}; null is a value and missing paths are errors. See docs/extensions/data.md.",
        }.into(),
    })
}

pub fn resolve(
    name: &str,
    selector: &RegionSelector,
    observation: &Observation,
    bytes: &[u8],
) -> Result<Value> {
    let domain = domain(name).with_context(|| format!("unknown data extension: {name}"))?;
    let source = std::str::from_utf8(bytes).with_context(|| format!("{name} requires UTF-8"))?;
    if matches!(name, "csv" | "tsv") {
        return resolve_table(name, domain, &selector.query, observation, source);
    }
    let query = query_object(&selector.query)?;
    allowed_fields(
        &query,
        if name == "jsonl" {
            &["pointer", "lines"]
        } else {
            &["pointer"]
        },
    )?;
    ensure!(
        !(query.contains_key("pointer") && query.contains_key("lines")),
        "pointer and lines are mutually exclusive"
    );
    let root = match name {
        "json" => Node::from_json(parse_json(source).context("invalid JSON")?),
        "jsonl" => Node::Array(parse_jsonl(source)?),
        "yaml" => {
            validate_yaml_tags(source)?;
            let shape = YamlSeed(source)
                .deserialize(serde_yaml_ng::Deserializer::from_str(source))
                .context("invalid or unsupported YAML")?
                .0;
            // A second pass reads the original numeric scalars as strings. The
            // deserializer's f64 dispatch must not round the captured values.
            Node::from_json(
                YamlNumbers(&shape)
                    .deserialize(serde_yaml_ng::Deserializer::from_str(source))
                    .context("unsupported YAML numeric representation")?,
            )
        }
        "toml" => Node::from_toml(toml::from_str(source).context("invalid TOML")?)?,
        _ => unreachable!(),
    };
    let (mut paths, values) = if let Some(lines) = query.get("lines") {
        let Node::Array(records) = &root else {
            unreachable!()
        };
        let lines = index_set(lines, "lines", records.len(), 1)?;
        let paths: Vec<_> = lines.iter().map(|i| vec![i.to_string()]).collect();
        let values: Vec<_> = lines.iter().map(|i| records[*i].content(false)).collect();
        (paths, values)
    } else {
        let pointer = match query.get("pointer") {
            Some(value) => value.as_str().context("pointer must be a string")?,
            None => "",
        };
        let path = pointer_tokens(pointer)?;
        let value = root.at(&path)?.content(name == "toml");
        (vec![path], vec![value])
    };
    for value in &values {
        validate_json_content(value)?;
    }
    paths.sort();
    Ok(json!({
        "domain": domain,
        "identity": TreeIdentity { observation: observation.id.clone(), paths },
        "content": {"values": values},
    }))
}

fn validate_json_content(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                ensure!(
                    !matches!(
                        key.as_str(),
                        "$serde_json::private::Number" | "$serde_json::private::RawValue"
                    ),
                    "selected mapping key {key:?} conflicts with the JSON transport's reserved keys; select its value directly with a pointer"
                );
                validate_json_content(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_json_content(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn query_object(query: &Value) -> Result<Map<String, Value>> {
    if query.is_null() {
        Ok(Map::new())
    } else {
        query
            .as_object()
            .cloned()
            .context("query must be null or an object")
    }
}

fn allowed_fields(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    for key in object.keys() {
        ensure!(
            allowed.contains(&key.as_str()),
            "unknown query field: {key}"
        );
    }
    Ok(())
}

fn index(value: &Value, label: &str) -> Result<usize> {
    let n = value.as_u64().with_context(|| {
        format!("{label} must be a nonnegative integer (not a boolean or decimal)")
    })?;
    usize::try_from(n).with_context(|| format!("{label} is too large"))
}

/// Returns zero-based indices, even for a one-based query.
fn index_set(value: &Value, label: &str, len: usize, base: usize) -> Result<BTreeSet<usize>> {
    let values = value
        .as_array()
        .with_context(|| format!("{label} must be an array of integers"))?;
    values
        .iter()
        .map(|value| {
            let n = index(value, label)?;
            let offset = n
                .checked_sub(base)
                .filter(|offset| *offset < len)
                .with_context(|| {
                    format!("{label} index {n} is out of range ({len} items; base {base})")
                })?;
            Ok(offset)
        })
        .collect()
}

fn pointer_tokens(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    ensure!(
        pointer.starts_with('/'),
        "RFC 6901 pointer must be empty or start with '/' (URI fragments are not accepted)"
    );
    pointer[1..]
        .split('/')
        .map(|token| {
            let mut decoded = String::new();
            let mut chars = token.chars();
            while let Some(ch) = chars.next() {
                if ch == '~' {
                    decoded.push(match chars.next() {
                        Some('0') => '~',
                        Some('1') => '/',
                        _ => bail!(
                            "invalid RFC 6901 escape in pointer token {token:?}; expected ~0 or ~1"
                        ),
                    });
                } else {
                    decoded.push(ch);
                }
            }
            Ok(decoded)
        })
        .collect()
}

fn array_index(token: &str, len: usize) -> Result<usize> {
    ensure!(
        token == "0"
            || (!token.starts_with('0')
                && !token.is_empty()
                && token.bytes().all(|b| b.is_ascii_digit())),
        "invalid array index {token:?}; use 0 or digits without leading zeros"
    );
    let n: usize = token
        .parse()
        .with_context(|| format!("array index {token:?} is too large"))?;
    ensure!(n < len, "array index {n} is out of range ({len} elements)");
    Ok(n)
}

enum Node {
    Scalar(Value),
    Array(Vec<Node>),
    Object(BTreeMap<String, Node>),
    Datetime(String),
}

impl Node {
    fn from_json(value: Value) -> Self {
        match value {
            Value::Array(array) => Self::Array(array.into_iter().map(Self::from_json).collect()),
            Value::Object(object) => Self::Object(
                object
                    .into_iter()
                    .map(|(k, v)| (k, Self::from_json(v)))
                    .collect(),
            ),
            value => Self::Scalar(value),
        }
    }

    fn from_toml(value: toml::Value) -> Result<Self> {
        Ok(match value {
            toml::Value::String(v) => Self::Scalar(Value::String(v)),
            toml::Value::Integer(v) => Self::Scalar(json!(v)),
            toml::Value::Float(v) => Self::Scalar(Value::Number(
                Number::from_f64(v).context("nonfinite TOML floats are unsupported")?,
            )),
            toml::Value::Boolean(v) => Self::Scalar(json!(v)),
            toml::Value::Datetime(v) => Self::Datetime(v.to_string()),
            toml::Value::Array(v) => {
                Self::Array(v.into_iter().map(Self::from_toml).collect::<Result<_>>()?)
            }
            toml::Value::Table(v) => Self::Object(
                v.into_iter()
                    .map(|(k, v)| Ok((k, Self::from_toml(v)?)))
                    .collect::<Result<_>>()?,
            ),
        })
    }

    fn at(&self, tokens: &[String]) -> Result<&Self> {
        let mut node = self;
        for token in tokens {
            node = match node {
                Self::Object(map) => map
                    .get(token)
                    .with_context(|| format!("pointer member {token:?} does not exist"))?,
                Self::Array(array) => &array[array_index(token, array.len())?],
                _ => bail!("pointer cannot descend through a scalar at token {token:?}"),
            };
        }
        Ok(node)
    }

    fn content(&self, typed: bool) -> Value {
        let (kind, value) = match self {
            Self::Scalar(value) => (
                match value {
                    Value::String(_) => "string",
                    Value::Bool(_) => "boolean",
                    Value::Number(n) if n.is_i64() => "integer",
                    Value::Number(_) => "float",
                    Value::Null => "null",
                    _ => unreachable!(),
                },
                value.clone(),
            ),
            Self::Array(array) => (
                "array",
                Value::Array(array.iter().map(|v| v.content(typed)).collect()),
            ),
            Self::Object(map) => (
                "table",
                Value::Object(
                    map.iter()
                        .map(|(k, v)| (k.clone(), v.content(typed)))
                        .collect(),
                ),
            ),
            Self::Datetime(value) => ("datetime", Value::String(value.clone())),
        };
        if typed {
            json!({"type":kind, "value":value})
        } else {
            value
        }
    }
}

/// RawValue keeps arbitrary-precision JSON numbers intact while this visitor detects
/// duplicate decoded object keys, including names resembling serde's private tokens.
fn parse_json(source: &str) -> Result<Value> {
    let raw: Box<RawValue> = serde_json::from_str(source)?;
    parse_json_raw(&raw, 0)
}

fn parse_json_raw(raw: &RawValue, depth: usize) -> Result<Value> {
    ensure!(depth < 128, "JSON nesting exceeds 128 levels");
    let source = raw.get();
    match source.as_bytes().first() {
        Some(b'{') => {
            struct Object(usize);
            impl<'de> Visitor<'de> for Object {
                type Value = Value;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a JSON object with unique keys")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Value, A::Error> {
                    let mut result = Map::new();
                    while let Some((key, raw)) = map.next_entry::<String, Box<RawValue>>()? {
                        if result.contains_key(&key) {
                            return Err(de::Error::custom(format!("duplicate JSON key {key:?}")));
                        }
                        result.insert(
                            key,
                            parse_json_raw(&raw, self.0 + 1).map_err(de::Error::custom)?,
                        );
                    }
                    Ok(Value::Object(result))
                }
            }
            Ok(serde_json::Deserializer::from_str(source).deserialize_map(Object(depth))?)
        }
        Some(b'[') => {
            let values: Vec<Box<RawValue>> = serde_json::from_str(source)?;
            Ok(Value::Array(
                values
                    .iter()
                    .map(|raw| parse_json_raw(raw, depth + 1))
                    .collect::<Result<_>>()?,
            ))
        }
        _ => Ok(serde_json::from_str(source)?),
    }
}

fn parse_jsonl(source: &str) -> Result<Vec<Node>> {
    source
        .split_terminator('\n')
        .enumerate()
        .map(|(line, text)| {
            let text = text.strip_suffix('\r').unwrap_or(text);
            ensure!(
                !text.trim().is_empty(),
                "JSONL line {} is blank; each line must contain a JSON value",
                line + 1
            );
            Ok(Node::from_json(parse_json(text).with_context(|| {
                format!("invalid JSONL at line {}", line + 1)
            })?))
        })
        .collect()
}

struct YamlJson(Value);

struct YamlSeed<'a>(&'a str);

struct YamlNumbers<'a>(&'a Value);

impl<'de> DeserializeSeed<'de> for YamlNumbers<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        match self.0 {
            Value::Number(_) => {
                let raw = String::deserialize(deserializer)?;
                yaml_number(&raw).map_err(de::Error::custom)
            }
            Value::Array(_) => deserializer.deserialize_seq(self),
            Value::Object(_) => deserializer.deserialize_map(self),
            _ => Value::deserialize(deserializer),
        }
    }
}

impl<'de> Visitor<'de> for YamlNumbers<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("the previously parsed YAML structure")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Value, A::Error> {
        let Value::Array(shape) = self.0 else {
            unreachable!()
        };
        let mut values = Vec::with_capacity(shape.len());
        for item in shape {
            values.push(seq.next_element_seed(YamlNumbers(item))?.ok_or_else(|| {
                de::Error::custom("YAML array length changed during numeric decoding")
            })?);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
        let Value::Object(shape) = self.0 else {
            unreachable!()
        };
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let item = shape
                .get(&key)
                .ok_or_else(|| de::Error::custom("YAML mapping changed during numeric decoding"))?;
            values.insert(key, map.next_value_seed(YamlNumbers(item))?);
        }
        Ok(Value::Object(values))
    }
}

fn yaml_number(raw: &str) -> Result<Value> {
    let (negative, unsigned) = if let Some(rest) = raw.strip_prefix('-') {
        (true, rest)
    } else {
        (false, raw.strip_prefix('+').unwrap_or(raw))
    };
    let sign = if negative { "-" } else { "" };
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = unsigned.strip_prefix(prefix) {
            let mut decimal = vec![0u32]; // little-endian decimal digits
            for digit in digits.chars() {
                let mut carry = digit
                    .to_digit(radix)
                    .context("invalid YAML radix integer")?;
                for place in &mut decimal {
                    carry += *place * radix;
                    *place = carry % 10;
                    carry /= 10;
                }
                while carry > 0 {
                    decimal.push(carry % 10);
                    carry /= 10;
                }
            }
            let digits: String = decimal
                .into_iter()
                .rev()
                .map(|d| char::from_digit(d, 10).unwrap())
                .collect();
            return Ok(serde_json::from_str(&format!("{sign}{digits}"))?);
        }
    }
    let exponent_start = unsigned.find(['e', 'E']).unwrap_or(unsigned.len());
    let (mantissa, exponent) = unsigned.split_at(exponent_start);
    let (integer, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, None), |(i, f)| (i, Some(f)));
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = match fraction {
        Some("") => ".0".to_owned(),
        Some(f) => format!(".{f}"),
        None => String::new(),
    };
    Ok(serde_json::from_str(&format!(
        "{sign}{integer}{fraction}{exponent}"
    ))?)
}

impl<'de> DeserializeSeed<'de> for YamlSeed<'de> {
    type Value = YamlJson;
    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<YamlJson, D::Error> {
        struct YamlVisitor<'a>(&'a str);
        impl<'de> Visitor<'de> for YamlVisitor<'de> {
            type Value = YamlJson;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(
                    "YAML with string keys, unique mappings, finite numbers and no custom tags",
                )
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<YamlJson, E> {
                self.visit_unit()
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(json!(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(json!(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(json!(v)))
            }
            fn visit_i128<E: de::Error>(self, v: i128) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(
                    serde_json::from_str(&v.to_string()).map_err(E::custom)?,
                ))
            }
            fn visit_u128<E: de::Error>(self, v: u128) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(
                    serde_json::from_str(&v.to_string()).map_err(E::custom)?,
                ))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<YamlJson, E> {
                let n = Number::from_f64(v)
                    .ok_or_else(|| E::custom("nonfinite YAML numbers are unsupported"))?;
                Ok(YamlJson(Value::Number(n)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<YamlJson, E> {
                Ok(YamlJson(Value::String(v.into())))
            }
            fn visit_borrowed_str<E: de::Error>(
                self,
                v: &'de str,
            ) -> std::result::Result<YamlJson, E> {
                // serde_yaml_ng lends undecorated plain/quoted scalar slices from
                // the input. Recover the style for numeric literals outside its
                // integer/f64 range; quoted or cooked strings remain strings.
                if yaml_numeric_literal(v) {
                    let offset = (v.as_ptr() as usize).checked_sub(self.0.as_ptr() as usize);
                    if let Some(offset) = offset.filter(|offset| {
                        *offset <= self.0.len()
                            && self.0.get(*offset..offset.saturating_add(v.len())) == Some(v)
                    }) {
                        let quoted =
                            offset > 0 && matches!(self.0.as_bytes()[offset - 1], b'\'' | b'"');
                        if !quoted {
                            return Ok(YamlJson(yaml_number(v).map_err(E::custom)?));
                        }
                    }
                }
                self.visit_str(v)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<YamlJson, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element_seed(YamlSeed(self.0))? {
                    values.push(value.0);
                }
                Ok(YamlJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<YamlJson, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key_seed(YamlSeed(self.0))? {
                    let Value::String(key) = key.0 else {
                        return Err(de::Error::custom("YAML mapping keys must be strings"));
                    };
                    if key == "<<" {
                        return Err(de::Error::custom(
                            "YAML merge key << is unsupported; write explicit mappings",
                        ));
                    }
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate YAML key {key:?}")));
                    }
                    values.insert(key, map.next_value_seed(YamlSeed(self.0))?.0);
                }
                Ok(YamlJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(YamlVisitor(self.0))
    }
}

fn yaml_numeric_literal(raw: &str) -> bool {
    let raw = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = raw.strip_prefix(prefix) {
            return !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix));
        }
    }
    let (mantissa, exponent) = raw
        .find(['e', 'E'])
        .map_or((raw, None), |i| (&raw[..i], Some(&raw[i + 1..])));
    if exponent.is_some_and(|e| {
        let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
        digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit())
    }) {
        return false;
    }
    let mut dot = false;
    let mut digit = false;
    for byte in mantissa.bytes() {
        if byte == b'.' && !dot {
            dot = true;
        } else if byte.is_ascii_digit() {
            digit = true;
        } else {
            return false;
        }
    }
    digit && (dot || exponent.is_some() || mantissa.len() == 1 || !mantissa.starts_with('0'))
}

/// serde_yaml_ng discards some global tags. Reserve tag syntax before deserializing
/// so an unsupported tagged value cannot silently turn into a plain value.
fn validate_yaml_tags(source: &str) -> Result<()> {
    let mut quote = None;
    let mut quote_indent = 0;
    let mut flow_depth = 0usize;
    let mut block_indent = None;
    for line in source.trim_start_matches('\u{feff}').lines() {
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        if let Some(parent_indent) = block_indent {
            if line.trim().is_empty() || indent > parent_indent {
                continue;
            }
            block_indent = None;
        }
        if indent < quote_indent && !line.trim().is_empty() {
            quote = None;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut scalar_started = false;
        let mut quoted_key = false;
        while i < bytes.len() {
            let ch = bytes[i];
            if let Some(q) = quote {
                if q == b'"' && ch == b'\\' {
                    i += 2;
                    continue;
                }
                if ch == q {
                    if q == b'\'' && bytes.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    quote = None;
                    scalar_started = true;
                    quoted_key = true;
                }
                i += 1;
                continue;
            }
            let boundary =
                i == 0 || bytes[i - 1].is_ascii_whitespace() || b"[{,:?".contains(&bytes[i - 1]);
            if ch == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
                break;
            }
            if ch == b'!' && boundary {
                bail!("explicit YAML tags are unsupported; quote literal text beginning with '!'");
            }
            if ch.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            let separated = bytes
                .get(i + 1)
                .is_none_or(|next| next.is_ascii_whitespace());
            if !scalar_started && matches!(ch, b'\'' | b'"') {
                quote = Some(ch);
                quote_indent = indent;
            } else if (!scalar_started && matches!(ch, b'-' | b'?') && separated)
                || (ch == b':' && (separated || (flow_depth > 0 && quoted_key)))
                || (flow_depth > 0 && ch == b',')
            {
                scalar_started = false;
                quoted_key = false;
            } else if !scalar_started && matches!(ch, b'[' | b'{') {
                flow_depth += 1;
            } else if flow_depth > 0 && matches!(ch, b']' | b'}') {
                flow_depth -= 1;
                scalar_started = true;
                quoted_key = true;
            } else if !scalar_started && ch == b'&' {
                // An anchor precedes a value; its name is not that value.
                i += 1;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !b",[]{}".contains(&bytes[i])
                {
                    i += 1;
                }
                continue;
            } else if !scalar_started
                && bytes[i..].starts_with(b"---")
                && bytes
                    .get(i + 3)
                    .is_none_or(|next| next.is_ascii_whitespace())
            {
                i += 3;
                continue;
            } else if !scalar_started && matches!(ch, b'|' | b'>') {
                let suffix = line[i + 1..].split('#').next().unwrap().trim();
                if suffix
                    .bytes()
                    .all(|b| matches!(b, b'1'..=b'9' | b'+' | b'-'))
                {
                    block_indent = Some(indent);
                }
                scalar_started = true;
            } else {
                scalar_started = true;
                quoted_key = false;
            }
            i += 1;
        }
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TreeIdentity {
    observation: String,
    paths: Vec<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TableIdentity {
    observation: String,
    headers: bool,
    cells: BTreeSet<(usize, usize)>,
}

pub fn compare(name: &str, left: &Region, right: &Region) -> Result<Relation> {
    let domain = domain(name).with_context(|| format!("unknown data extension: {name}"))?;
    ensure!(
        left.domain == domain && right.domain == domain,
        "{name} comparator requires domain {domain}"
    );
    if matches!(name, "csv" | "tsv") {
        let left: TableIdentity =
            serde_json::from_value(left.identity.clone()).context("invalid left table identity")?;
        let right: TableIdentity = serde_json::from_value(right.identity.clone())
            .context("invalid right table identity")?;
        if left.observation != right.observation || left.headers != right.headers {
            return Ok(Relation::Incomparable);
        }
        return Ok(subset_relation(
            left.cells.is_subset(&right.cells),
            right.cells.is_subset(&left.cells),
        ));
    }
    let left: TreeIdentity =
        serde_json::from_value(left.identity.clone()).context("invalid left tree identity")?;
    let right: TreeIdentity =
        serde_json::from_value(right.identity.clone()).context("invalid right tree identity")?;
    validate_paths(&left.paths)?;
    validate_paths(&right.paths)?;
    if left.observation != right.observation {
        return Ok(Relation::Incomparable);
    }
    let within = |a: &[Vec<String>], b: &[Vec<String>]| {
        a.iter()
            .all(|path| b.iter().any(|parent| path.starts_with(parent)))
    };
    Ok(subset_relation(
        within(&left.paths, &right.paths),
        within(&right.paths, &left.paths),
    ))
}

fn validate_paths(paths: &[Vec<String>]) -> Result<()> {
    for pair in paths.windows(2) {
        ensure!(
            pair[0] < pair[1] && !pair[1].starts_with(&pair[0]),
            "tree identity paths must be sorted, distinct and contain no ancestor/descendant pair"
        );
    }
    Ok(())
}

fn subset_relation(left_within: bool, right_within: bool) -> Relation {
    match (left_within, right_within) {
        (true, true) => Relation::Equal,
        (true, false) => Relation::Within,
        (false, true) => Relation::Contains,
        (false, false) => Relation::Incomparable,
    }
}

fn resolve_table(
    name: &str,
    domain: &str,
    query: &Value,
    observation: &Observation,
    source: &str,
) -> Result<Value> {
    let query = query_object(query)?;
    allowed_fields(&query, &["headers", "rows", "columns", "where"])?;
    let headers = match query.get("headers") {
        Some(v) => v.as_bool().context("headers must be a boolean")?,
        None => true,
    };
    let delimiter = if name == "csv" { b',' } else { b'\t' };
    validate_delimited(source.as_bytes(), delimiter)
        .with_context(|| format!("invalid {name} quoting"))?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .from_reader(source.as_bytes());
    let records = reader
        .records()
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("invalid {name} records"))?;
    let width = records.first().map_or(0, csv::StringRecord::len);
    let names: Vec<String> = if headers {
        records
            .first()
            .map(|r| r.iter().map(str::to_owned).collect())
            .unwrap_or_default()
    } else {
        vec![]
    };
    if headers {
        let mut seen = BTreeSet::new();
        for label in &names {
            ensure!(
                !label.is_empty(),
                "empty table header is ambiguous; use nonempty names or headers:false"
            );
            ensure!(
                seen.insert(label),
                "duplicate table header {label:?}; use unique names or headers:false"
            );
        }
    }
    let data = if headers && !records.is_empty() {
        &records[1..]
    } else {
        &records[..]
    };
    let mut rows: BTreeSet<usize> = match query.get("rows") {
        Some(v) => index_set(v, "rows", data.len(), 0)?,
        None => (0..data.len()).collect(),
    };
    let column = |value: &Value| -> Result<usize> {
        if let Some(label) = value.as_str() {
            ensure!(headers, "column names require headers:true");
            names
                .iter()
                .position(|name| name == label)
                .with_context(|| format!("unknown column name {label:?}"))
        } else {
            let n = index(value, "column")?;
            ensure!(
                n < width,
                "column index {n} is out of range ({width} columns)"
            );
            Ok(n)
        }
    };
    let columns: BTreeSet<usize> = match query.get("columns") {
        Some(v) => v
            .as_array()
            .context("columns must be an array of indices or header names")?
            .iter()
            .map(column)
            .collect::<Result<_>>()?,
        None => (0..width).collect(),
    };
    if let Some(filters) = query.get("where") {
        for filter in filters
            .as_array()
            .context("where must be an array of {column,equals} conditions")?
        {
            let filter = filter
                .as_object()
                .context("each where condition must be an object")?;
            allowed_fields(filter, &["column", "equals"])?;
            let col = column(
                filter
                    .get("column")
                    .context("where condition requires column")?,
            )?;
            let expected = filter
                .get("equals")
                .and_then(Value::as_str)
                .context("where condition requires a string equals value")?;
            rows.retain(|row| &data[*row][col] == expected);
        }
    }
    let cells: BTreeSet<_> = rows
        .iter()
        .flat_map(|row| columns.iter().map(move |col| (*row, *col)))
        .collect();
    let content = if cells.is_empty() {
        json!({"columns":[],"rows":[]})
    } else {
        let labels = if headers {
            json!(columns.iter().map(|col| &names[*col]).collect::<Vec<_>>())
        } else {
            Value::Null
        };
        let values: Vec<Vec<_>> = rows
            .iter()
            .map(|row| columns.iter().map(|col| &data[*row][*col]).collect())
            .collect();
        json!({"columns":labels,"rows":values})
    };
    Ok(json!({
        "domain":domain,
        "identity":TableIdentity {observation:observation.id.clone(), headers, cells},
        "content":content,
    }))
}

/// csv's parser intentionally accepts malformed quoting. Validate that grammar
/// first, then use csv to decode escaped quotes, delimiters and embedded newlines.
fn validate_delimited(bytes: &[u8], delimiter: u8) -> Result<()> {
    ensure!(
        !bytes.starts_with(b"\xef\xbb\xbf"),
        "UTF-8 BOM is unsupported; save the table without BOM"
    );
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Start,
        Bare,
        Quoted,
        Closed,
    }
    let mut state = State::Start;
    let mut record_started = false;
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if state == State::Quoted {
            if ch == b'"' {
                state = State::Closed;
            }
            i += 1;
            continue;
        }
        if ch == delimiter {
            state = State::Start;
            record_started = true;
        } else if ch == b'\n' || ch == b'\r' {
            ensure!(
                record_started,
                "blank record at byte {i}; quote a single empty field as \"\""
            );
            if ch == b'\r' {
                ensure!(
                    bytes.get(i + 1) == Some(&b'\n'),
                    "bare CR outside a quoted field at byte {i}; use LF or CRLF"
                );
                i += 1;
            }
            state = State::Start;
            record_started = false;
        } else {
            match (state, ch) {
                (State::Start, b'"') | (State::Closed, b'"') => state = State::Quoted,
                (State::Bare, b'"') => bail!("quote inside an unquoted field at byte {i}"),
                (State::Closed, _) => bail!("unexpected character after closing quote at byte {i}"),
                _ => state = State::Bare,
            }
            record_started = true;
        }
        i += 1;
    }
    ensure!(
        state != State::Quoted,
        "unterminated quoted field at end of input"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ResourceState;

    fn select(name: &str, source: &str, query: Value) -> Result<Region> {
        let observation = Observation::new(
            "data".into(),
            source.as_bytes(),
            "1".into(),
            "file".into(),
            ResourceState {
                exists: true,
                revision: Value::Null,
            },
        );
        let selector = RegionSelector {
            extension: name.into(),
            resource: Some("data".into()),
            query,
        };
        let mut result = resolve(name, &selector, &observation, source.as_bytes())?;
        assert!(result.get("observations").is_none());
        result["observations"] = json!([observation.id]);
        Ok(serde_json::from_value(result)?)
    }

    fn value(name: &str, source: &str, query: Value) -> Value {
        select(name, source, query).unwrap().content
    }

    fn rejects(name: &str, source: &str, query: Value) {
        assert!(
            select(name, source, query.clone()).is_err(),
            "{name}: {source:?} query {query}"
        );
    }

    #[test]
    fn manifests_and_utf8_contract() {
        for name in ["json", "jsonl", "yaml", "toml", "csv", "tsv"] {
            let manifest = manifest(name).unwrap();
            assert_eq!(manifest.name, name);
            assert_eq!(manifest.revision, "1");
            assert_eq!(manifest.operations, ["resolve", "compare"]);
            assert_eq!(manifest.domains, [domain(name).unwrap()]);
        }
        assert!(manifest("no-such-extension").is_none());
        let observation = Observation::new(
            "data".into(),
            &[255],
            "1".into(),
            "file".into(),
            ResourceState {
                exists: true,
                revision: Value::Null,
            },
        );
        let selector = RegionSelector {
            extension: "json".into(),
            resource: Some("data".into()),
            query: Value::Null,
        };
        assert!(
            resolve("json", &selector, &observation, &[255])
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
    }

    #[test]
    fn json_pointer_escaping_null_and_literal_object_keys() {
        let source = r#"{"":{"a/b":{"~key":[null,{"x":3}]}},"01":"literal","~1":4,"%2F":5}"#;
        assert_eq!(
            value("json", source, json!({"pointer":"//a~1b/~0key/0"})),
            json!({"values":[null]})
        );
        assert_eq!(
            value("json", source, json!({"pointer":"/01"})),
            json!({"values":["literal"]})
        );
        assert_eq!(
            value("json", source, json!({"pointer":"/~01"})),
            json!({"values":[4]})
        );
        assert_eq!(
            value("json", source, json!({"pointer":"/%2F"})),
            json!({"values":[5]})
        );
        assert_eq!(
            select("json", source, json!({})).unwrap().identity,
            select("json", source, json!({"pointer":""}))
                .unwrap()
                .identity
        );
        for pointer in [
            "/missing",
            "//a~1b/~0key/01",
            "//a~1b/~0key/-",
            "//a~1b/~0key/+1",
            "//a~1b/~0key/-1",
            "//a~1b/~0key/2",
            "//a~1b/~0key/0/x",
            "//a~1b/~0key/99999999999999999999999",
            "/~2",
            "/~",
            "#/%2F",
            "x",
        ] {
            rejects("json", source, json!({"pointer":pointer}));
        }
        for query in [
            json!("/01"),
            json!([]),
            json!({"pointer":null}),
            json!({"pointer":1}),
            json!({"typo":true}),
            json!({"lines":[]}),
        ] {
            rejects("json", source, query);
        }
    }

    #[test]
    fn json_numbers_and_keys_are_preserved_without_duplicate_overwrite() {
        let source = r#"{"huge":1234567890123456789012345678901234567890,"precise":0.12345678901234567890123456789,"exponent":1e9999}"#;
        let content = value("json", source, Value::Null);
        assert_eq!(
            content["values"][0],
            serde_json::from_str::<Value>(source).unwrap()
        );
        for source in [
            r#"{"x":1,"x":2}"#,
            r#"{"a":{"x":1,"\u0078":2}}"#,
            r#"[{"x":0,"x":1}]"#,
            r#"{"a":1} trailing"#,
            "",
            "[1,]",
            "NaN",
        ] {
            rejects("json", source, Value::Null);
        }
        let deep = format!("{}0{}", "[".repeat(130), "]".repeat(130));
        rejects("json", &deep, Value::Null);
        for key in [
            "$serde_json::private::Number",
            "$serde_json::private::RawValue",
        ] {
            let source = format!("{{{key:?}:\"ordinary\"}}");
            rejects("json", &source, Value::Null);
            assert_eq!(
                value("json", &source, json!({"pointer":format!("/{key}")})),
                json!({"values":["ordinary"]})
            );
        }
    }

    #[test]
    fn subtree_comparisons_obey_prefix_and_observation_semantics() {
        let source = r#"{"a":{"x":1},"ab":2}"#;
        let regions: Vec<_> = ["", "/a", "/a/x", "/ab"]
            .into_iter()
            .map(|p| select("json", source, json!({"pointer":p})).unwrap())
            .collect();
        assert_eq!(
            compare("json", &regions[0], &regions[2]).unwrap(),
            Relation::Contains
        );
        assert_eq!(
            compare("json", &regions[2], &regions[1]).unwrap(),
            Relation::Within
        );
        assert_eq!(
            compare("json", &regions[1], &regions[3]).unwrap(),
            Relation::Incomparable
        );
        let other = select("json", r#"{"a":{"x":1},"ab":3}"#, json!({"pointer":"/a"})).unwrap();
        assert_eq!(other.content, regions[1].content);
        assert_eq!(
            compare("json", &regions[1], &other).unwrap(),
            Relation::Incomparable
        );
        let mut malformed = regions[0].clone();
        malformed.identity["paths"] = json!([["a"], ["a", "x"]]);
        assert!(compare("json", &malformed, &regions[0]).is_err());
        malformed.domain = "wrong".into();
        assert!(compare("json", &malformed, &regions[0]).is_err());
        assert_laws("json", &regions);
    }

    #[test]
    fn jsonl_physical_lines_and_pointer_share_record_regions() {
        let source = "{\"a\":1}\r\nnull\r\n[3]\r\n";
        assert_eq!(
            value("jsonl", source, json!({"lines":[3,1,3]})),
            json!({"values":[{"a":1},[3]]})
        );
        let line = select("jsonl", source, json!({"lines":[1]})).unwrap();
        let pointer = select("jsonl", source, json!({"pointer":"/0"})).unwrap();
        assert_eq!(line.identity, pointer.identity);
        assert_eq!(line.content, pointer.content);
        let leaf = select("jsonl", source, json!({"pointer":"/0/a"})).unwrap();
        assert_eq!(compare("jsonl", &line, &leaf).unwrap(), Relation::Contains);
        let empty = select("jsonl", source, json!({"lines":[]})).unwrap();
        assert_eq!(empty.content, json!({"values":[]}));
        assert_eq!(compare("jsonl", &empty, &line).unwrap(), Relation::Within);
        assert_eq!(value("jsonl", "", Value::Null), json!({"values":[[]]}));
        assert_eq!(
            value("jsonl", "", json!({"lines":[]})),
            json!({"values":[]})
        );
        for source in [
            "null\n\n",
            "\n",
            "{}\n \n",
            "{}\nbroken",
            "{\n}\n",
            "{}\n{\"a\":1,\"a\":2}",
        ] {
            rejects("jsonl", source, json!({"lines":[]}));
        }
        for query in [
            json!({"lines":[0]}),
            json!({"lines":[4]}),
            json!({"lines":[-1]}),
            json!({"lines":[true]}),
            json!({"lines":[1.0]}),
            json!({"lines":null}),
            json!({"lines":[],"pointer":""}),
        ] {
            rejects("jsonl", source, query);
        }
        let many = (0..12)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            value("jsonl", &many, json!({"lines":[11,3]})),
            json!({"values":[2,10]})
        );
        let regions = [
            json!({}),
            json!({"lines":[]}),
            json!({"lines":[1]}),
            json!({"lines":[1,2]}),
            json!({"lines":[2,3]}),
            json!({"pointer":"/0/a"}),
        ]
        .into_iter()
        .map(|q| select("jsonl", source, q).unwrap())
        .collect::<Vec<_>>();
        assert_eq!(
            compare("jsonl", &regions[3], &regions[4]).unwrap(),
            Relation::Incomparable
        );
        assert_laws("jsonl", &regions);
    }

    #[test]
    fn yaml_preserves_structure_aliases_and_precise_numeric_scalars() {
        let source = "base: &base\n  x: null\ncopy: *base\nlist: [true, 'yes', 0xFF, 18446744073709551616, .125, +1., 1.234567890123456789012345]\n";
        assert_eq!(
            value("yaml", source, json!({"pointer":"/copy/x"})),
            json!({"values":[null]})
        );
        let values = value("yaml", source, json!({"pointer":"/list"}));
        assert_eq!(values,serde_json::from_str::<Value>(r#"{"values":[[true,"yes",255,18446744073709551616,0.125,1.0,1.234567890123456789012345]]}"#).unwrap());
        assert_eq!(value("yaml", "", Value::Null), json!({"values":[null]}));
        assert_eq!(
            value("yaml", "k: |\n  !literal\n", json!({"pointer":"/k"})),
            json!({"values":["!literal\n"]})
        );
        assert_eq!(
            value("yaml", "k: &s '!literal'\nv: *s\n", json!({"pointer":"/v"})),
            json!({"values":["!literal"]})
        );
        assert_eq!(
            value("yaml", "k: 'hello !literal'\n", Value::Null),
            json!({"values":[{"k":"hello !literal"}]})
        );
        assert_eq!(
            value("yaml", "日本語: 雪だるま☃\n", json!({"pointer":"/日本語"})),
            json!({"values":["雪だるま☃"]})
        );
        let source = "a: [1, 2]\nb: {}\n";
        let regions = ["", "/a", "/a/0", "/b"]
            .into_iter()
            .map(|pointer| select("yaml", source, json!({"pointer":pointer})).unwrap())
            .collect::<Vec<_>>();
        assert_laws("yaml", &regions);
    }

    #[test]
    fn yaml_rejects_ambiguous_mapping_tags_nonfinite_and_multiple_documents() {
        for source in [
            "1: value",
            "true: value",
            "? [a,b]\n: value",
            "a: 1\na: 2",
            "a: 1\n'a': 2",
            "a: .nan",
            "a: -.inf",
            "a: !custom value",
            "a: !<tag:example.com,2020:thing> value",
            "a: !!binary SGVsbG8=",
            "a: !!str text",
            "a: {<<: {x: 1}}",
            "a: 1\n---\nb: 2",
            "a: [1",
            "a: it's 'plain\nb: !<tag:example.org:thing> value",
        ] {
            rejects("yaml", source, Value::Null);
        }
        rejects("yaml", "a: null", json!({"pointer":"/missing"}));
    }

    #[test]
    fn yaml_tag_validation_distinguishes_quoted_flow_block_and_plain_scalars() {
        for source in [
            "a: {\"b\":\"hello !world\"}",
            "a: [\"hello !world\", {'b':'!value'}]",
            "a: &label 'hello !world'\nb: *label",
            "a: |\n  !literal\n  second !line\nb: done",
            "a: >-\n  !literal\n  second !line\nb: done",
            "--- 'hello !world'",
            "\u{feff}a: \"hello !world\"",
            "a: 'hello\n  !world'",
            "a: \"hello \\\" !world\"",
            "a: 'it''s !fine'",
            "a: don't\nb: 日本語",
        ] {
            select("yaml", source, Value::Null)
                .unwrap_or_else(|error| panic!("{source:?}: {error:#}"));
        }
        for source in [
            "a: plain\n  'quote\nb: !<tag:example.org:thing> value",
            "a:\n  text: plain\n    'quote\n  b: !<tag:example.org:thing> value",
            "a: {\"b\": !<tag:example.org:thing> value}",
            "a: [1,!<tag:example.org:thing> value]",
        ] {
            rejects("yaml", source, Value::Null);
        }
    }

    #[test]
    fn yaml_numeric_limits_never_coerce_plain_numbers_to_strings_or_round_them() {
        let decimal = "123456789012345678901234567890123456789012345678901234567890";
        for raw in [
            decimal,
            "1e9999",
            "-1e9999",
            "1e-9999",
            "0.123456789012345678901234567890123456789",
        ] {
            let expected: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(
                value("yaml", &format!("x: {raw}"), json!({"pointer":"/x"})),
                json!({"values":[expected]})
            );
            for quoted in [
                format!("'{raw}'"),
                format!("\"{raw}\""),
                format!("|\n  {raw}\n"),
            ] {
                assert!(
                    value("yaml", &format!("x: {quoted}"), json!({"pointer":"/x"}))["values"][0]
                        .is_string()
                );
            }
        }
        assert_eq!(
            value(
                "yaml",
                "x: 0x100000000000000000000000000000000",
                json!({"pointer":"/x"})
            ),
            serde_json::from_str::<Value>(
                r#"{"values":[340282366920938463463374607431768211456]}"#
            )
            .unwrap()
        );
        assert_eq!(
            value("yaml", "x: &n 1e9999\ny: *n", json!({"pointer":"/y"}))["values"][0],
            serde_json::from_str::<Value>("1e9999").unwrap()
        );
        assert_eq!(
            value("yaml", "x: 0123", json!({"pointer":"/x"})),
            json!({"values":["0123"]})
        );
        rejects("yaml", "1e9999: value", Value::Null);
    }

    #[test]
    fn toml_datetime_and_all_other_values_have_unambiguous_types() {
        let source = "stamp = 1979-05-27T07:32:00Z\nlocal = 1979-05-27T07:32:00\ndate = 1979-05-27\ntime = 07:32:00\nn = 9223372036854775807\nf = 1.0\ns = '1979-05-27T07:32:00Z'\nflag = true\nxs = [1,2]\n[record]\ntype = 'datetime'\nvalue = '1979-05-27T07:32:00Z'\n";
        assert_eq!(
            value("toml", source, json!({"pointer":"/stamp"})),
            json!({"values":[{"type":"datetime","value":"1979-05-27T07:32:00Z"}]})
        );
        assert_ne!(
            value("toml", source, json!({"pointer":"/stamp"})),
            value("toml", source, json!({"pointer":"/s"}))
        );
        assert_ne!(
            value("toml", source, json!({"pointer":"/stamp"})),
            value("toml", source, json!({"pointer":"/record"}))
        );
        assert_eq!(
            value("toml", source, json!({"pointer":"/n"}))["values"][0],
            json!({"type":"integer","value":9223372036854775807i64})
        );
        assert_eq!(
            value("toml", source, json!({"pointer":"/f"}))["values"][0],
            json!({"type":"float","value":1.0})
        );
        assert_eq!(
            value("toml", source, json!({"pointer":"/xs/1"}))["values"][0],
            json!({"type":"integer","value":2})
        );
        assert_eq!(
            value("toml", "", Value::Null),
            json!({"values":[{"type":"table","value":{}}]})
        );
        for source in [
            "x=nan",
            "x=inf",
            "x=-inf",
            "x=1\nx=2",
            "x=9223372036854775808",
            "x=[",
        ] {
            rejects("toml", source, Value::Null);
        }
        rejects("toml", source, json!({"pointer":"/xs/01"}));
        let regions = ["", "/record", "/record/type", "/xs", "/xs/0"]
            .into_iter()
            .map(|pointer| select("toml", source, json!({"pointer":pointer})).unwrap())
            .collect::<Vec<_>>();
        assert_laws("toml", &regions);
    }

    #[test]
    fn tables_parse_rfc_quoting_embedded_newlines_and_empty_strings() {
        let csv =
            "name,note,empty\r\na,\"comma, and \"\"quote\"\"\",\r\nb,\"line 1\r\nline 2\",\"\"\r\n";
        assert_eq!(
            value("csv", csv, Value::Null),
            json!({"columns":["name","note","empty"],"rows":[["a","comma, and \"quote\"",""],["b","line 1\r\nline 2",""]]})
        );
        let tsv = "name\tnote\na\t\"tab\there\nnewline\"\n";
        assert_eq!(
            value("tsv", tsv, Value::Null),
            json!({"columns":["name","note"],"rows":[["a","tab\there\nnewline"]]})
        );
        assert_eq!(
            value("csv", "\"\"\n", json!({"headers":false})),
            json!({"columns":null,"rows":[[""]]})
        );
        assert_eq!(
            value("csv", "x,y\n,\n", Value::Null),
            json!({"columns":["x","y"],"rows":[["",""]]})
        );
        for (name, source) in [("csv", ""), ("tsv", ""), ("csv", "x,y\n")] {
            assert_eq!(
                value(name, source, Value::Null),
                json!({"columns":[],"rows":[]})
            );
        }
    }

    #[test]
    fn table_queries_select_canonical_cells_and_filter_exact_strings() {
        let source = "name,status,score\na,ok,01\nb,no,2\nc,ok,3\n";
        let by_name = select(
            "csv",
            source,
            json!({"rows":[2,0,2],"columns":["score","name",0]}),
        )
        .unwrap();
        let by_index = select("csv", source, json!({"rows":[0,2],"columns":[0,2]})).unwrap();
        let by_filter = select(
            "csv",
            source,
            json!({"columns":[0,2],"where":[{"column":"status","equals":"ok"}]}),
        )
        .unwrap();
        assert_eq!(by_name.identity, by_index.identity);
        assert_eq!(by_name.identity, by_filter.identity);
        assert_eq!(
            by_name.content,
            json!({"columns":["name","score"],"rows":[["a","01"],["c","3"]]})
        );
        let filtered = value(
            "csv",
            source,
            json!({"where":[{"column":"status","equals":"ok"},{"column":2,"equals":"3"}]}),
        );
        assert_eq!(filtered["rows"], json!([["c", "ok", "3"]]));
        assert_eq!(
            value(
                "csv",
                source,
                json!({"headers":false,"rows":[0],"columns":[0]})
            ),
            json!({"columns":null,"rows":[["name"]]})
        );
        let queries = [
            json!({}),
            json!({"rows":[0]}),
            json!({"rows":[0,1]}),
            json!({"columns":[0]}),
            json!({"rows":[0],"columns":[0]}),
            json!({"rows":[]}),
            json!({"columns":[]}),
            json!({"where":[{"column":0,"equals":"absent"}]}),
        ];
        let regions: Vec<_> = queries
            .into_iter()
            .map(|q| select("csv", source, q).unwrap())
            .collect();
        assert_eq!(regions[5].identity, regions[6].identity);
        assert_eq!(regions[5].identity, regions[7].identity);
        assert_eq!(
            compare("csv", &regions[1], &regions[3]).unwrap(),
            Relation::Incomparable
        );
        assert_eq!(
            compare("csv", &regions[1], &regions[4]).unwrap(),
            Relation::Contains
        );
        assert_laws("csv", &regions);
        let raw = select("csv", source, json!({"headers":false})).unwrap();
        assert_eq!(
            compare("csv", &raw, &regions[0]).unwrap(),
            Relation::Incomparable
        );
    }

    #[test]
    fn table_content_is_unchanged_when_unselected_rows_move_or_change() {
        let query = json!({"columns":["name"],"where":[{"column":"status","equals":"ok"}]});
        let a = select("csv", "name,status\na,ok\nb,no\nc,ok\n", query.clone()).unwrap();
        let b = select("csv", "name,status\nnew,no\na,ok\nb,changed\nc,ok\n", query).unwrap();
        assert_eq!(a.content, b.content);
        assert_eq!(compare("csv", &a, &b).unwrap(), Relation::Incomparable);
        let positional = json!({"rows":[0],"columns":[0]});
        assert_ne!(
            value("csv", "x\na\nb\n", positional.clone()),
            value("csv", "x\nb\na\n", positional)
        );
    }

    #[test]
    fn tables_reject_malformed_sources_and_queries_even_for_empty_selections() {
        for source in [
            "a,a\nx,y\n",
            ",b\nx,y\n",
            "a,b\nx\n",
            "a\nx,y\n",
            "a\n\"unterminated",
            "a\nno\"quote\n",
            "a\n\"ok\"suffix\n",
            "a\n\n",
            "a\rvalue",
            "\u{feff}a\nx\n",
        ] {
            rejects("csv", source, json!({"rows":[]}));
        }
        let source = "a,b\nx,y\n";
        for query in [
            json!({"headers":null}),
            json!({"headers":"true"}),
            json!({"rows":null}),
            json!({"rows":[-1]}),
            json!({"rows":[true]}),
            json!({"rows":[0.0]}),
            json!({"rows":[1]}),
            json!({"columns":null}),
            json!({"columns":[-1]}),
            json!({"columns":[false]}),
            json!({"columns":[2]}),
            json!({"columns":["missing"]}),
            json!({"headers":false,"columns":["a"]}),
            json!({"where":{}}),
            json!({"where":[null]}),
            json!({"where":[{}]}),
            json!({"where":[{"column":0}]}),
            json!({"where":[{"column":0,"equals":1}]}),
            json!({"where":[{"column":0,"equals":"x","extra":1}]}),
            json!({"rows":[],"columns":["missing"]}),
            json!({"rows":[],"where":[{"column":"missing","equals":"x"}]}),
            json!({"unknown":0}),
            json!([]),
        ] {
            rejects("csv", source, query);
        }
    }

    fn assert_laws(name: &str, regions: &[Region]) {
        let is_subset = |relation| matches!(relation, Relation::Equal | Relation::Within);
        for a in regions {
            assert_eq!(compare(name, a, a).unwrap(), Relation::Equal);
            for b in regions {
                let ab = compare(name, a, b).unwrap();
                let ba = compare(name, b, a).unwrap();
                let inverse = match ab {
                    Relation::Equal => Relation::Equal,
                    Relation::Within => Relation::Contains,
                    Relation::Contains => Relation::Within,
                    Relation::Incomparable => Relation::Incomparable,
                };
                assert_eq!(ba, inverse);
                assert_eq!(ab == Relation::Equal, is_subset(ab) && is_subset(ba));
                for c in regions {
                    let bc = compare(name, b, c).unwrap();
                    if is_subset(ab) && is_subset(bc) {
                        assert!(is_subset(compare(name, a, c).unwrap()));
                    }
                }
            }
        }
    }
}
