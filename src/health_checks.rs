//! The merchant-facing "is everything OK?" checklist on the admin Home tab.
//!
//! Pure logic: [`build`] turns a few facts (collected by the status endpoint)
//! into plain-language lines with a level. No I/O here, so every rule is unit
//! tested. Wording is for a shop owner, not an operator: what is wrong, why
//! it matters, and the one thing to do. Never includes secrets.

use serde::Serialize;

/// Where the till empties to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Payout {
    /// Could not find out (no check shown).
    #[default]
    Unknown,
    NotSet,
    Set,
    /// The saved address fails the address checks (typo, wrong network):
    /// sweeps are refused until it is fixed.
    Invalid,
    /// A change is waiting; takes effect at this unix time.
    ChangePending(i64),
}

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
    /// Dollars per bitcoin, to write "($0.40)" next to sats; unknown or off = None.
    pub usd_per_btc: Option<f64>,
    /// Seconds since the Bitcoin price was last refreshed; `None` = never yet.
    pub price_age_secs: Option<i64>,
    /// The public address starts with https://.
    pub https: bool,
    /// The public address is localhost (only reachable from this machine).
    pub local_only: bool,
    /// The wallet service answered at all.
    pub wallet_reachable: bool,
    /// The wallet is connected to its payment server (ASP).
    pub asp_connected: Option<bool>,
    /// Coins the wallet says need renewing (`needs_refresh_sat`).
    pub needs_refresh_sats: Option<i64>,
    /// Offline-spendable balance and the configured safety cap.
    pub offchain_sats: Option<i64>,
    pub hot_wallet_cap_sats: i64,
    pub unmatched_open: i64,
    pub webhooks_failed: i64,
    pub webhook_configured: bool,
    pub last_settlement_failed: bool,
    /// How many of the most recent coin renewals in a row failed.
    pub renewals_failing: u32,
    pub payout: Payout,
    /// This server is in live (real money) mode.
    pub live: bool,
    /// Sats on the server too small to renew (under the wallet's minimum),
    /// and how long until they lapse; `None` = nothing too small.
    pub too_small_to_renew_sats: Option<i64>,
    pub too_small_lapse_in_secs: Option<i64>,
    /// The network the wallet's Ark server reports (`"bitcoin"` for
    /// mainnet); `None` = could not find out (no check shown).
    pub wallet_network: Option<String>,
}

