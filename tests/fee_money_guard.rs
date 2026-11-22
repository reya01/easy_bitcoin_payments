//! Guard: the on-chain "fee money" is only ever spent by the owner's own button.
//!
//! The server keeps a little regular Bitcoin to pay the fees of an emergency
//! withdrawal. Nothing that runs by itself (the transfer to the payout address, coin
//! renewals, the watcher, health checks, alerts) may ever spend it. The calls that can
//! spend it are the wallet functions below. This test reads the source and fails the
//! build if any of them is called from anywhere except the emergency-withdrawal page's
//! own handlers (src/routes/exit.rs), which sit behind the password brake and the
//! second step.
//!
//! How: for each function, every `.name(` outside its definition file must be in the
//! allowed file.

use std::fs;
use std::path::{Path, PathBuf};

/// Wallet calls that can spend or move the on-chain fee money.
const SPENDERS: &[&str] = &[
    "onchain_send",
    "onchain_drain",
    "start_exit_all",
    "claim_exits",
    "cancel_exit",
];

/// The only file that may call them: the owner's emergency-withdrawal buttons.
const ALLOWED: &str = "src/routes/exit.rs";
/// Where they are defined (and tested against a fake wallet).
const DEFINED_IN: &str = "src/wallet/barkd.rs";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn only_the_emergency_withdrawal_buttons_can_spend_the_fee_money() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(files.len() > 20, "the source files were not found");

    let mut found_in_allowed = 0;
    let mut stray = Vec::new();
    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == DEFINED_IN {
            continue;
        }
        let text = fs::read_to_string(f).unwrap();
        for name in SPENDERS {
            let call = format!(".{name}(");
            if text.contains(&call) {
                if rel == ALLOWED {
                    found_in_allowed += 1;
                } else {
                    stray.push(format!("{rel} calls {name}"));
                }
            }
        }
    }
    assert!(
        stray.is_empty(),
        "these can spend the on-chain fee money and must only be called from {ALLOWED}: {stray:?}"
    );
    // The scan itself works: the buttons really are where we expect.
    assert!(
        found_in_allowed >= 3,
        "expected the emergency-withdrawal buttons in {ALLOWED}, found {found_in_allowed} calls"
    );
}

#[test]
fn the_list_of_spenders_matches_the_wallet_file() {
    // If someone adds another wallet call that sends on-chain money, it must be added
    // to SPENDERS (and kept out of automatic code). Every `/onchain/` or `/exits/`
    // path other than the read-only ones must belong to a function listed above.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join(DEFINED_IN)).unwrap();
    let read_only = [
        "PATH_ONCHAIN_ADDRESS_NEXT",
        "PATH_ONCHAIN_BALANCE",
        "PATH_ONCHAIN_SYNC",
        "PATH_EXITS_FEE",
        "PATH_EXITS_LIVE",
    ];
    let mut unknown = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("const PATH_") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let name = format!("PATH_{name}");
        let is_onchain_or_exit = line.contains("\"/onchain/") || line.contains("\"/exits/");
        if is_onchain_or_exit && !read_only.contains(&name.as_str()) {
            let owner_fn = match name.as_str() {
                "PATH_ONCHAIN_SEND" => "onchain_send",
                "PATH_ONCHAIN_DRAIN" => "onchain_drain",
                "PATH_EXITS_START_ALL" => "start_exit_all",
                "PATH_EXITS_CLAIM_ALL" => "claim_exits",
                "PATH_EXITS_CANCEL" => "cancel_exit",
                _ => "",
            };
            if !SPENDERS.contains(&owner_fn) {
                unknown.push(name);
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "new on-chain or exit wallet paths that are not in SPENDERS (decide if they can spend fee money): {unknown:?}"
    );
}
