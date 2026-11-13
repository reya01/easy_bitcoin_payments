//! Live checks against REAL mainnet barkd wallets on Second's Ark server.
//! Every test here is `#[ignore]`: they only run from the `live-test`
//! workflow (`.github/workflows/live-mainnet-test.yml`), which starts the
//! wallets and passes their addresses in the environment. They exercise
//! EBP's own code (the barkd client, its parsers, the address checks and the
//! settlement engine) against real answers, not fakes.
//!
//! Environment: `LIVE_A_URL`/`LIVE_A_TOKEN` (wallet A), `LIVE_B_URL`/
//! `LIVE_B_TOKEN` (wallet B), `LIVE_DB` (EBP database file, kept between
//! steps), `LIVE_RETURN` (where the owner wants the money back),
//! `LIVE_WALLET` (A or B, for the per-wallet steps).

use std::sync::Arc;

use easy_bitcoin_payments::config::Config;
use easy_bitcoin_payments::db::Db;
use easy_bitcoin_payments::emergency_exit as ee;
use easy_bitcoin_payments::wallet::barkd::{
    is_mainnet_network, vtxo_expiry_unix, BarkdClient, BarkdConfig,
};
use easy_bitcoin_payments::wallet::settlement::SettlementEngine;

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set"))
}

fn wallet(which: &str) -> BarkdClient {
    BarkdClient::new(BarkdConfig::new(
        env(&format!("LIVE_{which}_URL")),
        env(&format!("LIVE_{which}_TOKEN")),
    ))
    .unwrap()
}

fn live_config(destination: &str) -> Arc<Config> {
    Arc::new(Config {
        mode: "live".into(),
        settlement_destination: Some(destination.into()),
        hot_wallet_cap_sats: 1_000,
        ..Config::default()
    })
}

async fn show(label: &str, c: &BarkdClient) -> i64 {
    let b = c.balances().await.expect("balance");
    let v = c.vtxos().await.expect("coins");
    let tip = c.bitcoin_tip().await.ok();
    println!(
        "[{label}] spendable {} sats, pending in round {}, claimable lightning {}, needs refresh {}",
        b.offchain_sats, b.pending_in_round_sats, b.claimable_lightning_sats, b.needs_refresh_sats
    );
    for x in v.spendable.iter().chain(v.pending.iter()) {
        let left = vtxo_expiry_unix(x, tip.as_ref())
            .map(|t| (t - easy_bitcoin_payments::util::now_unix()) / 3600);
        println!(
            "[{label}]   coin {}… {} sats, expires in about {:?} h",
            &x.id[..12.min(x.id.len())],
            x.amount_sats,
            left
        );
    }
    b.offchain_sats
}

/// Step 1, no money: every answer EBP reads from a real mainnet wallet
/// parses the way the code expects.
#[tokio::test]
#[ignore]
async fn unfunded_mainnet_wallet_answers_match_what_ebp_expects() {
    let a = wallet("A");

    let connected = a.connected().await.expect("connected");
    println!("connected: {connected:?}");
    assert_eq!(connected.connected, Some(true), "the Ark server answers");

    // Network (PR B's red line) and terms (PR D's renewal point).
    let net = a.ark_network().await.expect("ark-info").expect("network");
    println!("network: {net}");
    assert!(is_mainnet_network(&net), "mainnet reports {net}");
    let terms = a.ark_terms().await.expect("terms");
    println!("terms: {terms:?}");
    assert_eq!(terms.vtxo_lifetime_blocks, Some(4032));
    assert_eq!(terms.free_refresh_below_blocks, Some(288));
    assert!(terms.round_interval_secs.is_some(), "round_interval parsed");
    println!(
        "exit delta: {:?} blocks",
        a.exit_delta_blocks().await.expect("exit delta")
    );

    // Change 4's round list and PR G's exit list: both empty, both parse.
    let rounds = a.round_participations().await.expect("GET /wallet/rounds");
    println!("rounds: {rounds:?}");
    assert!(rounds.is_empty());
    let live = a.exits_live().await.expect("GET /exits/status/live");
    println!("exits live (raw): {live}");
    assert!(ee::parse_live(&live).is_empty());
    match a.exit_fee(None).await {
        Ok(v) => println!(
            "exit fee, empty wallet (raw): {v} -> parsed {:?}",
            ee::parse_fee(&v)
        ),
        Err(e) => println!("exit fee, empty wallet: error {e}"),
    }
    let onchain = a.onchain_balance_raw().await.expect("on-chain balance");
    println!("on-chain balance (raw): {onchain}");
    assert_eq!(ee::confirmed_onchain_sats(&onchain), Some(0));

    // Real mainnet addresses pass EBP's own checks (PR C), and only on a
    // real-money server.
    let ark = a.ark_address().await.expect("ark address");
    let chain = a.onchain_address().await.expect("on-chain address");
    println!("ark address: {ark}\non-chain address: {chain}");
    assert_eq!(
        easy_bitcoin_payments::payout::check(&ark, "live").unwrap(),
        ark
    );
    assert!(easy_bitcoin_payments::payout::check(&ark, "test").is_err());
    assert_eq!(
        easy_bitcoin_payments::payout::check(&chain, "live").unwrap(),
        chain
    );
    assert!(
        ee::check_destination(&ark, "live").is_err(),
        "exit never to Ark"
    );
    assert!(ee::check_destination(&chain, "live").is_ok());

    // A Lightning invoice and the combined payment link.
    let inv = a
        .ln_invoice(1_000, "EBP live check")
        .await
        .expect("invoice");
    println!("invoice: {}", inv.bolt11);
    assert!(inv.bolt11.to_lowercase().starts_with("lnbc"));
    let link =
        easy_bitcoin_payments::uri::combined(1_000, Some(&chain), Some(&inv.bolt11), Some(&ark));
    assert!(link.starts_with(&format!("bitcoin:{chain}?amount=0.00001&lightning=")));

    // Home on a live server with this wallet: no "wrong network" line.
    let facts = easy_bitcoin_payments::health_checks::Facts {
        live: true,
        wallet_network: Some(net),
        ..Default::default()
    };
    let checks = easy_bitcoin_payments::health_checks::build(&facts);
    assert!(checks.iter().all(|c| c.id != "network"));

    // The settlement engine on an empty live wallet: nothing to do, no error.
    let db = Db::open(":memory:").unwrap();
    let engine = SettlementEngine::new(a, db, live_config(&ark));
    let report = engine.tick().await.expect("tick");
    println!("engine tick, empty wallet: {report:?}");
    assert_eq!(report.refreshed_vtxos, 0);
    assert!(report.swept_sats.is_none());
}

