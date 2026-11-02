//! `parser_cli bitcoin wallet-check`: render a PSBT with ownership checked
//! against the signer's own watch-only Bitcoin Core wallet (prototype).
//!
//! Meant for an offline machine: the node holds only public descriptors and
//! has networking off. It is the trust anchor, so there is no backend and no
//! signature, and every row it contributes says "your local node".

use crate::bitcoind_rpc::{BitcoindRpc, NodeInfo, RpcAuth, default_port};
use crate::wallet_source::{WalletCheck, build_wallet_check};
use crate::{
    BitcoinMetadata, BitcoinTransactionWrapper, BitcoinVisualSignConverter, Classified, LocalNode,
    MetadataSource, NodeCheck, Ownership, OwnershipReport,
};
use std::path::{Path, PathBuf};

use bitcoin::psbt::Psbt;
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use parser_cli_core::output::HumanReadableFormatter;
use serde_json::{Value, json};
use visualsign::SignablePayload;
use visualsign::vsptrait::VisualSignOptions;

/// Bitcoin-only subcommands (`parser_cli bitcoin ...`).
#[derive(Subcommand, Debug)]
pub enum BitcoinCommand {
    /// Check a PSBT against your own watch-only Bitcoin Core wallet, then render it.
    WalletCheck(WalletCheckArgs),
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum WalletCheckOutput {
    Text,
    Json,
}

/// Flags for `parser_cli bitcoin wallet-check`.
#[derive(ClapArgs, Debug)]
pub struct WalletCheckArgs {
    /// PSBT, base64 or hex. Prefix with '@' to read from a file
    /// (e.g. '@/path/to/tx.psbt'), or use '@-' to read from stdin.
    #[arg(long, value_name = "PSBT")]
    pub psbt: String,

    /// Bitcoin Core RPC URL, http only. Default: http://127.0.0.1 on the
    /// network's RPC port.
    #[arg(long)]
    pub rpc_url: Option<String>,

    /// The node's RPC cookie file, e.g. ~/.bitcoin/regtest/.cookie.
    #[arg(
        long,
        conflicts_with = "rpc_user",
        required_unless_present = "rpc_user"
    )]
    pub rpc_cookie: Option<PathBuf>,

    /// RPC user name. The password is read from --rpc-password-env.
    #[arg(long, requires = "rpc_password_env")]
    pub rpc_user: Option<String>,

    /// Environment variable holding the RPC password.
    #[arg(long, requires = "rpc_user")]
    pub rpc_password_env: Option<String>,

    /// Wallet to read, ideally watch-only. Default: the node's only loaded wallet.
    #[arg(long)]
    pub wallet: Option<String>,

    /// bitcoin, testnet, testnet4, signet or regtest. The node must be on it.
    #[arg(long, default_value = "bitcoin")]
    pub network: String,

    /// Name shown for the wallet's accounts. Default: the wallet name.
    #[arg(long)]
    pub account_name: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value = "text")]
    pub output: WalletCheckOutput,

    /// Wallet metadata JSON (e.g. what a backend sent) to compare with the
    /// node, per input and output.
    #[arg(long, value_name = "METADATA_JSON")]
    pub compare: Option<PathBuf>,
}

impl BitcoinCommand {
    /// Run the subcommand, printing to stdout.
    pub fn run(&self) -> Result<(), String> {
        match self {
            Self::WalletCheck(args) => run_wallet_check(args),
        }
    }
}

/// One input or output: does the supplied metadata say it is yours, and does
/// the node?
struct CompareRow {
    item: String,
    supplied: (bool, String),
    node: (bool, String),
}

struct Comparison {
    file: String,
    /// `Err` when the supplied metadata does not fit the PSBT.
    rows: Result<Vec<CompareRow>, String>,
}