/// The "large balance" warning never fires below this many sats, whatever the
/// amount the server keeps is set to.
const LARGE_BALANCE_FLOOR_SATS: i64 = 100_000;

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
            "New payments cannot be received right now. Wait a minute and refresh. If it stays red, restart your server from your hosting dashboard, then check again.",
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

    // A wallet belongs to one network for good. A server switched to the
    // other mode after setup (EBP_MODE changed in .env) would show prices
    // and addresses for one network while the wallet is on the other.
    if let Some(net) = f.wallet_network.as_deref() {
        let wallet_is_real = crate::wallet::barkd::is_mainnet_network(net);
        if f.live && !wallet_is_real {
            push(
                "network",
                Level::Bad,
                "This server is set to real money, but its wallet is play money",
                "The wallet was made for a test network, so real payments cannot work here. A wallet cannot be moved to real Bitcoin. Set EBP_MODE back to test in the .env file and restart, or make a new server for real money.",
            );
        } else if !f.live && wallet_is_real {
            push(
                "network",
                Level::Bad,
                "This server is set to play money, but its wallet holds REAL bitcoin",
                "The wallet was made for real Bitcoin (mainnet). Set EBP_MODE back to live in the .env file and restart. Do not take payments until this line is gone.",
            );
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

    match f.payout {
        Payout::Unknown => {}
        Payout::NotSet => push(
            "payout",
            Level::Warn,
            "No payout address yet",
            "Payments stay in the small till wallet and are not moved to your own wallet. Add your wallet's address in Settings.",
        ),
        Payout::Set => push(
            "payout",
            Level::Ok,
            "Payout address is set",
            "Money above the safety cap is sent to your own wallet.",
        ),
        Payout::Invalid => push(
            "payout",
            Level::Bad,
            "Your payout address is not valid",
            "Transfers to your own wallet are stopped until this is fixed (it may have a typo or belong to the wrong network). Open Settings and enter a valid address for this server.",
        ),
        Payout::ChangePending(_) => push(
            "payout",
            Level::Warn,
            "A payout address change is waiting",
            "Your payout address was changed in Settings and takes effect after a safety delay. If you did not do this, open Settings and cancel it, then change your admin password.",
        ),
    }

    match f.price_age_secs {
        None => push(
            "price",
            Level::Warn,
            "Bitcoin price not available yet",
            "The server looks up the Bitcoin price every few minutes. Until it has one, dollar amounts are hidden and payment requests priced in a currency (instead of sats) cannot be created. If this stays here for more than a few minutes, check that the server can reach the internet.",
        ),
        Some(age) if age > crate::rates::MAX_AGE_SECS => push(
            "price",
            Level::Bad,
            "Bitcoin price is more than a day old",
            "The server could not refresh the price for over a day, so it stopped using it: dollar amounts are hidden and payment requests priced in a currency (instead of sats) are refused. Check that the server can reach the internet.",
        ),
        Some(age) if age > crate::rates::OLD_AFTER_SECS => push(
            "price",
            Level::Warn,
            "Bitcoin price is out of date",
            "The server could not refresh the price for over an hour. It keeps using the last price it had and tries again every ten minutes. If this stays for a day, dollar amounts disappear and payment requests priced in a currency are refused.",
        ),
        Some(_) => {}
    }

    if f.unmatched_open > 0 {
        push(
            "late",
            Level::Warn,
            &format!("{} payment(s) need your attention", f.unmatched_open),
            "Money arrived that did not match an open order (for example it came after the order expired). Open Alerts to see and resolve each one.",
        );
    }

    match f.needs_refresh_sats {
        Some(n) if n > 0 => push(
            "expiry",
            Level::Warn,
            "Some coins are due for renewal",
            &format!(
                "{} are close to expiring. The server renews them automatically; if this stays here for more than an hour, check that the wallet is connected.",
                crate::rates::sats_with_usd(n, f.usd_per_btc)
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

    if f.renewals_failing >= 3 {
        // Coins due for renewal AND renewals failing is the dangerous
        // combination: coins that expire can only be recovered the slow way.
        let due = f.needs_refresh_sats.unwrap_or(0) > 0;
        push(
            "renewals",
            if due { Level::Bad } else { Level::Warn },
            "Renewing your coins is not working",
            &format!(
                "The last {} attempts failed. Your money is safe for now, but coins that are never renewed eventually expire. Check that the wallet is connected (see the top of this page); if this stays here, see the guide \"If something looks wrong\".",
                f.renewals_failing
            ),
        );
    }

    if let Some(sats) = f.too_small_to_renew_sats.filter(|s| *s > 0) {
        let when = match f.too_small_lapse_in_secs {
            Some(s) if s <= 0 => "now".to_string(),
            Some(s) if s < 2 * 3600 => "within the next couple of hours".to_string(),
            Some(s) if s < 2 * 86_400 => format!("in about {} hours", s / 3600),
            Some(s) => format!("in about {} days", s / 86_400),
            None => "when they expire".to_string(),
        };
        push(
            "renewal_small",
            Level::Warn,
            &format!("{sats} sats on the server are too small to renew"),
            &format!(
                "The Ark server cannot renew less than {} sats, so they will lapse {when} unless more payments arrive. Nothing to do: the amount is tiny, and new payments are not affected.",
                crate::wallet::settlement::MIN_RENEW_SATS
            ),
        );
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
        // Both tests must pass: more than twice the amount the server keeps, AND
        // a real sum. With a small kept amount (1,000 sats) "twice" alone would
        // warn about a couple of thousand sats, which is not worth a warning.
        let limit = f
            .hot_wallet_cap_sats
            .saturating_mul(2)
            .max(LARGE_BALANCE_FLOOR_SATS);
        if f.hot_wallet_cap_sats > 0 && offchain > limit {
            let detail = if f.payout == Payout::Set {
                "More is held here than usual. Money is normally sent on to your own wallet by itself; look under Recent transfers on Home, or press Move my money now."
            } else {
                "Move some to your own wallet. Set a withdrawal address in Settings so this happens automatically."
            };
            push(
                "balance",
                Level::Warn,
                "A large balance is sitting in the shop wallet",
                detail,
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
            needs_refresh_sats: Some(0),
            offchain_sats: Some(1_000),
            hot_wallet_cap_sats: 500_000,
            webhook_configured: true,
            price_age_secs: Some(60),
            ..Facts::default()
        }
    }

    #[test]
    fn coins_too_small_to_renew_get_one_plain_yellow_line() {
        let mut f = healthy();
        assert!(build(&f).iter().all(|c| c.id != "renewal_small"));
        f.too_small_to_renew_sats = Some(136);
        f.too_small_lapse_in_secs = Some(30 * 3600);
        let c = build(&f);
        let line = find(&c, "renewal_small");
        assert_eq!(line.level, Level::Warn);
        assert!(line.title.contains("136 sats"), "{}", line.title);
        assert!(line.detail.contains("in about 30 hours"), "{}", line.detail);
        f.too_small_lapse_in_secs = Some(3 * 86_400);
        assert!(find(&build(&f), "renewal_small")
            .detail
            .contains("about 3 days"));
    }

    #[test]
    fn a_wallet_on_the_other_network_from_the_mode_is_a_red_line() {
        let mut f = healthy();
        // Matching: nothing to say.
        for (live, net) in [(false, "signet"), (false, "regtest"), (true, "bitcoin")] {
            f.live = live;
            f.wallet_network = Some(net.into());
            assert!(build(&f).iter().all(|c| c.id != "network"), "{live} {net}");
        }
        // Unknown: nothing to say.
        f.wallet_network = None;
        assert!(build(&f).iter().all(|c| c.id != "network"));
        // Live server, play-money wallet.
        f.live = true;
        f.wallet_network = Some("signet".into());
        let c = build(&f);
        assert_eq!(find(&c, "network").level, Level::Bad);
        assert!(find(&c, "network").title.contains("real money"));
        // Test server, real-money wallet (either spelling).
        f.live = false;
        for net in ["bitcoin", "Mainnet"] {
            f.wallet_network = Some(net.into());
            let c = build(&f);
            assert_eq!(find(&c, "network").level, Level::Bad, "{net}");
            assert!(find(&c, "network").title.contains("REAL"));
        }
    }

    fn find<'a>(c: &'a [Check], id: &str) -> &'a Check {
        c.iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    #[test]
    fn an_old_or_missing_bitcoin_price_is_flagged_in_plain_words() {
        let mut f = healthy();
        assert!(
            build(&f).iter().all(|c| c.id != "price"),
            "fresh: nothing to say"
        );

        f.price_age_secs = None;
        assert_eq!(find(&build(&f), "price").level, Level::Warn);

        f.price_age_secs = Some(crate::rates::OLD_AFTER_SECS + 1);
        let c = build(&f);
        assert_eq!(find(&c, "price").level, Level::Warn);
        assert!(find(&c, "price")
            .detail
            .contains("keeps using the last price"));

        f.price_age_secs = Some(crate::rates::MAX_AGE_SECS + 1);
        assert_eq!(find(&build(&f), "price").level, Level::Bad);
    }

    #[test]
    fn repeated_renewal_failures_warn_and_become_an_alarm_when_coins_are_due() {
        let mut f = healthy();
        f.renewals_failing = 2;
        assert!(
            build(&f).iter().all(|c| c.id != "renewals"),
            "two is a blip"
        );
        f.renewals_failing = 3;
        assert_eq!(find(&build(&f), "renewals").level, Level::Warn);
        f.needs_refresh_sats = Some(500);
        let c = build(&f);
        assert_eq!(find(&c, "renewals").level, Level::Bad);
        assert!(find(&c, "renewals").detail.contains("last 3"));
    }

    #[test]
    fn an_invalid_payout_address_is_an_alarm() {
        let mut f = healthy();
        f.payout = Payout::Invalid;
        let c = build(&f);
        assert_eq!(find(&c, "payout").level, Level::Bad);
        assert!(find(&c, "payout").title.contains("not valid"));
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
    fn expiry_and_late_payments_warn_with_numbers() {
        let mut f = healthy();
        f.needs_refresh_sats = Some(777);
        f.unmatched_open = 2;
        let c = build(&f);
        assert!(find(&c, "expiry").detail.contains("777 sats"));
        assert!(
            !find(&c, "expiry").detail.contains('$'),
            "no price known, no dollars"
        );
        f.usd_per_btc = Some(40_000.0);
        let c = build(&f);
        assert!(find(&c, "expiry").detail.contains("777 sats ($0.31)"));
        assert!(find(&c, "late").title.starts_with('2'));
        assert!(["expiry", "late"]
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
        f.needs_refresh_sats = None;
        f.offchain_sats = None;
        let c = build(&f);
        assert!(c.iter().all(|x| x.id != "expiry"));
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

    #[test]
    fn a_small_kept_amount_does_not_make_a_small_balance_a_large_one() {
        let mut f = healthy();
        f.hot_wallet_cap_sats = 1_000;
        // Above twice the kept amount, but only a couple of thousand sats.
        f.offchain_sats = Some(2_195);
        assert!(build(&f).iter().all(|x| x.id != "balance"));
        f.offchain_sats = Some(100_000);
        assert!(build(&f).iter().all(|x| x.id != "balance"));
        f.offchain_sats = Some(100_001);
        assert_eq!(find(&build(&f), "balance").level, Level::Warn);
        // A large kept amount still uses twice that amount.
        f.hot_wallet_cap_sats = 500_000;
        f.offchain_sats = Some(900_000);
        assert!(build(&f).iter().all(|x| x.id != "balance"));
    }
}