/// Print both wallets (used between funded steps).
#[tokio::test]
#[ignore]
async fn show_both_wallets() {
    show("A", &wallet("A")).await;
    show("B", &wallet("B")).await;
    let r = wallet("A").round_participations().await;
    println!("[A] rounds: {r:?}");
    let r = wallet("B").round_participations().await;
    println!("[B] rounds: {r:?}");
}

/// Funded: a plain Ark transfer from A to B with EBP's client.
#[tokio::test]
#[ignore]
async fn ark_send_from_a_to_b() {
    let a = wallet("A");
    let b = wallet("B");
    let before_b = show("B", &b).await;
    let have = show("A", &a).await;
    let amount = have / 2;
    assert!(amount >= 330, "A has {have} sats; need at least 660");
    let dest = b.ark_address().await.unwrap();
    let receipt = a.ark_send(&dest, amount).await.expect("ark send");
    println!("ark send {amount} sats A -> B: {receipt}");
    for _ in 0..30 {
        if show("B", &b).await >= before_b + amount {
            println!("B received {amount} sats over Ark");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
    panic!("B did not receive the Ark transfer within 5 minutes");
}

/// Funded: one settlement cycle on wallet A, live mode, payout to B's Ark
/// address, keep 1,000. Run repeatedly by the workflow; the database file
/// keeps "requested" renewals between runs.
#[tokio::test]
#[ignore]
async fn engine_cycle_on_a_with_payout_to_b() {
    let a = wallet("A");
    let b = wallet("B");
    show("A", &a).await;
    let dest = b.ark_address().await.unwrap();
    let db = Db::open(&env("LIVE_DB")).unwrap();
    let engine = SettlementEngine::new(a, db.clone(), live_config(&dest));
    let report = engine.tick().await.expect("tick");
    println!("engine tick: {report:?}");
    let rows: Vec<(String, String, i64, String)> = db
        .call(|c| {
            let mut st = c.prepare(
                "SELECT kind, status, amount_sats, COALESCE(detail, '') FROM settlements ORDER BY rowid",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap();
    for r in rows {
        println!(
            "  row: {} {} {} sats  {}",
            r.0,
            r.1,
            r.2,
            &r.3[..r.3.len().min(160)]
        );
    }
    show("A", &wallet("A")).await;
    show("B", &b).await;
}

/// Funded: the fee estimate for an emergency withdrawal of A's coins, and
/// what EBP's rule says about it. Nothing is started.
#[tokio::test]
#[ignore]
async fn emergency_withdrawal_estimate_on_a_funded_wallet() {
    let a = wallet("A");
    let have = show("A", &a).await;
    let chain = a.onchain_address().await.unwrap();
    match a.exit_fee(Some(&chain)).await {
        Ok(v) => {
            let fee = ee::parse_fee(&v);
            println!("exit fee (raw): {v}\nparsed: {fee:?}");
            println!("EBP rule: {:?}", ee::refusal(have, fee.as_ref()));
            assert!(
                ee::refusal(have, fee.as_ref()).is_some(),
                "small wallets are refused"
            );
        }
        Err(e) => println!("exit fee: error {e}"),
    }
}

/// Always last: "Move my money now" (EBP's manual sweep) from one wallet to
/// the owner's return address.
#[tokio::test]
#[ignore]
async fn move_my_money_now_to_the_owner() {
    let which = env("LIVE_WALLET");
    let c = wallet(&which);
    let have = show(&which, &c).await;
    if have == 0 {
        println!("[{which}] nothing to return");
        return;
    }
    let dest = env("LIVE_RETURN");
    // The same database as the engine steps (A) or its own (B), so a renewal
    // already requested is remembered between retries, never asked twice.
    let path = if which == "A" {
        env("LIVE_DB")
    } else {
        format!("{}-{which}", env("LIVE_DB"))
    };
    let db = Db::open(&path).unwrap();
    let engine = SettlementEngine::new(c, db, live_config(dest.trim()));
    let report = engine.sweep_now().await.expect("sweep");
    println!("[{which}] move my money now: {report:?}");
    show(&which, &wallet(&which)).await;
}
