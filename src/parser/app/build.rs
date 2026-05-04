use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=VERSION");
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git/logs/HEAD");
    println!("cargo:rerun-if-changed=../../../scripts/auto-version.sh");

    if let Some(version) = std::env::var("VERSION").ok().filter(|v| !v.is_empty()) {
        println!("cargo:rustc-env=VERSION={version}");
        return Ok(());
    }

    if let Some(version) = Command::new("../../../scripts/auto-version.sh")
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