fn run_wallet_check(args: &WalletCheckArgs) -> Result<(), String> {
    let network = crate::parse_network(&args.network)?;
    let raw = parser_cli_core::tx_input::resolve_transaction_input(&args.psbt)?;
    let psbt = crate::decode_psbt(&raw).map_err(|e| e.to_string())?;
    let auth = match (&args.rpc_cookie, &args.rpc_user, &args.rpc_password_env) {
        (Some(cookie), _, _) => RpcAuth::Cookie(cookie.clone()),
        (None, Some(user), Some(var)) => RpcAuth::UserPassword {
            user: user.clone(),
            password: std::env::var(var)
                .map_err(|_| format!("environment variable {var} is not set"))?,
        },
        _ => return Err("pass --rpc-cookie, or --rpc-user with --rpc-password-env".to_string()),
    };
    let url = args
        .rpc_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", default_port(network)));
    let node = BitcoindRpc::new(&url, args.wallet.as_deref(), &auth, network)?
        .with_account_name(args.account_name.clone())
        .connect()
        .map_err(|e| e.to_string())?;
    let check = build_wallet_check(&psbt, &node).map_err(|e| e.to_string())?;

    let converter = BitcoinVisualSignConverter::new(network);
    let report = converter
        .ownership_report_from(&psbt, &check.metadata, &check.source)
        .map_err(|e| e.to_string())?;
    let payload = converter
        .to_visual_sign_payload_with_metadata_from(
            BitcoinTransactionWrapper::new(psbt.clone()),
            VisualSignOptions::default(),
            &check.metadata,
            &check.source,
        )
        .map_err(|e| e.to_string())?
        .payload;
    let comparison = args
        .compare
        .as_deref()
        .map(|path| compare(&converter, &psbt, path, &report))
        .transpose()?;
    let info = node.info();

    match args.output {
        WalletCheckOutput::Text => {
            print!("{}", node_header(&check, info));
            println!("{}", HumanReadableFormatter::new(&payload, false));
            if let Some(c) = &comparison {
                print!("{}", comparison_text(c));
            }
        }
        WalletCheckOutput::Json => {
            println!(
                "{}",
                json_output(&check, info, &payload, comparison.as_ref())?
            );
        }
    }
    Ok(())
}

fn local_node(check: &WalletCheck) -> Option<&LocalNode> {
    match &check.source {
        MetadataSource::LocalNode(node) => Some(node),
        MetadataSource::Request => None,
    }
}

fn ismine_text(answers: &[Option<bool>], what: &str) -> Vec<String> {
    answers
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let a = match a {
                Some(true) => "yes",
                Some(false) => "no",
                None => "n/a",
            };
            format!("{what} {i} {a}")
        })
        .collect()
}

/// What was read from the node, before the rendering.
fn node_header(check: &WalletCheck, info: Option<&NodeInfo>) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut out = String::new();
    if let Some(node) = local_node(check) {
        out.push_str(&format!("Local node: {}\n", node.detail));
    }
    if let Some(info) = info {
        let keys = if info.private_keys_enabled {
            "WARNING: this wallet holds private keys; use a watch-only wallet"
        } else {
            "watch-only (private keys disabled)"
        };
        out.push_str(&format!(
            "  Node:        {} chain, {} block{}, wallet {}: {keys}\n",
            info.chain,
            info.blocks,
            plural(usize::try_from(info.blocks).unwrap_or(usize::MAX)),
            info.wallet_name
        ));
    }
    let unsupported: Vec<String> = check
        .unsupported
        .iter()
        .map(|(reason, n)| format!("{n} descriptor{} not supported: {reason}", plural(*n)))
        .collect();
    out.push_str(&format!(
        "  Descriptors: {} read, {} account{}; {}\n",
        check.entries_read,
        check.accounts_available,
        plural(check.accounts_available),
        if unsupported.is_empty() {
            "none unsupported".to_string()
        } else {
            unsupported.join("; ")
        }
    ));
    let utxos = &check.metadata.utxos;
    let prevouts = utxos.iter().filter(|u| u.prev_tx.is_some()).count();
    let labels =
        check.metadata.output_labels.len() + utxos.iter().filter(|u| u.label.is_some()).count();
    out.push_str(&format!(
        "  This PSBT:   names {} of them; {prevouts} prevout{} and {labels} label{} from the node\n",
        check.metadata.accounts.len(),
        plural(prevouts),
        plural(labels),
    ));
    if let Some(node) = local_node(check) {
        let mut answers = ismine_text(&node.inputs_ismine, "input");
        answers.extend(ismine_text(&node.outputs_ismine, "output"));
        out.push_str(&format!("  ismine:      {}\n", answers.join(", ")));
    }
    out.push('\n');
    out
}

