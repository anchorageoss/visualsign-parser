#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Helper to run the parser_cli binary with given args and return (stdout, stderr).
fn run_cli_full(args: &[&str]) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
        .args(args)
        .output()
        .expect("Failed to execute parser_cli");
    assert!(
        output.status.success(),
        "CLI exited with error. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("Invalid UTF-8 output"),
        String::from_utf8(output.stderr).expect("Invalid UTF-8 stderr"),
    )
}

/// Helper to run the parser_cli binary with given args and return stdout.
fn run_cli(args: &[&str]) -> String {
    run_cli_full(args).0
}

/// Helper to write a temp JSON file and return its path.
/// Duplicated from `test_utils::write_temp_json` because `crate::test_utils`
/// is behind `#[cfg(test)]` in `lib.rs` and therefore not compiled for
/// integration tests.
fn write_temp_json(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("vsp_cli_tests");
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(format!(
        "{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::write(&path, content).expect("write temp file");
    path
}

#[test]
fn test_cli_with_fixtures() {
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");

    let test_cases = fs::read_dir(&fixtures_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".input")
        });

    // Fixture names are prefixed with their chain (e.g. "solana-json", "ethereum-text").
    // Build the skip list once from compile-time feature flags.
    let disabled_chain_prefixes: &[&str] = &[
        #[cfg(not(feature = "ethereum"))]
        "ethereum",
        #[cfg(not(feature = "solana"))]
        "solana",
    ];

    for input_file in test_cases {
        let input_path = input_file.path();
        let test_name = input_path.file_stem().unwrap().to_str().unwrap();

        if disabled_chain_prefixes
            .iter()
            .any(|p| test_name.starts_with(p))
        {
            println!("Skipping fixture '{test_name}' (chain feature not enabled)");
            continue;
        }

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {input_path:?}"));

        let mut command = Command::new(env!("CARGO_BIN_EXE_parser_cli"));
        // Pin CWD so fixtures referencing relative paths (e.g. `@tests/fixtures/foo.hex`)
        // resolve regardless of how `cargo test` is invoked.
        command.current_dir(env!("CARGO_MANIFEST_DIR"));
        for line in input_contents.lines() {
            if !line.trim().is_empty() {
                command.arg(line);
            }
        }

        // Run the CLI program with the input file
        let output = command
            .output()
            .unwrap_or_else(|e| panic!("Failed to execute CLI: {e}"));
        println!("Output {test_name:?}: {output:?}");

        // Construct expected output path
        let expected_path = fixtures_dir.join(format!("{test_name}.expected"));

        // Read expected output
        let expected_output = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("Expected output file not found: {expected_path:?}"));

        let actual_output = String::from_utf8(output.stdout)
            .unwrap_or_else(|e| panic!("Invalid UTF-8 output: {e}"));

        let expected = expected_output.trim();
        let actual = actual_output.trim();

        if expected != actual {
            let diff = TextDiff::from_lines(expected, actual);
            let mut diff_output = String::new();

            for change in diff.iter_all_changes() {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                diff_output.push_str(&format!("{sign}{change}"));
            }

            panic!("Test case '{test_name}' failed:\n{diff_output}");
        }
    }
}

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_transaction_from_stdin() {
    // Same hex as `tests/fixtures/ethereum-from-file.hex`, fed via `@-`.
    let hex = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("ethereum-from-file.hex"),
    )
    .expect("read hex fixture");

    let mut child = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
        .args([
            "decode",
            "--chain",
            "ethereum",
            "--network",
            "ETHEREUM_MAINNET",
            "-o",
            "json",
            "-t",
            "@-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn parser_cli");

    child
        .stdin
        .as_mut()
        .expect("stdin handle")
        .write_all(hex.as_bytes())
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait parser_cli");
    assert!(
        output.status.success(),
        "CLI exited non-zero. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Ethereum Transaction");
}

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_transaction_at_missing_file_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
        .args([
            "decode",
            "--chain",
            "ethereum",
            "--network",
            "ETHEREUM_MAINNET",
            "-t",
            "@/this/path/does/not/exist.hex",
        ])
        .output()
        .expect("execute parser_cli");

    assert!(!output.status.success(), "CLI should fail on missing file");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Failed to open transaction file"),
        "stderr should mention failure to open, got: {stderr}"
    );
}

/// ERC-20 transfer(address,uint256) to an unknown contract 0x1111...1111.
/// Without a custom ABI mapping the built-in ERC-20 visualizer decodes it as
/// "ERC20 Transfer". With a custom ABI the dynamic decoder takes over and
/// shows the raw function signature "transfer(address,uint256)".
const ERC20_TRANSFER_TX: &str = "02f86c0180830f4240843b9aca00830186a094111111111111111111111111111111111111111180b844a9059cbb000000000000000000000000000000000000000000000000000000000000dead00000000000000000000000000000000000000000000000000000000000f4240c0";

