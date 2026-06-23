//! Standalone TVC deploy helper for `parser_app`.
//!
//! Subcommands:
//!   gen-operator-key --out <path>
//!       Mint a qos_p256 operator key. Writes the 32-byte master seed (hex) to
//!       <path> with mode 0600 and prints ONLY the public key hex to stdout.
//!   deploy --app-id <id> --image-url <url> --expected-digest <hex>
//!          --operator-id <id> [--operator-seed <path>] [--qos-version v]
//!          [--host-ip 0.0.0.0] [--host-port 3000]
//!       Assemble tvc-deploy.json (gRPC health), create the deployment, assert
//!       the manifest pivot hash matches --expected-digest BEFORE approving,
//!       approve, set live, and poll until replicas are healthy.
//!
//! All Turnkey API actions shell out to the `tvc` CLI (it owns auth/consensus);
//! this binary owns config assembly, the pivot-digest safety gate, and polling.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qos_p256::P256Pair;

const POLL_TIMEOUT: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_secs(15);
const SETLIVE_TIMEOUT: Duration = Duration::from_secs(300);

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let cmd = argv.get(1).map(String::as_str).unwrap_or("");
    let flags = parse_flags(if argv.len() > 2 { &argv[2..] } else { &[] });
    let result = match cmd {
        "gen-operator-key" => gen_operator_key(&flags),
        "deploy" => deploy(&flags),
        _ => {
            eprintln!(
                "usage:\n  tvc-deploy gen-operator-key --out <path>\n  tvc-deploy deploy \
                 --app-id <id> --image-url <url> --expected-digest <hex> --operator-id <id> \
                 [--operator-seed <path>] [--qos-version v2026.2.6] [--host-ip 0.0.0.0] [--host-port 3000]\n\
                 (operator seed may instead come from env TVC_CI_OPERATOR_SEED)"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Minimal `--key value` flag parser. A flag with no following value (or
/// followed by another `--flag`) is recorded as "true".
fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            match args.get(i + 1) {
                Some(v) if !v.starts_with("--") => {
                    m.insert(key.to_string(), v.clone());
                    i += 2;
                }
                _ => {
                    m.insert(key.to_string(), "true".to_string());
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    m
}

fn req<'a>(flags: &'a HashMap<String, String>, key: &str) -> Result<&'a String, String> {
    flags.get(key).ok_or_else(|| format!("missing --{key}"))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn gen_operator_key(flags: &HashMap<String, String>) -> Result<(), String> {
    let out = req(flags, "out")?;
    let pair = P256Pair::generate().map_err(|e| format!("key generation failed: {e:?}"))?;
    let seed_hex = hex_encode(&pair.to_master_seed()[..]);
    let pub_hex = hex_encode(&pair.public_key().to_bytes());
    write_secret_file(Path::new(out), &seed_hex)?;
    // SECURITY: only the public key is ever printed; the seed stays in the file.
    println!("{pub_hex}");
    eprintln!("operator seed written to {out} (mode 0600); public key printed above");
    Ok(())
}

fn write_secret_file(path: &Path, contents: &str) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    f.write_all(contents.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn deploy(flags: &HashMap<String, String>) -> Result<(), String> {
    let app_id = req(flags, "app-id")?.clone();
    let image = req(flags, "image-url")?.clone();
    let digest = req(flags, "expected-digest")?.clone();
    let operator_id = req(flags, "operator-id")?.clone();
    let qos = flags
        .get("qos-version")
        .cloned()
        .unwrap_or_else(|| "v2026.2.6".to_string());
    let host_ip = flags
        .get("host-ip")
        .cloned()
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let host_port: u16 = match flags.get("host-port") {
        Some(s) => s.parse().map_err(|_| format!("invalid --host-port: {s}"))?,
        None => 3000,
    };

    validate_digest(&digest)?;
    let (seed_path, cleanup_seed) = resolve_seed_file(flags)?;

    // 1. Assemble the deployment config (gRPC health is mandatory for parser_app).
    let cfg = serde_json::json!({
        "appId": app_id,
        "qosVersion": qos,
        "pivotContainerImageUrl": image,
        "pivotPath": "/parser_app",
        "pivotArgs": ["--host-ip", host_ip, "--host-port", host_port.to_string()],
        "expectedPivotDigest": digest,
        "debugMode": false,
        "healthCheckType": "TVC_HEALTH_CHECK_TYPE_GRPC",
        "healthCheckPort": host_port,
        "publicIngressPort": host_port,
    });
    let cfg_path = temp_path("tvc-deploy", "json");
    std::fs::write(
        &cfg_path,
        serde_json::to_vec_pretty(&cfg).map_err(|e| format!("serialize config: {e}"))?,
    )
    .map_err(|e| format!("write {}: {e}", cfg_path.display()))?;

    // 2. Create the deployment.
    let create_out = run_capture(
        "tvc",
        &["deploy", "create", "--config-file", path(&cfg_path)],
    )?;
    let deploy_id = parse_after(&create_out, "Deployment ID:")
        .ok_or_else(|| format!("could not parse deployment id from:\n{create_out}"))?;
    println!("created deployment {deploy_id}");

    // 3. Safety gate: the manifest's pivot binary hash MUST match --expected-digest
    //    before we approve. Read it via a non-approving dry-run.
    let manifest_hash = read_manifest_pivot_hash(&deploy_id)?;
    if !manifest_hash.eq_ignore_ascii_case(&digest) {
        let _ = std::fs::remove_file(&cfg_path);
        return Err(format!(
            "DIGEST GATE FAILED: manifest pivot hash {manifest_hash} != expected {digest}; refusing to approve"
        ));
    }
    println!("digest gate passed: manifest pivot hash == {digest}");

    // 4. Approve with the operator key.
    run_status(
        "tvc",
        &[
            "deploy",
            "approve",
            "--deploy-id",
            &deploy_id,
            "--operator-id",
            &operator_id,
            "--operator-seed",
            path(&seed_path),
            "--dangerous-skip-interactive",
        ],
    )?;
    println!("approved manifest for {deploy_id}");

    // 5-6. Poll until the deployment is healthy, THEN set it live -- TVC refuses
    //      to target a deployment with zero healthy replicas. A fresh app
    //      auto-targets its first deploy on approval ("already" -> success).
    //      Clean up secrets regardless of outcome.
    let outcome = poll_health(&app_id, &deploy_id, POLL_TIMEOUT)
        .and_then(|_| set_live(&deploy_id, SETLIVE_TIMEOUT));
    if cleanup_seed {
        let _ = std::fs::remove_file(&seed_path);
    }
    let _ = std::fs::remove_file(&cfg_path);
    outcome?;
    println!("deployment {deploy_id} is healthy and live");
    Ok(())
}

/// Set the deployment live, retrying while TVC reports the deployment status is
/// still settling. A fresh app auto-targets its first deploy on approval, which
/// surfaces as an "already" error -- treat that as success.
fn set_live(deploy_id: &str, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        match run_capture("tvc", &["app", "set-live-deploy", "--deploy-id", deploy_id]) {
            Ok(_) => {
                println!("set {deploy_id} live");
                return Ok(());
            }
            Err(e) => {
                let m = e.to_lowercase();
                if m.contains("already") {
                    println!("{deploy_id} already live (auto-targeted)");
                    return Ok(());
                }
                let transient = m.contains("not yet available")
                    || m.contains("try again")
                    || m.contains("not found")
                    || m.contains("zero healthy replicas");
                if transient && start.elapsed() < timeout {
                    sleep(POLL_INTERVAL);
                    continue;
                }
                return Err(e);
            }
        }
    }
}

fn validate_digest(d: &str) -> Result<(), String> {
    if d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!(
            "--expected-digest must be 64 hex chars (sha256), got {:?}",
            d
        ))
    }
}

/// Resolve the operator seed to a file path. Prefers `--operator-seed <path>`;
/// otherwise reads the hex seed from env `TVC_CI_OPERATOR_SEED` and writes it to
/// a temp 0600 file (returning cleanup=true so the caller deletes it).
fn resolve_seed_file(flags: &HashMap<String, String>) -> Result<(PathBuf, bool), String> {
    if let Some(p) = flags.get("operator-seed") {
        return Ok((PathBuf::from(p), false));
    }
    let seed = std::env::var("TVC_CI_OPERATOR_SEED")
        .map_err(|_| "no --operator-seed and env TVC_CI_OPERATOR_SEED is unset".to_string())?;
    let p = temp_path("tvc-operator", "seed");
    write_secret_file(&p, seed.trim())?;
    Ok((p, true))
}

/// Run `tvc deploy approve --dry-run`, which fetches the manifest, prints it, and
/// exits WITHOUT approving. We feed `y` confirmations so the interactive walk
/// completes and prints the "Pivot Binary Hash" line, then parse it out.
fn read_manifest_pivot_hash(deploy_id: &str) -> Result<String, String> {
    let confirms = "y\n".repeat(16);
    let out = run_capture_with_input(
        "tvc",
        &["deploy", "approve", "--deploy-id", deploy_id, "--dry-run"],
        &confirms,
    )?;
    parse_after(&out, "Pivot Binary Hash:")
        .ok_or_else(|| format!("could not read manifest pivot hash from dry-run output:\n{out}"))
}

fn poll_health(app_id: &str, deploy_id: &str, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    let mut last = String::new();
    loop {
        // Status can 404 / fail transiently right after set-live while the app
        // registers; keep polling through errors and only give up at timeout.
        let out = match run_capture("tvc", &["app", "status", "--app-id", app_id]) {
            Ok(o) => o,
            Err(e) => {
                if start.elapsed() >= timeout {
                    return Err(format!("timed out; last status error: {e}"));
                }
                sleep(POLL_INTERVAL);
                continue;
            }
        };
        if let Some(ratio) = deployment_health(&out, deploy_id) {
            if ratio != last {
                println!("  {deploy_id}: {ratio}");
                last = ratio.clone();
            }
            if let Some((h, d)) = ratio.split_once('/') {
                if h == d && h != "0" {
                    return Ok(());
                }
            }
        }
        if start.elapsed() >= timeout {
            return Err(format!(
                "timed out after {}s waiting for {deploy_id} to be healthy (last: {})",
                timeout.as_secs(),
                if last.is_empty() { "unknown" } else { &last }
            ));
        }
        sleep(POLL_INTERVAL);
    }
}

/// From `tvc app status` output, find the `Healthy / Desired Replicas: X/Y`
/// line that belongs to `deploy_id`'s block.
fn deployment_health(status: &str, deploy_id: &str) -> Option<String> {
    let mut in_block = false;
    for line in status.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Deployment:") {
            in_block = rest.trim() == deploy_id;
        } else if in_block {
            if let Some(rest) = t.strip_prefix("Healthy / Desired Replicas:") {
                return rest.split_whitespace().next().map(|s| s.to_string());
            }
        }
    }
    None
}

/// Return the trimmed remainder of the first line containing `marker`.
fn parse_after(haystack: &str, marker: &str) -> Option<String> {
    haystack.lines().find_map(|line| {
        line.find(marker)
            .map(|i| line[i + marker.len()..].trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

fn temp_path(prefix: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}.{ext}", std::process::id()))
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap_or_default()
}

fn run_capture(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("spawn {cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd} {args:?} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run_capture_with_input(cmd: &str, args: &[&str], input: &str) -> Result<String, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {cmd}: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("no stdin handle")?
        .write_all(input.as_bytes())
        .map_err(|e| format!("write stdin: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait {cmd}: {e}"))?;
    // dry-run may exit non-zero depending on confirmations; we only need stdout,
    // so return it regardless and let the caller's parse succeed or fail.
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run_status(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("spawn {cmd}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} {args:?} failed ({status})"))
    }
}
