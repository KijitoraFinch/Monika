use super::{annotations, compare, manifest, providers, resolve};
use crate::model::{Observation, Region, RegionSelector};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u32,
    id: u64,
    workspace: PathBuf,
    operation: String,
    input: Value,
}

fn send(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn receive(input: &mut impl BufRead) -> Result<Option<Value>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&line).context("invalid extension protocol JSON")?;
    ensure!(value["protocol"] == 1, "unsupported extension protocol");
    Ok(Some(value))
}

fn invoke(
    name: &str,
    request: &Request,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Value> {
    ensure!(request.protocol == 1, "unsupported extension protocol");
    let info = manifest(name)?;
    if request.operation == "describe" {
        return Ok(serde_json::to_value(info)?);
    }
    ensure!(
        info.supports(&request.operation),
        "unsupported operation: {}",
        request.operation
    );
    match request.operation.as_str() {
        "observe" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                resource: String,
                content_path: PathBuf,
            }
            let args: Input = serde_json::from_value(request.input.clone())?;
            providers::observe(name, &args.resource, &request.workspace, &args.content_path)
        }
        "resolve" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                selector: RegionSelector,
            }
            let args: Input = serde_json::from_value(request.input.clone())?;
            if name == "anchor" {
                return annotations::anchor(&args.selector);
            }
            let resource = args
                .selector
                .resource
                .as_ref()
                .context("this extension requires a Resource")?;
            send(
                output,
                &json!({"protocol":1,"id":request.id,"read":[resource]}),
            )?;
            let response = loop {
                let value = receive(input)?.context("Core closed stdin while resolving")?;
                if value.get("operation").is_some() {
                    let nested: Request = serde_json::from_value(value)?;
                    ensure!(
                        nested.operation == "observe" && nested.id != request.id,
                        "expected an observation request"
                    );
                    respond(name, nested, input, output)?;
                } else {
                    break value;
                }
            };
            ensure!(
                response["id"] == request.id,
                "observation response ID mismatch"
            );
            if let Some(error) = response.get("read_error") {
                bail!("Observation failed: {error}");
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Read {
                observation: Observation,
                content_path: PathBuf,
            }
            let observations: Vec<Read> = serde_json::from_value(
                response
                    .get("read_result")
                    .context("missing read_result")?
                    .clone(),
            )?;
            ensure!(observations.len() == 1, "expected one Observation");
            let observed = &observations[0];
            ensure!(
                observed.observation.state.exists,
                "Resource does not exist: {resource}"
            );
            let bytes = std::fs::read(&observed.content_path)?;
            resolve(name, &args.selector, &observed.observation, &bytes)
        }
        "compare" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                left: Region,
                right: Region,
            }
            let args: Input = serde_json::from_value(request.input.clone())?;
            Ok(serde_json::to_value(compare(
                name,
                &args.left,
                &args.right,
            )?)?)
        }
        "check" => annotations::check(&request.input),
        _ => bail!("unsupported bundled operation"),
    }
}

fn respond(
    name: &str,
    request: Request,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    let result = match invoke(name, &request, input, output) {
        Ok(value) => json!({"protocol":1,"id":request.id,"result":value}),
        Err(error) => json!({"protocol":1,"id":request.id,"error":format!("{error:#}")}),
    };
    send(output, &result)
}

/// Blocking protocol worker; called before starting the CLI's async runtime.
pub fn serve(name: &str) -> Result<()> {
    manifest(name)?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(value) = receive(&mut input)? {
        respond(
            name,
            serde_json::from_value(value)?,
            &mut input,
            &mut output,
        )?;
    }
    Ok(())
}
