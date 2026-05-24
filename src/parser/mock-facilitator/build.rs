fn main() {
    println!(
        "cargo:rustc-env=VERSION={}",
        std::env::var("VERSION").unwrap_or_else(|_| "0.0.0-dev".to_string())
    );
    println!("cargo:rerun-if-env-changed=VERSION");
}
