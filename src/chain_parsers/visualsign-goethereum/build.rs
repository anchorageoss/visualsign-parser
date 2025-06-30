use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let project_dir = env::current_dir().unwrap();

    // Find the Go source file - fix path to match actual directory structure
    let go_source_dir = project_dir.join("go");
    let go_source_path = go_source_dir.join("lib.go");

    println!("Building Go library from: {}", go_source_path.display());

    // Initialize Go module if it doesn't exist
    let go_mod_path = project_dir.join("go.mod");
    if !go_mod_path.exists() {
        let status = Command::new("go")
            .current_dir(&project_dir)
            .args(&["mod", "tidy"])
            .status()
            .expect("Failed to tidy Go module");

        if !status.success() {
            panic!("Failed to tidy Go module");
        }
    }

    // Build the Go library with static linking flags
    let mut cmd = Command::new("go");
    cmd.current_dir(&project_dir); // Build from the project root where go.mod is

    // Set environment variables for static linking
    cmd.env("CGO_ENABLED", "1");
    cmd.env("CGO_LDFLAGS", "-static");

    // For musl target, set the C compiler
    if env::var("TARGET").unwrap_or_default().contains("musl") {
        cmd.env("CC", "musl-gcc");
        cmd.env("CGO_LDFLAGS", "-static -extldflags '-static'");
    }

    let status = cmd
        .args(&[
            "build",
            "-buildmode=c-archive",
            "-ldflags",
            "-extldflags=-static",
            "-o",
        ])
        .arg(&out_dir.join("libgo_lib.a"))
        .arg("./go") // Build the entire go package
        .status()
        .expect("Failed to build Go library");

    if !status.success() {
        panic!("Failed to compile Go library");
    }

    // Tell cargo to link the static library and required system libraries
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=go_lib");

    // Add required system libraries for Go runtime (keep some dynamic to avoid issues)
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=rt");

    // Try to link libgcc statically to reduce dependencies
    println!("cargo:rustc-link-arg=-static-libgcc");

    // Let cargo know to re-run if the Go files change
    println!(
        "cargo:rerun-if-changed={}",
        go_source_dir.join("lib.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_source_dir.join("visualsign.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_source_dir.join("clib.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        project_dir.join("go.mod").display()
    );

    // Generate bindings for the Go library
    // Note: We use manually crafted bindings instead of bindgen to avoid
    // compatibility issues with complex Go-generated headers
    let header_path = out_dir.join("libgo_lib.h");

    // Check if header file exists before trying to generate bindings
    if !header_path.exists() {
        panic!("Header file not found: {}", header_path.display());
    }

    println!(
        "Header file exists, size: {} bytes",
        std::fs::metadata(&header_path).unwrap().len()
    );

    // Manual bindings that match the Go C header interface
    let bindings_content = r#"
/* Manually crafted bindings for Go library - matches libgo_lib.h */

pub type GoInt = ::std::os::raw::c_longlong;

#[repr(C)]
pub struct EthTransaction {
    pub hash: *mut ::std::os::raw::c_char,
    pub from: *mut ::std::os::raw::c_char,
    pub to: *mut ::std::os::raw::c_char,
    pub value: *mut ::std::os::raw::c_char,
    pub nonce: ::std::os::raw::c_ulonglong,
    pub input_data: *mut ::std::os::raw::c_char,
    pub gas: ::std::os::raw::c_ulonglong,
    pub gas_price: *mut ::std::os::raw::c_char,
}

unsafe extern "C" {
    pub fn HelloFromGo(name: *mut ::std::os::raw::c_char) -> *mut ::std::os::raw::c_char;
    pub fn AddNumbers(a: GoInt, b: GoInt) -> GoInt;
    pub fn GoFree(ptr: *mut ::std::os::raw::c_char);
    pub fn FreeEthTransaction(tx: *mut EthTransaction);
    pub fn DecodeEthereumTransaction(rawTxHex: *mut ::std::os::raw::c_char) -> *mut EthTransaction;
    pub fn DecodeEthereumTransactionToJSON(rawTxHex: *mut ::std::os::raw::c_char) -> *mut ::std::os::raw::c_char;
    pub fn FreeString(s: *mut ::std::os::raw::c_char);
}
"#;

    std::fs::write(out_dir.join("bindings.rs"), bindings_content)
        .expect("Couldn't write bindings file");
}
