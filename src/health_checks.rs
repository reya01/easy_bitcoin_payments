//! The merchant-facing "is everything OK?" checklist on the admin Home tab.
//!
//! Pure logic: [`build`] turns a few facts (collected by the status endpoint)
//! into plain-language lines with a level. No I/O here, so every rule is unit
//! tested. Wording is for a shop owner, not an operator: what is wrong, why
//! it matters, and the one thing to do. Never includes secrets.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Bad,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub level: Level,
    pub title: String,
    pub detail: String,
}

/// Facts the checklist is built from. `None` means "could not find out".
#[derive(Debug, Default, Clone)]
pub struct Facts {
    /// The public address starts with https://.
    pub https: bool,
    /// The public address is localhost (only reachable from this machine).
    pub local_only: bool,
    /// The wallet service answered at all.
    pub wallet_reachable: bool,
    /// The wallet is connected to its payment server (ASP).
    pub asp_connected: Option<bool>,
    /// Emergency-exit reserve is funded.
    pub exit_reserve_ok: Option<bool>,
    pub exit_reserve_shortfall_sats: i64,
    /// Coins the wallet says need renewing (`needs_refresh_sat`).
    pub needs_refresh_sats: Option<i64>,
    /// Offline-spendable balance and the configured safety cap.
    pub offchain_sats: Option<i64>,
    pub hot_wallet_cap_sats: i64,
    pub unmatched_open: i64,
    pub webhooks_failed: i64,
    pub webhook_configured: bool,
    pub last_settlement_failed: bool,
}

