//! Bundled implementations of the public extension protocol.

mod annotations;
pub mod code;
pub mod data;
pub mod documents;
mod providers;
mod server;

use crate::extensions::process::Manifest;
use crate::model::{Observation, Region, RegionSelector, Relation};
use crate::workspace::{Config, ProcessExtension, Workspace};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use server::serve;

pub const NAMES: &[&str] = &[
    "file", "http", "git", "text", "markdown", "html", "json", "jsonl", "yaml", "toml", "csv",
    "tsv", "code", "anchor", "assert",
];

pub fn manifest(name: &str) -> Result<Manifest> {
    providers::manifest(name)
        .or_else(|| annotations::manifest(name))
        .or_else(|| documents::manifest(name))
        .or_else(|| data::manifest(name))
        .or_else(|| code::manifest(name))
        .with_context(|| format!("unknown bundled extension: {name}"))
}

pub fn registration(name: &str) -> Result<ProcessExtension> {
    manifest(name)?;
    let schemes = match name {
        "file" => vec!["file".into()],
        "http" => vec!["http".into(), "https".into()],
        "git" => vec!["git".into()],
        _ => vec![],
    };
    Ok(ProcessExtension {
        command: vec![
            "@monika".into(),
            "extension".into(),
            "serve".into(),
            name.into(),
        ],
        revision: "1".into(),
        schemes,
        timeout_ms: 30_000,
    })
}

pub fn add_to_config(config: &mut Config, names: &[String]) -> Result<()> {
    for name in names {
        let registration = registration(name)?;
        if let Some(existing) = config.extensions.get(name) {
            ensure!(
                serde_json::to_value(existing)? == serde_json::to_value(&registration)?,
                "extension {name} is already configured differently; edit monika.json explicitly"
            );
            continue;
        }
        for scheme in &registration.schemes {
            for (owner, existing) in &config.extensions {
                ensure!(
                    !existing.schemes.contains(scheme),
                    "scheme {scheme} is already provided by {owner}; edit monika.json explicitly"
                );
            }
        }
        config.extensions.insert(name.clone(), registration);
    }
    if !config.comparators.contains_key("monika.bytes.v1") {
        for name in ["text", "markdown", "code"] {
            let standard = registration(name)?;
            if let Some(existing) = config.extensions.get(name)
                && existing.command == standard.command
                && existing.revision == standard.revision
            {
                config
                    .comparators
                    .insert("monika.bytes.v1".into(), name.into());
                break;
            }
        }
    }
    config.validate()
}

pub fn default_config() -> Config {
    let mut config = Config::default();
    add_to_config(
        &mut config,
        &NAMES.iter().map(|name| (*name).into()).collect::<Vec<_>>(),
    )
    .expect("valid bundled registrations");
    config
}

pub async fn enable(workspace: &Workspace, names: &[String]) -> Result<()> {
    let path = workspace.root.join("monika.json");
    ensure!(path.is_file(), "run monika init before enabling extensions");
    let mut config = workspace.config.clone();
    add_to_config(&mut config, names)?;
    let mut bytes = serde_json::to_vec_pretty(&config)?;
    bytes.push(b'\n');
    workspace.replace(&path, &bytes).await
}

fn resolve(
    name: &str,
    selector: &RegionSelector,
    observation: &Observation,
    bytes: &[u8],
) -> Result<Value> {
    if providers::manifest(name).is_some() {
        providers::resolve(name, selector, observation, bytes)
    } else if documents::manifest(name).is_some() {
        documents::resolve(name, selector, observation, bytes)
    } else if data::manifest(name).is_some() {
        data::resolve(name, selector, observation, bytes)
    } else {
        code::resolve(name, selector, observation, bytes)
    }
}

fn compare(name: &str, left: &Region, right: &Region) -> Result<Relation> {
    ensure!(
        left.domain == right.domain,
        "Regions have different domains"
    );
    if name == "anchor" {
        annotations::compare(left, right)
    } else if providers::manifest(name).is_some() {
        providers::compare(name, left, right)
    } else if documents::manifest(name).is_some() {
        documents::compare(name, left, right)
    } else if data::manifest(name).is_some() {
        data::compare(name, left, right)
    } else {
        code::compare(name, left, right)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpanIdentity {
    observation: String,
    ranges: Vec<(usize, usize)>,
}

pub fn span_region(
    observation: &Observation,
    bytes: &[u8],
    mut ranges: Vec<(usize, usize)>,
) -> Result<Value> {
    let text = std::str::from_utf8(bytes).context("Resource is not UTF-8 text")?;
    for &(start, end) in &ranges {
        ensure!(
            start <= end && end <= bytes.len(),
            "invalid byte range {start}..{end}"
        );
        ensure!(
            text.is_char_boundary(start) && text.is_char_boundary(end),
            "range splits a UTF-8 character"
        );
    }
    ranges.retain(|(start, end)| start != end);
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(previous) = merged.last_mut()
            && start <= previous.1
        {
            previous.1 = previous.1.max(end);
            continue;
        }
        merged.push((start, end));
    }
    let selected: String = merged
        .iter()
        .map(|&(start, end)| &text[start..end])
        .collect();
    Ok(json!({
        "domain": "monika.bytes.v1",
        "identity": SpanIdentity { observation: observation.id.clone(), ranges: merged },
        "content": {"text": selected}
    }))
}

pub fn compare_spans(left: &Region, right: &Region) -> Result<Relation> {
    ensure!(
        left.domain == "monika.bytes.v1" && right.domain == left.domain,
        "expected monika.bytes.v1 Regions"
    );
    let left: SpanIdentity = serde_json::from_value(left.identity.clone())?;
    let right: SpanIdentity = serde_json::from_value(right.identity.clone())?;
    if left.observation != right.observation {
        return Ok(Relation::Incomparable);
    }
    let subset = |a: &[(usize, usize)], b: &[(usize, usize)]| {
        a.iter()
            .all(|&(start, end)| b.iter().any(|&(bs, be)| bs <= start && end <= be))
    };
    Ok(
        match (
            subset(&left.ranges, &right.ranges),
            subset(&right.ranges, &left.ranges),
        ) {
            (true, true) => Relation::Equal,
            (true, false) => Relation::Within,
            (false, true) => Relation::Contains,
            (false, false) => Relation::Incomparable,
        },
    )
}