const ERC20_TRANSFER_ABI: &str = r#"[{
    "type": "function",
    "name": "transfer",
    "inputs": [
        {"name": "to", "type": "address"},
        {"name": "amount", "type": "uint256"}
    ],
    "outputs": [{"name": "", "type": "bool"}],
    "stateMutability": "nonpayable"
}]"#;

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_ethereum_abi_json_mappings() {
    let abi_path = write_temp_json("erc20_transfer.json", ERC20_TRANSFER_ABI);
    let mapping = format!(
        "TestERC20:{}:0x1111111111111111111111111111111111111111",
        abi_path.display()
    );

    let output = run_cli(&[
        "decode",
        "--chain",
        "ethereum",
        "--network",
        "ETHEREUM_MAINNET",
        "-o",
        "json",
        "--abi-json-mappings",
        &mapping,
        "-t",
        ERC20_TRANSFER_TX,
    ]);

    // Dynamic ABI decoder produces "transfer" as the function name in the title
    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");

    let fields = json["Fields"]
        .as_array()
        .expect("Fields should be an array");

    // Find the preview_layout field produced by the dynamic ABI decoder
    let abi_field = fields.iter().find(|f| {
        f["Type"] == "preview_layout"
            && f["PreviewLayout"]["Title"]["Text"]
                .as_str()
                .is_some_and(|t| t == "transfer")
    });
    assert!(
        abi_field.is_some(),
        "Expected a preview_layout with Title 'transfer' from dynamic ABI decoding, got: {output}"
    );

    // Verify the function signature appears in the subtitle
    let subtitle = abi_field.unwrap()["PreviewLayout"]["Subtitle"]["Text"]
        .as_str()
        .unwrap();
    assert!(
        subtitle.contains("transfer(address,uint256)"),
        "Subtitle should contain function signature, got: {subtitle}"
    );
}

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_ethereum_without_abi_uses_builtin_visualizer() {
    let output = run_cli(&[
        "decode",
        "--chain",
        "ethereum",
        "--network",
        "ETHEREUM_MAINNET",
        "-o",
        "json",
        "-t",
        ERC20_TRANSFER_TX,
    ]);

    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");

    let fields = json["Fields"]
        .as_array()
        .expect("Fields should be an array");

    // Without a custom ABI, the built-in ERC-20 visualizer should handle it
    let erc20_field = fields
        .iter()
        .find(|f| f["Label"].as_str().is_some_and(|l| l == "ERC20 Transfer"));
    assert!(
        erc20_field.is_some(),
        "Expected built-in 'ERC20 Transfer' field without custom ABI, got: {output}"
    );
}

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_ethereum_abi_invalid_file_still_parses() {
    // Pointing to a nonexistent ABI file should not crash the CLI —
    // it should fall back to the built-in visualizer.
    let mapping = "Bad:/nonexistent/abi.json:0x1111111111111111111111111111111111111111";

    let output = run_cli(&[
        "decode",
        "--chain",
        "ethereum",
        "--network",
        "ETHEREUM_MAINNET",
        "-o",
        "json",
        "--abi-json-mappings",
        mapping,
        "-t",
        ERC20_TRANSFER_TX,
    ]);

    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Ethereum Transaction");
}

#[test]
#[cfg(feature = "solana")]
fn test_cli_solana_idl_json_mappings() {
    // Minimal Anchor IDL — enough to verify the flag is wired through.
    // The actual IDL won't match the transaction's program, but it should
    // still load without error and be present in metadata.
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "instructions": []
    }"#;
    let idl_path = write_temp_json("test_idl.json", idl_json);
    let mapping = format!(
        "TestProgram:{}:TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        idl_path.display()
    );

    // Use the same Solana transaction from the existing fixture
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let input =
        fs::read_to_string(fixtures_dir.join("solana-json.input")).expect("read solana-json.input");

    let mut args: Vec<&str> = Vec::new();
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    for line in &lines {
        args.push(line);
    }
    args.push("--idl-json-mappings");
    args.push(&mapping);

    let (stdout, stderr) = run_cli_full(&args);

    // Verify the mapping was actually loaded — check the per-entry success line
    assert!(
        stderr.contains("Loaded IDL 'TestProgram'"),
        "Expected per-mapping 'Loaded IDL' line in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("Successfully loaded 1/1 IDL mappings"),
        "Expected 1/1 IDL mappings loaded, got: {stderr}"
    );

    // The transaction should still parse successfully
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Solana Transaction");
    assert!(json["Fields"].as_array().is_some_and(|f| !f.is_empty()));
}