pub fn build(f: &Facts) -> Vec<Check> {
    let mut out = Vec::new();
    let mut push = |id, level, title: &str, detail: &str| {
        out.push(Check {
            id,
            level,
            title: title.to_string(),
            detail: detail.to_string(),
        });
    };

    if !f.wallet_reachable {
        push(
            "wallet",
            Level::Bad,
            "Your wallet is not answering",
            "New payments cannot be received right now. Restart the service; if it keeps happening, check your host's logs.",
        );
    } else {
        match f.asp_connected {
            Some(true) => push(
                "wallet",
                Level::Ok,
                "Wallet is connected",
                "Your wallet is online and can receive payments.",
            ),
            Some(false) => push(
                "wallet",
                Level::Bad,
                "Wallet cannot reach the payment network",
                "Payments will not be seen until it reconnects. This is usually temporary; check again in a few minutes.",
            ),
            None => push(
                "wallet",
                Level::Warn,
                "Wallet connection is unclear",
                "The wallet answered but did not say whether it is connected. Check again shortly.",
            ),
        }
    }

    if f.local_only {
        push(
            "https",
            Level::Warn,
            "Your address only works on this computer",
            "Customers cannot reach it. Before taking real payments, use a public https:// address.",
        );
    } else if f.https {
        push(
            "https",
            Level::Ok,
            "Your address is secure (HTTPS)",
            "Customers and you reach the shop over an encrypted connection.",
        );
    } else {
        push(
            "https",
            Level::Bad,
            "Your address is not secure (no HTTPS)",
            "Passwords and payment details could be read by others on the network. Put the shop behind an https:// address before real use.",
        );
    }

    if f.unmatched_open > 0 {
        push(
            "late",
            Level::Warn,
            &format!("{} payment(s) need your attention", f.unmatched_open),
            "Money arrived that did not match an open order (for example it came after the order expired). Open Alerts to see and resolve each one.",
        );
    }

    if let Some(ok) = f.exit_reserve_ok {
        if ok {
            push(
                "reserve",
                Level::Ok,
                "Safety reserve is funded",
                "You hold enough on-chain bitcoin to rescue your funds if the payment network ever stops cooperating.",
            );
        } else {
            push(
                "reserve",
                Level::Warn,
                "Safety reserve is low",
                &format!(
                    "Add about {} sats of regular on-chain bitcoin to the wallet so you could always withdraw on your own. Automatic withdrawals are paused until then.",
                    f.exit_reserve_shortfall_sats.max(0)
                ),
            );
        }
    }

    match f.needs_refresh_sats {
        Some(n) if n > 0 => push(
            "expiry",
            Level::Warn,
            "Some coins are due for renewal",
            &format!(
                "{n} sats are close to expiring. The server renews them automatically; if this stays here for more than an hour, check that the wallet is connected."
            ),
        ),
        Some(_) => push(
            "expiry",
            Level::Ok,
            "No coins are close to expiring",
            "Renewals are up to date.",
        ),
        None => {}
    }

    if f.webhooks_failed > 0 {
        push(
            "webhooks",
            Level::Warn,
            &format!("{} shop notification(s) failed", f.webhooks_failed),
            "Your shop was not told about some payments. Check the shop address on the Connect tab and send a test.",
        );
    } else if !f.webhook_configured {
        push(
            "webhooks",
            Level::Warn,
            "Your shop is not connected yet",
            "Orders will not be marked paid automatically. Open the Connect tab to finish.",
        );
    } else {
        push(
            "webhooks",
            Level::Ok,
            "Your shop is receiving payment notices",
            "No failed notifications.",
        );
    }

    if f.last_settlement_failed {
        push(
            "withdrawals",
            Level::Warn,
            "The last automatic withdrawal did not complete",
            "Your funds are still in the wallet. It will try again; if it keeps failing, check your withdrawal address.",
        );
    }

    if let Some(offchain) = f.offchain_sats {
        if f.hot_wallet_cap_sats > 0 && offchain > f.hot_wallet_cap_sats.saturating_mul(2) {
            push(
                "balance",
                Level::Warn,
                "A large balance is sitting in the shop wallet",
                "Move some to your own wallet. Set a withdrawal address in Settings so this happens automatically.",
            );
        }
    }

    // Worst problems first; stable within a level.
    out.sort_by_key(|c| match c.level {
        Level::Bad => 0,
        Level::Warn => 1,
        Level::Ok => 2,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> Facts {
        Facts {
            https: true,
            wallet_reachable: true,
            asp_connected: Some(true),
            exit_reserve_ok: Some(true),
            needs_refresh_sats: Some(0),
            offchain_sats: Some(1_000),
            hot_wallet_cap_sats: 500_000,
            webhook_configured: true,
            ..Facts::default()
        }
    }

    fn find<'a>(c: &'a [Check], id: &str) -> &'a Check {
        c.iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    #[test]
    fn a_localhost_address_is_a_warning_not_an_alarm() {
        let mut f = healthy();
        f.https = false;
        f.local_only = true;
        let c = build(&f);
        assert_eq!(find(&c, "https").level, Level::Warn);
        assert!(find(&c, "https")
            .title
            .contains("only works on this computer"));
    }

    #[test]
    fn a_healthy_shop_is_all_green() {
        let c = build(&healthy());
        assert!(c.iter().all(|x| x.level == Level::Ok), "{c:?}");
    }

    #[test]
    fn unreachable_wallet_is_bad_and_listed_first() {
        let mut f = healthy();
        f.wallet_reachable = false;
        f.https = false;
        let c = build(&f);
        assert_eq!(c[0].level, Level::Bad);
        assert_eq!(find(&c, "wallet").level, Level::Bad);
        assert_eq!(find(&c, "https").level, Level::Bad);
    }

    #[test]
    fn disconnected_from_the_payment_network_is_bad() {
        let mut f = healthy();
        f.asp_connected = Some(false);
        assert_eq!(find(&build(&f), "wallet").level, Level::Bad);
        f.asp_connected = None;
        assert_eq!(find(&build(&f), "wallet").level, Level::Warn);
    }

    #[test]
    fn low_reserve_expiry_and_late_payments_warn_with_numbers() {
        let mut f = healthy();
        f.exit_reserve_ok = Some(false);
        f.exit_reserve_shortfall_sats = 12_345;
        f.needs_refresh_sats = Some(777);
        f.unmatched_open = 2;
        let c = build(&f);
        assert!(find(&c, "reserve").detail.contains("12345"));
        assert!(find(&c, "expiry").detail.contains("777"));
        assert!(find(&c, "late").title.starts_with('2'));
        assert!(["reserve", "expiry", "late"]
            .iter()
            .all(|id| find(&c, id).level == Level::Warn));
    }

    #[test]
    fn shop_connection_states() {
        let mut f = healthy();
        f.webhook_configured = false;
        assert_eq!(find(&build(&f), "webhooks").level, Level::Warn);
        f.webhook_configured = true;
        f.webhooks_failed = 3;
        let c = build(&f);
        assert_eq!(find(&c, "webhooks").level, Level::Warn);
        assert!(find(&c, "webhooks").title.starts_with('3'));
    }

    #[test]
    fn unknown_facts_do_not_invent_problems() {
        let mut f = healthy();
        f.exit_reserve_ok = None;
        f.needs_refresh_sats = None;
        f.offchain_sats = None;
        let c = build(&f);
        assert!(c.iter().all(|x| x.id != "reserve" && x.id != "expiry"));
        assert!(c.iter().all(|x| x.level == Level::Ok));
    }

    #[test]
    fn a_big_balance_and_a_failed_withdrawal_warn() {
        let mut f = healthy();
        f.offchain_sats = Some(1_200_000);
        f.last_settlement_failed = true;
        let c = build(&f);
        assert_eq!(find(&c, "balance").level, Level::Warn);
        assert_eq!(find(&c, "withdrawals").level, Level::Warn);
    }
}
