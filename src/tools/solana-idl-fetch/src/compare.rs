//! The `--compare` summary: how an on-chain IDL differs from a committed
//! one. Pure; takes two parsed IDLs, returns lines.

use anyhow::{Result, bail};
use std::collections::BTreeMap;

use crate::json::{self, Json};

/// Signature of one instruction, in the three comparison keys the Python
/// script used: discriminator, argument names, account names. Each part is
/// rendered canonically so comparing parts matches Python's tuple
/// comparison.
#[derive(Debug, PartialEq)]
struct Signature {
    discriminator: String,
    args: String,
    accounts: String,
}

/// Canonical render of the empty list, the default for all three parts.
fn empty_list() -> String {
    json::canonical(&Json::Array(Vec::new()))
}

/// Canonical render of an instruction list's member names (missing `name`
/// renders as null, the way Python's tuple kept `None`).
fn names_of(value: Option<&Json>) -> Result<String> {
    let items = match value {
        // Python: `ix.get("args", [])` only defaults when the key is
        // absent; a JSON null crashes there. Treating null as empty is the
        // only intentional (and unobservable for real IDLs) deviation.
        None | Some(Json::Null) => return Ok(empty_list()),
        Some(Json::Array(items)) => items,
        Some(_) => bail!("expected an array, found something else"),
    };
    let mut names = Vec::new();
    for item in items {
        let Json::Object(_) = item else {
            bail!("expected an object, found something else");
        };
        names.push(item.get("name").cloned().unwrap_or(Json::Null));
    }
    Ok(json::canonical(&Json::Array(names)))
}

fn instruction_signatures(idl: &Json) -> Result<BTreeMap<String, Signature>> {
    let mut sigs = BTreeMap::new();
    let items = match idl.get("instructions") {
        None => Vec::new(),
        Some(Json::Array(items)) => items.clone(),
        Some(_) => bail!("IDL 'instructions' is not an array"),
    };
    for ix in &items {
        let Json::Object(_) = ix else {
            bail!("IDL instruction is not an object");
        };
        let Some(name) = ix.get("name").and_then(Json::as_str) else {
            bail!("IDL instruction has no string 'name'");
        };
        let discriminator = match ix.get("discriminator") {
            None | Some(Json::Null) => empty_list(),
            Some(value @ Json::Array(_)) => json::canonical(value),
            Some(_) => bail!("IDL instruction 'discriminator' is not an array"),
        };
        let sig = Signature {
            discriminator,
            args: names_of(ix.get("args"))?,
            accounts: names_of(ix.get("accounts"))?,
        };
        sigs.insert(name.to_string(), sig);
    }
    Ok(sigs)
}

fn joined(items: &[&str]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

/// The `--compare` summary lines, exactly as the Python script printed
/// them (to stderr):
///
/// ```text
/// identical: yes|no
/// instructions: committed N, on chain M
/// added: a, b (or -)
/// removed: c (or -)
/// changed: d (or -)
/// ```
pub fn compare_lines(onchain: &Json, committed: &Json) -> Result<Vec<String>> {
    let new = instruction_signatures(onchain)?;
    let old = instruction_signatures(committed)?;
    // BTreeMap keys are already sorted, like Python's sorted() over names.
    let added: Vec<&str> = new
        .keys()
        .filter(|name| !old.contains_key(*name))
        .map(String::as_str)
        .collect();
    let removed: Vec<&str> = old
        .keys()
        .filter(|name| !new.contains_key(*name))
        .map(String::as_str)
        .collect();
    let changed: Vec<&str> = new
        .iter()
        .filter(|(name, sig)| match old.get(*name) {
            Some(old_sig) => old_sig != *sig,
            None => false,
        })
        .map(|(name, _)| name.as_str())
        .collect();
    let identical = json::canonical(onchain) == json::canonical(committed);
    Ok(vec![
        format!("identical: {}", if identical { "yes" } else { "no" }),
        format!(
            "instructions: committed {}, on chain {}",
            old.len(),
            new.len()
        ),
        format!("added: {}", joined(&added)),
        format!("removed: {}", joined(&removed)),
        format!("changed: {}", joined(&changed)),
    ])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn summary_lines_match_the_python_format() {
        let committed = json::parse(
            r#"{"instructions":[
                {"name":"noop"},
                {"name":"old","discriminator":[1]},
                {"name":"same","args":[{"name":"x"}],"accounts":[{"name":"y"}]},
                {"name":"gone"}]}"#,
        )
        .unwrap();
        let onchain = json::parse(
            r#"{"instructions":[
                {"name":"noop"},
                {"name":"new","discriminator":[2,3]},
                {"name":"same","args":[{"name":"x"}],"accounts":[{"name":"y"}]},
                {"name":"old","discriminator":[9]}]}"#,
        )
        .unwrap();
        assert_eq!(
            compare_lines(&onchain, &committed).unwrap(),
            vec![
                "identical: no",
                "instructions: committed 4, on chain 4",
                "added: new",
                "removed: gone",
                "changed: old",
            ]
        );
        // Identical content (any key order) collapses to "yes".
        let reordered = json::parse(
            r#"{"metadata":{},"instructions":[{"accounts":[],"args":[],"discriminator":[7],"name":"noop"}]}"#,
        )
        .unwrap();
        let same_twice = json::parse(
            r#"{"instructions":[{"name":"noop","discriminator":[7],"args":[],"accounts":[]}],"metadata":{}}"#,
        )
        .unwrap();
        assert_eq!(
            compare_lines(&same_twice, &reordered).unwrap(),
            vec![
                "identical: yes",
                "instructions: committed 1, on chain 1",
                "added: -",
                "removed: -",
                "changed: -",
            ]
        );
    }

    #[test]
    fn changed_accounts_and_args_are_detected() {
        let old_idl = json::parse(
            r#"{"instructions":[{"name":"ix","args":[{"name":"a"}],"accounts":[{"name":"b"}]}]}"#,
        )
        .unwrap();
        let arg_changed = json::parse(
            r#"{"instructions":[{"name":"ix","args":[{"name":"z"}],"accounts":[{"name":"b"}]}]}"#,
        )
        .unwrap();
        let account_changed = json::parse(
            r#"{"instructions":[{"name":"ix","args":[{"name":"a"}],"accounts":[{"name":"z"}]}]}"#,
        )
        .unwrap();
        let lines = compare_lines(&arg_changed, &old_idl).unwrap();
        assert_eq!(lines[4], "changed: ix");
        let lines = compare_lines(&account_changed, &old_idl).unwrap();
        assert_eq!(lines[4], "changed: ix");
    }

    #[test]
    fn missing_instructions_is_an_empty_idl() {
        let idl = json::parse(r#"{"instructions":[]}"#).unwrap();
        let none = json::parse("{}").unwrap();
        assert_eq!(
            compare_lines(&none, &idl).unwrap(),
            vec![
                "identical: no",
                "instructions: committed 0, on chain 0",
                "added: -",
                "removed: -",
                "changed: -",
            ]
        );
    }

    #[test]
    fn non_array_instructions_is_an_error() {
        let idl = json::parse(r#"{"instructions":42}"#).unwrap();
        let ok = json::parse("{}").unwrap();
        assert!(compare_lines(&idl, &ok).is_err());
    }
}
