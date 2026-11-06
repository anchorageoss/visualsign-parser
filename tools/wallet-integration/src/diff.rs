//! Minimal parser for `git diff --unified=0` output.
//!
//! Only what the rules need: the path of each changed file, and every added or
//! removed line with its line number on the side it lives on. Context lines are
//! never requested (`--unified=0`), so hunks contain only `+` and `-` lines.

/// Which side of the diff a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Removed line; numbered against the base revision.
    Base,
    /// Added line; numbered against the head revision.
    Head,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedLine {
    pub side: Side,
    pub number: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Head path, or the base path for a deleted file.
    pub path: String,
    /// True for `Binary files ... differ` entries, which carry no lines.
    pub binary: bool,
    pub lines: Vec<ChangedLine>,
}

/// Parse `git diff --unified=0 --no-renames --src-prefix=a/ --dst-prefix=b/`
/// output. Lines that are not part of a recognised header or hunk are ignored.
pub fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_path: Option<String> = None;
    // File headers sit between `diff --git` and the first `@@`. Tracking that
    // keeps a removed line reading `-- x` (shown as `--- x`) out of the headers.
    let mut in_header = false;
    let (mut base_no, mut head_no) = (0u32, 0u32);
    for raw in diff.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with("diff --git ") {
            old_path = None;
            in_header = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some((b, h)) = hunk_starts(rest) {
                (base_no, head_no) = (b, h);
            }
            in_header = false;
            continue;
        }
        if !in_header {
            if let Some(file) = files.last_mut() {
                push_line(file, line, &mut base_no, &mut head_no);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("--- ") {
            old_path = strip_side_prefix(rest, "a/");
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            // `+++ /dev/null` is a deletion: report it under its old path.
            let path = strip_side_prefix(rest, "b/").or_else(|| old_path.clone());
            if let Some(path) = path {
                files.push(FileDiff {
                    path,
                    binary: false,
                    lines: Vec::new(),
                });
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("Binary files ") {
            if let Some(path) = binary_path(rest) {
                files.push(FileDiff {
                    path,
                    binary: true,
                    lines: Vec::new(),
                });
            }
        }
    }
    files
}

fn push_line(file: &mut FileDiff, line: &str, base_no: &mut u32, head_no: &mut u32) {
    if let Some(text) = line.strip_prefix('+') {
        file.lines.push(ChangedLine {
            side: Side::Head,
            number: *head_no,
            text: text.to_owned(),
        });
        *head_no += 1;
    } else if let Some(text) = line.strip_prefix('-') {
        file.lines.push(ChangedLine {
            side: Side::Base,
            number: *base_no,
            text: text.to_owned(),
        });
        *base_no += 1;
    }
}

/// `a/path` -> `path`; `/dev/null` -> None.
fn strip_side_prefix(rest: &str, prefix: &str) -> Option<String> {
    let rest = rest.trim_end();
    if rest == "/dev/null" {
        return None;
    }
    Some(rest.strip_prefix(prefix).unwrap_or(rest).to_owned())
}

/// `a/x and b/y differ` -> `y` (or `x` when the head side is /dev/null).
fn binary_path(rest: &str) -> Option<String> {
    let rest = rest.strip_suffix(" differ")?;
    let (old, new) = rest.split_once(" and ")?;
    strip_side_prefix(new, "b/").or_else(|| strip_side_prefix(old, "a/"))
}

/// `-12,3 +14,0 @@ ...` -> (12, 14). A count of 0 means the start names the
/// line *before* the (empty) range, which never gets a line of its own here.
fn hunk_starts(rest: &str) -> Option<(u32, u32)> {
    let mut parts = rest.split_whitespace();
    let base = parts.next()?.strip_prefix('-')?;
    let head = parts.next()?.strip_prefix('+')?;
    let start = |s: &str| s.split(',').next().and_then(|n| n.parse::<u32>().ok());
    Some((start(base)?, start(head)?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/proto/parser/parser.proto b/proto/parser/parser.proto
index 1da473f0..f5b67900 100644
--- a/proto/parser/parser.proto
+++ b/proto/parser/parser.proto
@@ -171,0 +172,2 @@ message SolanaMetadata {
+  // cap
+  optional uint64 max_fee_lamports = 6;
diff --git a/src/x.rs b/src/x.rs
index 002c1fd5..f14cca65 100644
--- a/src/x.rs
+++ b/src/x.rs
@@ -71 +73 @@ const A: &str = \"\";
-pub const V: u16 = 4;
+pub const V: u16 = 5;
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn gone() {}
diff --git a/d.bin b/d.bin
index 20d7637b..a3ba8c9e 100644
Binary files a/d.bin and b/d.bin differ
";

    #[test]
    fn parses_paths_sides_and_line_numbers() {
        let files = parse(SAMPLE);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["proto/parser/parser.proto", "src/x.rs", "gone.rs", "d.bin"]
        );

        assert_eq!(
            files[0].lines[1],
            ChangedLine {
                side: Side::Head,
                number: 173,
                text: "  optional uint64 max_fee_lamports = 6;".to_owned(),
            }
        );
        assert_eq!(
            files[1].lines,
            vec![
                ChangedLine {
                    side: Side::Base,
                    number: 71,
                    text: "pub const V: u16 = 4;".to_owned(),
                },
                ChangedLine {
                    side: Side::Head,
                    number: 73,
                    text: "pub const V: u16 = 5;".to_owned(),
                },
            ]
        );
        assert_eq!(files[2].lines[0].side, Side::Base);
        assert!(files[3].binary && files[3].lines.is_empty());
    }

    #[test]
    fn tolerates_crlf_and_garbage() {
        let files = parse(
            "noise\r\ndiff --git a/f b/f\r\n--- a/f\r\n+++ b/f\r\n@@ -1 +0,0 @@\r\n--- x\r\n",
        );
        assert_eq!(files.len(), 1);
        // A removed `-- x` line must not be read as a file header.
        assert_eq!(files[0].lines[0].text, "-- x");
        assert_eq!(files[0].lines[0].side, Side::Base);
    }
}