/// Whether a row is verified as yours, and why, as one side of a comparison.
fn verdict(c: &Classified) -> (bool, String) {
    match (&c.ownership, c.node) {
        (_, NodeCheck::Disagrees { ismine }) => (
            false,
            format!("not verified (ismine={ismine} contradicts the PSBT)"),
        ),
        (Ownership::OwnVerified { account, path }, _) => {
            (true, format!("yours ({account} m/{path})"))
        }
        (Ownership::ClaimedOwnUnverified { .. }, _) => {
            (false, "not yours (false ownership claim)".to_string())
        }
        (Ownership::External, _) => (false, "not yours".to_string()),
    }
}

fn compare(
    converter: &BitcoinVisualSignConverter,
    psbt: &Psbt,
    path: &Path,
    node_report: &OwnershipReport,
) -> Result<Comparison, String> {
    let file = path.display().to_string();
    let json = std::fs::read_to_string(path).map_err(|e| format!("cannot read {file}: {e}"))?;
    let rows = BitcoinMetadata::from_json(&json)
        .and_then(|supplied| {
            converter
                .ownership_report(psbt, &supplied)
                .map_err(|e| e.to_string())
        })
        .map(|supplied| {
            let inputs = supplied
                .inputs
                .iter()
                .zip(&node_report.inputs)
                .enumerate()
                .map(|(i, (s, n))| CompareRow {
                    item: format!("Input {i}"),
                    supplied: verdict(s),
                    node: verdict(n),
                });
            let outputs = supplied
                .outputs
                .iter()
                .zip(&node_report.outputs)
                .enumerate()
                .map(|(i, (s, n))| CompareRow {
                    item: format!("Output {i}"),
                    supplied: verdict(s),
                    node: verdict(n),
                });
            inputs.chain(outputs).collect()
        });
    Ok(Comparison { file, rows })
}

fn comparison_text(c: &Comparison) -> String {
    let mut out = format!("\nCompare: {} vs your local node\n", c.file);
    match &c.rows {
        Err(e) => out.push_str(&format!("  supplied metadata rejected: {e}\n")),
        Ok(rows) => {
            for row in rows {
                let agreement = if row.supplied.0 == row.node.0 {
                    "agree"
                } else {
                    "DISAGREE"
                };
                out.push_str(&format!(
                    "  {:<9} {agreement:<8} supplied: {} | node: {}\n",
                    row.item, row.supplied.1, row.node.1
                ));
            }
        }
    }
    out
}

fn json_output(
    check: &WalletCheck,
    info: Option<&NodeInfo>,
    payload: &SignablePayload,
    comparison: Option<&Comparison>,
) -> Result<String, String> {
    let parse = |text: &str| -> Result<Value, String> {
        serde_json::from_str(text).map_err(|e| format!("internal JSON error: {e}"))
    };
    let node = local_node(check);
    let out = json!({
        "node": {
            "description": node.map(|n| n.detail.clone()),
            "wallet": info.map(|i| i.wallet_name.clone()),
            "watch_only": info.map(|i| !i.private_keys_enabled),
            "chain": info.map(|i| i.chain.clone()),
            "blocks": info.map(|i| i.blocks),
            "descriptors_read": check.entries_read,
            "accounts_available": check.accounts_available,
            "accounts_used": check.metadata.accounts.len(),
            "unsupported": check.unsupported,
            "inputs_ismine": node.map(|n| n.inputs_ismine.clone()),
            "outputs_ismine": node.map(|n| n.outputs_ismine.clone()),
        },
        "metadata": parse(&check.metadata.to_json())?,
        "payload": parse(&payload.to_json().map_err(|e| e.to_string())?)?,
        "compare": comparison.map(|c| match &c.rows {
            Err(e) => json!({"file": c.file, "error": e}),
            Ok(rows) => json!({
                "file": c.file,
                "rows": rows.iter().map(|r| json!({
                    "item": r.item,
                    "supplied_yours": r.supplied.0,
                    "supplied": r.supplied.1,
                    "node_yours": r.node.0,
                    "node": r.node.1,
                    "agree": r.supplied.0 == r.node.0,
                })).collect::<Vec<_>>(),
            }),
        }),
    });
    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
}
