use crate::extensions::process::Manifest;
use crate::model::{Region, RegionSelector, Relation};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn manifest(name: &str) -> Option<Manifest> {
    let (operations, domains, description) = match name {
        "anchor" => (
            vec!["resolve", "compare"],
            vec!["monika.anchor.v1"],
            "A named, unobserved anchor for references and keeping quoted Captures: no resource, query is a nonempty string. Content is {name:query}. The label is not a claim about any external Resource.",
        ),
        "assert" => (
            vec!["check"],
            vec![],
            "Opt-in annotation check. Configure checks:[{extension:assert}]. annotation.expect is an array of {endpoint:source|target,pointer:JSON-Pointer,equals:JSON}. Compares exact Region content values; missing paths fail, malformed rules error. options may set {field:string}.",
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

pub fn anchor(selector: &RegionSelector) -> Result<Value> {
    ensure!(
        selector.resource.is_none(),
        "anchor uses a label, not a Resource"
    );
    let name = selector
        .query
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .context("anchor query must be a nonempty string")?;
    Ok(json!({"domain":"monika.anchor.v1","identity":name,"content":{"name":name}}))
}

pub fn compare(left: &Region, right: &Region) -> Result<Relation> {
    ensure!(
        left.domain == "monika.anchor.v1" && right.domain == left.domain,
        "expected anchor Regions"
    );
    Ok(if left.identity == right.identity {
        Relation::Equal
    } else {
        Relation::Incomparable
    })
}

pub fn check(input: &Value) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Options {
        field: String,
    }
    let field = if input["options"].is_null() {
        "expect".into()
    } else {
        serde_json::from_value::<Options>(input["options"].clone())?.field
    };
    ensure!(!field.is_empty(), "assert field must not be empty");
    let Some(rules) = input["link"]["annotation"].get(&field) else {
        return Ok(
            json!([{"code":"expect","status":"not_applicable","message":format!("No {field} expectations")}]),
        );
    };
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Endpoint {
        Source,
        Target,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rule {
        endpoint: Endpoint,
        pointer: String,
        equals: Value,
    }
    // Deserialize each rule after verifying `equals` is present: JSON null is a
    // meaningful expected value, not a missing field default.
    let rules = rules.as_array().context("expectations must be an array")?;
    let mut verdicts = Vec::new();
    for (index, value) in rules.iter().enumerate() {
        ensure!(
            value.get("equals").is_some(),
            "expectation {index} needs equals (null is allowed)"
        );
        let rule: Rule = serde_json::from_value(value.clone())
            .with_context(|| format!("expectation {index}"))?;
        ensure!(
            rule.pointer.is_empty() || rule.pointer.starts_with('/'),
            "expectation pointer must be RFC 6901 JSON Pointer"
        );
        let mut chars = rule.pointer.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                ensure!(
                    matches!(chars.next(), Some('0' | '1')),
                    "invalid expectation JSON Pointer escape"
                );
            }
        }
        let endpoint = match rule.endpoint {
            Endpoint::Source => "source",
            Endpoint::Target => "target",
        };
        let content = input
            .get(endpoint)
            .and_then(|end| end.get("region"))
            .and_then(|region| region.get("content"))
            .context("check input lacks Region content")?;
        let actual = content.pointer(&rule.pointer);
        verdicts.push(json!({
            "code":format!("expect.{index}"),
            "status":if actual == Some(&rule.equals) {"pass"} else {"fail"},
            "message":format!("{endpoint} content{}: expected {}, observed {}",rule.pointer,rule.equals,actual.map(Value::to_string).unwrap_or_else(|| "<missing>".into()))
        }));
    }
    Ok(Value::Array(verdicts))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assertions_distinguish_missing_and_null_and_reject_malformed_rules() -> Result<()> {
        let mut input = json!({"link":{"annotation":{"expect":[{"endpoint":"target","pointer":"/a","equals":null}]}},"target":{"region":{"content":{"a":null}}},"options":null});
        assert_eq!(check(&input)?[0]["status"], "pass");
        input["target"]["region"]["content"] = json!({});
        assert_eq!(check(&input)?[0]["status"], "fail");
        input["link"]["annotation"]["expect"][0]
            .as_object_mut()
            .unwrap()
            .remove("equals");
        assert!(check(&input).is_err());
        input["link"]["annotation"] = json!({});
        assert_eq!(check(&input)?[0]["status"], "not_applicable");
        Ok(())
    }
}