#[test]
#[cfg(all(feature = "ethereum", feature = "serve"))]
fn test_cli_serve_renders_directory() {
    use std::io::{BufRead, BufReader, Read as _};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    // Use the existing ethereum-from-file.hex as a one-file fixture directory.
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");

    // Ask the OS for a free port: bind ephemeral, drop, race-tolerable since
    // the server immediately re-binds.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);

    let mut child = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
        .args([
            "serve",
            "--chain",
            "ethereum",
            "--network",
            "ETHEREUM_MAINNET",
            "--dir",
            fixture_dir.to_str().unwrap(),
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn parser_cli serve");

    // Wait for the "Serving on" line on stdout, with a hard timeout.
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ready = false;
    let mut greeting = String::new();
    while Instant::now() < deadline {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        greeting.push_str(&line);
        if line.contains("Serving on") {
            ready = true;
            break;
        }
    }
    assert!(ready, "server never printed ready line. got: {greeting}");

    // Hit `/` over a raw TCP socket — keep the test free of new HTTP-client deps.
    let body = http_get(port, "/");
    assert!(
        body.contains("<title>parser_cli serve</title>"),
        "got: {body}"
    );
    assert!(body.contains("ethereum-from-file.hex"));
    assert!(body.contains("Ethereum Transaction"));

    // JSON endpoint round-trip.
    let api = http_get(port, "/api/file?path=ethereum-from-file.hex");
    let parsed: serde_json::Value = {
        // body is "headers\r\n\r\nbody"
        let payload = api.split("\r\n\r\n").nth(1).unwrap_or("");
        serde_json::from_str(payload).expect("api response should be JSON")
    };
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["payload"]["Title"], "Ethereum Transaction");

    // Cleanup.
    let _ = child.kill();
    let _ = child.wait();

    fn http_get(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("tcp connect");
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        std::io::Write::write_all(&mut s, req.as_bytes()).expect("write req");
        let mut buf = String::new();
        s.read_to_string(&mut buf).expect("read response");
        buf
    }
}

#[test]
#[cfg(all(feature = "ethereum", feature = "serve"))]
fn test_cli_serve_live_reload_and_json_passthrough() {
    use std::io::{BufRead, BufReader, Read as _};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    // Working directory we can mutate between requests.
    let work = std::env::temp_dir().join(format!(
        "vsp_serve_live_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&work).unwrap();

    let valid_hex = "02f86c0180830f4240843b9aca00830186a094111111111111111111111111111111111111111180b844a9059cbb000000000000000000000000000000000000000000000000000000000000dead00000000000000000000000000000000000000000000000000000000000f4240c0";
    fs::write(work.join("tx.hex"), valid_hex).unwrap();
    fs::write(work.join("expected.json"), r#"{"sentinel":"first"}"#).unwrap();

    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);

    let mut child = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
        .args([
            "serve",
            "--chain",
            "ethereum",
            "--network",
            "ETHEREUM_MAINNET",
            "--dir",
            work.to_str().unwrap(),
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn parser_cli serve");

    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ready = false;
    let mut greeting = String::new();
    while Instant::now() < deadline {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        greeting.push_str(&line);
        if line.contains("Serving on") {
            ready = true;
            break;
        }
    }
    assert!(ready, "server never printed ready line. got: {greeting}");

    fn http_get(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("tcp connect");
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        std::io::Write::write_all(&mut s, req.as_bytes()).expect("write req");
        let mut buf = String::new();
        s.read_to_string(&mut buf).expect("read response");
        buf
    }

    fn parse_json_body(api: &str) -> serde_json::Value {
        let payload = api.split("\r\n\r\n").nth(1).unwrap_or("");
        serde_json::from_str(payload).expect("api response should be JSON")
    }

    // 1. Initial: hex decodes, json passes through verbatim.
    let hex_resp = parse_json_body(&http_get(port, "/api/file?path=tx.hex"));
    assert_eq!(hex_resp["ok"], true);
    assert_eq!(hex_resp["payload"]["Title"], "Ethereum Transaction");

    let json_resp = parse_json_body(&http_get(port, "/api/file?path=expected.json"));
    assert_eq!(json_resp["ok"], true);
    assert_eq!(json_resp["payload"]["sentinel"], "first");

    // 2. Mutate the JSON file on disk and verify the next GET reflects it
    //    without restarting the server.
    fs::write(work.join("expected.json"), r#"{"sentinel":"second"}"#).unwrap();
    let json_resp2 = parse_json_body(&http_get(port, "/api/file?path=expected.json"));
    assert_eq!(json_resp2["payload"]["sentinel"], "second");

    // 3. Drop in a brand-new file and verify it shows up in /.
    fs::write(
        work.join("late.json"),
        r#"{"created":"after-server-start"}"#,
    )
    .unwrap();
    let body = http_get(port, "/");
    assert!(
        body.contains("late.json"),
        "new file missing from /. got: {body}"
    );

    // 4. Truncate the hex file and verify it flips to error state.
    fs::write(work.join("tx.hex"), "").unwrap();
    let hex_resp2 = parse_json_body(&http_get(port, "/api/file?path=tx.hex"));
    assert_eq!(hex_resp2["ok"], false);

    // Cleanup.
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&work);
}

#[test]
#[cfg(feature = "solana")]
fn test_cli_solana_idl_invalid_file_still_parses() {
    let mapping = "Bad:/nonexistent/idl.json:11111111111111111111111111111111";

    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let input =
        fs::read_to_string(fixtures_dir.join("solana-json.input")).expect("read solana-json.input");

    let mut args: Vec<&str> = Vec::new();
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    for line in &lines {
        args.push(line);
    }
    args.push("--idl-json-mappings");
    args.push(mapping);

    let output = run_cli(&args);

    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Solana Transaction");
}
