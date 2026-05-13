use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let git_dir = workspace_root.join(".git");

    println!("cargo:rerun-if-env-changed=VERSION");
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("logs/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("refs/remotes/origin/main").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("refs/remotes/origin/master").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("scripts/auto-version.sh").display()
    );

    if let Some(version) = std::env::var("VERSION").ok().filter(|v| !v.is_empty()) {
        println!("cargo:rustc-env=VERSION={version}");
        return Ok(());
    }

    if let Some(version) = Command::new(workspace_root.join("scripts/auto-version.sh"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
    {
        println!("cargo:rustc-env=VERSION={version}");
        return Ok(());
    }

    println!("cargo:rustc-env=VERSION=dev");
    Ok(())
}
