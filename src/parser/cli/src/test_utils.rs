/// Write `content` to a uniquely-named temporary file and return its path.
///
/// Filenames include the PID and a nanosecond timestamp to avoid collisions
/// when tests run in parallel.
pub fn write_temp_json(subdir: &str, name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(subdir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
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
