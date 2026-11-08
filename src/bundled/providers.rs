use crate::extensions::process::Manifest;
use crate::model::{Observation, Region, RegionSelector, Relation, digest};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Component, Path};
use std::process::Command;
use url::Url;

pub fn manifest(name: &str) -> Option<Manifest> {
    let (operations, domains, description) = match name {
        "file" => (
            vec!["observe", "resolve", "compare"],
            vec!["monika.file.v1"],
            "Observe local regular files. query:null selects the whole file, including binary files. Content is {sha256,size}; use text, markdown, code or a data extension to capture interpreted content. Not-found is an absent Observation; other I/O failures are errors.",
        ),
        "http" => (
            vec!["observe"],
            vec![],
            "Observe http/https with GET and validated TLS. Follows up to 10 redirects; 404/410 mean absent, other non-2xx statuses are errors. Content is the response body. Use text/html/json/etc to select Regions. No credentials or custom headers are configured by this provider.",
        ),
        "git" => (
            vec!["observe"],
            vec![],
            "Observe a tracked blob from a local Git revision: git:src/main.rs?rev=HEAD (rev defaults to HEAD). Resolves the commit first; never changes the index/worktree. Requires git on PATH. Use text/code/etc to select Regions.",
        ),
        _ => return None,
    };
    Some(Manifest {
        name: name.into(),
        revision: "1".into(),
        operations: operations.into_iter().map(String::from).collect(),
        domains: domains.into_iter().map(String::from).collect(),
        description: description.into(),
    })
}

pub fn observe(name: &str, resource: &str, workspace: &Path, destination: &Path) -> Result<Value> {
    let url = Url::parse(resource)?;
    let (bytes, state) = match name {
        "file" => file(&url)?,
        "http" => http(&url)?,
        "git" => git(&url, workspace)?,
        _ => bail!("{name} is not an observation provider"),
    };
    std::fs::write(destination, bytes).context("write Observation content")?;
    Ok(json!({"state":state}))
}

fn file(url: &Url) -> Result<(Vec<u8>, Value)> {
    ensure!(
        url.scheme() == "file" && url.query().is_none() && url.fragment().is_none(),
        "expected a local file URI"
    );
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("invalid file URI"))?;
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((vec![], json!({"exists":false,"revision":null})));
        }
        Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
    };
    ensure!(
        metadata.is_file(),
        "Resource is not a regular file: {}",
        path.display()
    );
    let mut input =
        std::fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    let state = json!({"exists":true,"revision":{"sha256":digest(&bytes)}});
    Ok((bytes, state))
}

fn http(url: &Url) -> Result<(Vec<u8>, Value)> {
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "expected http or https URI"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "HTTP credentials in a Resource URI are not supported"
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("monika/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(10))
        .retry(reqwest::retry::never())
        .build()?;
    let response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .context("HTTP observation failed")?;
    let status = response.status();
    let final_url = response.url().as_str().to_owned();
    if matches!(status.as_u16(), 404 | 410) {
        return Ok((
            vec![],
            json!({"exists":false,"revision":{"url":final_url,"status":status.as_u16()}}),
        ));
    }
    ensure!(
        status.is_success(),
        "HTTP observation returned {status}: {final_url}"
    );
    if let Some(encoding) = response.headers().get(reqwest::header::CONTENT_ENCODING) {
        ensure!(
            encoding.to_str()?.eq_ignore_ascii_case("identity"),
            "server ignored Accept-Encoding: identity; unsupported content encoding"
        );
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()?;
    let modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()?;
    let bytes = response
        .bytes()
        .context("read HTTP response body")?
        .to_vec();
    let state = json!({"exists":true,"revision":{"url":final_url,"etag":etag,"last_modified":modified,"sha256":digest(&bytes)}});
    Ok((bytes, state))
}

fn git(url: &Url, workspace: &Path) -> Result<(Vec<u8>, Value)> {
    ensure!(
        url.scheme() == "git" && url.host_str().is_none() && url.fragment().is_none(),
        "use git:relative/path?rev=REV"
    );
    let mut revision = None;
    for (name, value) in url.query_pairs() {
        ensure!(
            name == "rev" && revision.is_none() && !value.is_empty(),
            "Git URI accepts one nonempty rev parameter"
        );
        revision = Some(value.into_owned());
    }
    let revision = revision.unwrap_or_else(|| "HEAD".into());
    // Decode percent escapes as a path, not form data (a '+' remains a '+').
    let path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .context("Git path must be UTF-8")?;
    let path = path.as_ref();
    ensure!(
        !path.is_empty()
            && !path.contains('\0')
            && Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "Git path must be relative to the project root"
    );
    let run = |args: &[&str]| -> Result<std::process::Output> {
        Command::new("git")
            .args([
                "--literal-pathspecs",
                "-C",
                workspace.to_str().context("workspace must be UTF-8")?,
            ])
            .args(args)
            .output()
            .context("run git (install git to use git: Resources)")
    };
    let commit = run(&[
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("{revision}^{{commit}}"),
    ])?;
    ensure!(
        commit.status.success(),
        "resolve Git revision: {}",
        String::from_utf8_lossy(&commit.stderr).trim()
    );
    let commit = String::from_utf8(commit.stdout)?.trim().to_owned();
    // ls-tree distinguishes an absent path from a corrupt repository or bad revision.
    let listing = run(&["ls-tree", "-z", &commit, "--", path])?;
    ensure!(
        listing.status.success(),
        "list Git path: {}",
        String::from_utf8_lossy(&listing.stderr).trim()
    );
    if listing.stdout.is_empty() {
        return Ok((
            vec![],
            json!({"exists":false,"revision":{"commit":commit,"path":path}}),
        ));
    }
    let mut records = listing
        .stdout
        .split(|&byte| byte == 0)
        .filter(|record| !record.is_empty());
    let record = records.next().context("missing Git tree record")?;
    let separator = record
        .iter()
        .position(|&byte| byte == b'\t')
        .context("invalid Git tree record")?;
    ensure!(
        records.next().is_none() && &record[separator + 1..] == path.as_bytes(),
        "Git Resource must name one regular file, not a directory"
    );
    let header = std::str::from_utf8(&record[..separator])?;
    let fields: Vec<_> = header.split_whitespace().collect();
    ensure!(
        fields.len() == 3 && fields[1] == "blob" && matches!(fields[0], "100644" | "100755"),
        "Git Resource must be a regular file blob"
    );
    let blob = run(&["cat-file", "blob", fields[2]])?;
    ensure!(
        blob.status.success(),
        "read Git blob: {}",
        String::from_utf8_lossy(&blob.stderr).trim()
    );
    Ok((
        blob.stdout,
        json!({"exists":true,"revision":{"commit":commit,"path":path,"blob":fields[2]}}),
    ))
}

pub fn resolve(
    name: &str,
    selector: &RegionSelector,
    observation: &Observation,
    bytes: &[u8],
) -> Result<Value> {
    ensure!(
        name == "file" && selector.query.is_null(),
        "file accepts query:null; use text or a format extension to select a part"
    );
    Ok(
        json!({"domain":"monika.file.v1","identity":observation.id,"content":{"sha256":digest(bytes),"size":bytes.len()}}),
    )
}

pub fn compare(name: &str, left: &Region, right: &Region) -> Result<Relation> {
    ensure!(
        name == "file" && left.domain == "monika.file.v1" && right.domain == left.domain,
        "expected file Regions"
    );
    Ok(if left.identity == right.identity {
        Relation::Equal
    } else {
        Relation::Incomparable
    })
}
