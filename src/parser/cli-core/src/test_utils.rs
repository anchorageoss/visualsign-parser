/// Write `content` to a uniquely-named temporary file and return its path.
///
/// Filenames include the PID and a nanosecond timestamp to avoid collisions
/// when tests run in parallel.
///
/// Call sites pass trusted string literals, but as a defense-in-depth measure
/// this function canonicalizes the resolved directory and verifies it is
/// contained within `std::env::temp_dir()`. The check prevents path-traversal
/// sequences (e.g. `../`) from escaping the temp directory even if a caller
/// inadvertently passes tainted input.
///
/// # Panics
///
/// Panics if the temp directory cannot be created, the file cannot be written,
/// or the resolved path escapes the temp directory.
#[must_use]
#[allow(clippy::expect_used)]
pub fn write_temp_json(subdir: &str, name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(subdir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // Defense-in-depth: confirm the resolved directory is still inside temp_dir.
    let temp_canonical =
        std::fs::canonicalize(std::env::temp_dir()).expect("canonicalize temp_dir");
    let dir_canonical = std::fs::canonicalize(&dir).expect("canonicalize subdir");
    assert!(
        dir_canonical.starts_with(&temp_canonical),
        "write_temp_json: resolved path `{}` escapes temp dir `{}`",
        dir_canonical.display(),
        temp_canonical.display(),
    );

    let path = dir.join(format!(
        "{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::write(&path, content).expect("write temp file");
    path
}
