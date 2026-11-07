//! The `apply` subcommand: write fetched IDLs over the committed preset
//! IDLs where they differ, together with their provenance records, and
//! emit one compare section per changed program for the report.
//!
//! Pure filesystem + JSON work: no network, no binary. The fetched files
//! come from the `solana-idl-fetch` workflow artifacts (`<program_id>.json`
//! and `<program_id>.provenance.json`); the committed locations come from
//! [`crate::presets`]. "Differs" means the parsed JSON differs, not the
//! bytes, so a reformat alone never produces a change.

use anyhow::{Context, Result, bail};
use solana_idl_fetch::compare;
use solana_idl_fetch::json::{self, Json};
use solana_idl_fetch::record::{record_path_for, sha256_hex, validate_record};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::presets::{PresetEntry, preset_entries};
use crate::sanitize;

/// Apply the fetched IDLs in `fetched_dir` over the committed preset IDLs
/// under `repo_root`, writing a compare section for each changed program
/// into `sections_dir` when given. Returns the changed program ids,
/// sorted. Files in `fetched_dir` that do not belong to a preset are
/// ignored.
pub fn apply(
    fetched_dir: &Path,
    repo_root: &Path,
    sections_dir: Option<&Path>,
) -> Result<Vec<String>> {
    let presets: BTreeMap<String, PresetEntry> = preset_entries()?
        .into_iter()
        .map(|entry| (entry.program_id.clone(), entry))
        .collect();
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(fetched_dir)
        .with_context(|| format!("failed to list {}", fetched_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to list {}", fetched_dir.display()))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();

    let mut changed: Vec<String> = Vec::new();
    for name in &names {
        let Some(program_id) = name.strip_suffix(".json") else {
            continue;
        };
        // Records ride along with their IDL, not on their own.
        if program_id.ends_with(".provenance") {
            continue;
        }
        let Some(preset) = presets.get(program_id) else {
            eprintln!("ignoring {name}: not a preset program id");
            continue;
        };
        let fetched_text = fs::read_to_string(fetched_dir.join(name))
            .with_context(|| format!("failed to read {}", name))?;
        let fetched = json::parse(&fetched_text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("{name} is not valid JSON"))?;
        let committed_path = repo_root.join(&preset.idl_path);
        let committed_text = fs::read_to_string(&committed_path).with_context(|| {
            format!(
                "failed to read the committed IDL {}",
                committed_path.display()
            )
        })?;
        let committed = json::parse(&committed_text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("{} is not valid JSON", committed_path.display()))?;
        if json::canonical(&fetched) == json::canonical(&committed) {
            continue;
        }
        let record_text = fs::read_to_string(fetched_dir.join(format!("{program_id}.provenance.json")))
            .with_context(|| {
                format!(
                    "the on-chain IDL for {program_id} differs but {} holds no provenance record for it",
                    fetched_dir.display()
                )
            })?;
        let mut record = json::parse(&record_text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("the provenance record for {program_id} is not valid JSON"))?;
        if record.get("program_id").and_then(Json::as_str) != Some(program_id) {
            bail!("the provenance record for {program_id} names another program id");
        }

        // Write the IDL in the canonical 2-space form (the form
        // solana-idl-fetch writes), then re-point the record's digest at
        // the exact bytes written -- a no-op for a well-formed artifact,
        // required if the IDL bytes were reformatted anywhere along the
        // way. The visualsign-solana provenance test pins this hash.
        let idl_text = format!("{}\n", json::pretty(&fetched));
        set_field(
            &mut record,
            "idl_sha256",
            Json::str(&sha256_hex(idl_text.as_bytes())),
        )?;
        validate_record(&record)
            .map_err(anyhow::Error::msg)
            .context("the provenance record from the fetch does not validate")?;

        fs::write(&committed_path, idl_text.as_bytes())
            .with_context(|| format!("failed to write {}", committed_path.display()))?;
        let record_dest = record_path_for(&committed_path)?;
        fs::write(&record_dest, format!("{}\n", json::pretty(&record)))
            .with_context(|| format!("failed to write {}", record_dest.display()))?;
        if let Some(dir) = sections_dir {
            write_compare_section(dir, program_id, &fetched, &committed)?;
        }
        eprintln!(
            "{program_id}: IDL updated ({}), record at {}",
            preset.idl_path,
            record_dest.display()
        );
        changed.push(program_id.to_string());
    }
    changed.sort();
    Ok(changed)
}

/// The `apply` subcommand's stdout: the changed program ids as a
/// single-line JSON array.
pub fn changed_json(changed: &[String]) -> String {
    json::canonical(&Json::Array(
        changed.iter().map(|id| Json::str(id)).collect(),
    ))
}

/// Replace `key` in an object, keeping the position: the record format is
/// pinned to its key order, so the rewrite stays byte-identical apart
/// from the replaced value.
fn set_field(record: &mut Json, key: &str, value: Json) -> Result<()> {
    let Json::Object(fields) = record else {
        bail!("record is not a JSON object");
    };
    match fields.iter_mut().find(|(field, _)| field == key) {
        Some(slot) => {
            slot.1 = value;
            Ok(())
        }
        None => bail!("record has no {key:?} field"),
    }
}

/// One `compare-<program_id>.md` section for the report, carrying the
/// `solana-idl-fetch --compare` summary of the change.
fn write_compare_section(
    dir: &Path,
    program_id: &str,
    fetched: &Json,
    committed: &Json,
) -> Result<()> {
    let lines = compare::compare_lines(fetched, committed)?;
    let section = format!(
        "### `{program_id}`\n\nThe on-chain IDL differs from the committed one:\n\n{}\n",
        sanitize::fenced(&lines.join("\n"))
    );
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(format!("compare-{program_id}.md"));
    fs::write(&path, section.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use solana_idl_fetch::record::{onchain_record, validate_record};

    use super::*;
    use crate::presets::preset_entries;

    /// A repo-root-shaped temp dir with the committed IDL of one preset
    /// (sorted first, so the tests are deterministic) in place.
    struct Fixture {
        repo: tempfile::TempDir,
        fetched: tempfile::TempDir,
        sections: tempfile::TempDir,
        preset: PresetEntry,
    }

    impl Fixture {
        fn new(committed_json: &str) -> Self {
            let preset = preset_entries().unwrap().remove(0);
            let repo = tempfile::tempdir().unwrap();
            let idl = repo.path().join(&preset.idl_path);
            fs::create_dir_all(idl.parent().unwrap()).unwrap();
            fs::write(&idl, committed_json).unwrap();
            Fixture {
                repo,
                fetched: tempfile::tempdir().unwrap(),
                sections: tempfile::tempdir().unwrap(),
                preset,
            }
        }

        fn idl_path(&self) -> std::path::PathBuf {
            self.repo.path().join(&self.preset.idl_path)
        }

        fn record_path(&self) -> std::path::PathBuf {
            self.repo.path().join(self.preset.record_path())
        }

        fn put_fetched(&self, idl_text: &str) {
            let record = onchain_record(
                &self.preset.program_id,
                &sha256_hex(idl_text.as_bytes()),
                "2026-01-01T00:00:00Z",
                "Some1DLAccount11111111111111111111111111111",
                123,
                Some(456),
            );
            fs::write(
                self.fetched
                    .path()
                    .join(format!("{}.json", self.preset.program_id)),
                idl_text,
            )
            .unwrap();
            fs::write(
                self.fetched
                    .path()
                    .join(format!("{}.provenance.json", self.preset.program_id)),
                record,
            )
            .unwrap();
        }
    }

    #[test]
    fn differing_idl_is_written_with_its_record_and_section() {
        let committed = r#"{"instructions": [{"name": "old"}]}"#;
        let fixture = Fixture::new(committed);
        let fetched_text =
            "{\n  \"instructions\": [\n    {\"name\": \"old\"},\n    {\"name\": \"new\"}\n  ]\n}\n";
        fixture.put_fetched(fetched_text);

        let changed = apply(
            fixture.fetched.path(),
            fixture.repo.path(),
            Some(fixture.sections.path()),
        )
        .unwrap();
        assert_eq!(changed, vec![fixture.preset.program_id.clone()]);

        // The committed IDL now holds the fetched content in the
        // canonical pretty form.
        let written = fs::read_to_string(fixture.idl_path()).unwrap();
        assert_eq!(
            written,
            "{\n  \"instructions\": [\n    {\n      \"name\": \"old\"\n    },\n    {\n      \"name\": \"new\"\n    }\n  ]\n}\n"
        );

        // The record sits next to the IDL, as `<idl stem>.provenance.json`,
        // and hashes the exact bytes written.
        let record_text = fs::read_to_string(fixture.record_path()).unwrap();
        let record = json::parse(&record_text).unwrap();
        validate_record(&record).unwrap();
        assert_eq!(
            record.get("idl_sha256").and_then(Json::as_str).unwrap(),
            sha256_hex(written.as_bytes())
        );
        assert_eq!(
            record.get("program_id").and_then(Json::as_str).unwrap(),
            fixture.preset.program_id
        );
        // The record keeps its canonical key order and 2-space form.
        assert_eq!(record_text, format!("{}\n", json::pretty(&record)));

        // The compare section names the program and summarizes the change.
        let section = fs::read_to_string(
            fixture
                .sections
                .path()
                .join(format!("compare-{}.md", fixture.preset.program_id)),
        )
        .unwrap();
        assert!(section.contains(&format!("### `{}`", fixture.preset.program_id)));
        assert!(section.contains("identical: no"));
        assert!(section.contains("added: new"));

        // Re-running with the same fetched dir is a no-op: the committed
        // IDL now matches as parsed JSON.
        let changed_again = apply(
            fixture.fetched.path(),
            fixture.repo.path(),
            Some(fixture.sections.path()),
        )
        .unwrap();
        assert!(changed_again.is_empty());
        assert_eq!(
            fs::read_to_string(fixture.idl_path()).unwrap(),
            written,
            "an unchanged IDL must not be rewritten"
        );
    }

    #[test]
    fn identical_json_with_different_bytes_is_not_a_change() {
        let committed =
            "{\n  \"instructions\": [\n    {\n      \"name\": \"noop\"\n    }\n  ]\n}\n";
        let fixture = Fixture::new(committed);
        // Same JSON, compact and with keys reordered.
        fixture.put_fetched("{\"instructions\":[{\"name\":\"noop\"}]}\n");
        let changed = apply(fixture.fetched.path(), fixture.repo.path(), None).unwrap();
        assert!(changed.is_empty());
        assert_eq!(
            fs::read_to_string(fixture.idl_path()).unwrap(),
            committed,
            "a byte-level reformat must not rewrite the committed IDL"
        );
    }

    #[test]
    fn files_that_do_not_belong_to_a_preset_are_ignored() {
        let fixture = Fixture::new("{\"instructions\": []}");
        // A plausible-looking program id that is no preset of ours...
        fs::write(
            fixture
                .fetched
                .path()
                .join("NotAPreset111111111111111111111111111.json"),
            "{\"instructions\": []}",
        )
        .unwrap();
        // ...a stray file that is not JSON-named...
        fs::write(fixture.fetched.path().join("notes.txt"), "hello").unwrap();
        // ...and a record without its IDL.
        fs::write(
            fixture
                .fetched
                .path()
                .join(format!("{}.provenance.json", fixture.preset.program_id)),
            "{}",
        )
        .unwrap();
        let changed = apply(fixture.fetched.path(), fixture.repo.path(), None).unwrap();
        assert!(changed.is_empty());
        assert!(!fixture.record_path().exists());
    }

    #[test]
    fn a_differing_idl_without_a_record_is_an_error() {
        let fixture = Fixture::new("{\"instructions\": []}");
        let idl_path = fixture
            .fetched
            .path()
            .join(format!("{}.json", fixture.preset.program_id));
        fs::write(&idl_path, "{\"instructions\": [{\"name\": \"new\"}]}").unwrap();
        let err = apply(fixture.fetched.path(), fixture.repo.path(), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no provenance record"), "{err}");
        assert_eq!(
            fs::read_to_string(fixture.idl_path()).unwrap(),
            "{\"instructions\": []}",
            "a failed apply must not leave a half-updated tree"
        );
    }

    #[test]
    fn a_record_naming_another_program_is_an_error() {
        let fixture = Fixture::new("{\"instructions\": []}");
        fixture.put_fetched("{\"instructions\": [{\"name\": \"new\"}]}");
        // Corrupt the record's program id after putting a valid one.
        let record_path = fixture
            .fetched
            .path()
            .join(format!("{}.provenance.json", fixture.preset.program_id));
        let mut record = json::parse(&fs::read_to_string(&record_path).unwrap()).unwrap();
        set_field(
            &mut record,
            "program_id",
            Json::str("11111111111111111111111111111111"),
        )
        .unwrap();
        fs::write(&record_path, json::pretty(&record)).unwrap();

        let err = apply(fixture.fetched.path(), fixture.repo.path(), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("another program id"), "{err}");
    }
}
