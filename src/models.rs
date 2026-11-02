use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Invoice lifecycle.
///
/// ```text
/// pending → paid | underpaid | overpaid | expired | cancelled
/// underpaid → expired
/// ```
///
/// All other states are terminal. In particular there is no partial-payment
/// top-up path in v1.0: an underpaid invoice expires and the customer starts a
/// fresh invoice at the current rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Pending,
    Paid,
    Underpaid,
    Overpaid,
    Expired,
    Cancelled,
}

impl InvoiceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            InvoiceStatus::Pending => "pending",
            InvoiceStatus::Paid => "paid",
            InvoiceStatus::Underpaid => "underpaid",
            InvoiceStatus::Overpaid => "overpaid",
            InvoiceStatus::Expired => "expired",
            InvoiceStatus::Cancelled => "cancelled",
        }
    }

    /// True when no further transition is possible.
    pub fn is_terminal(self) -> bool {
        !matches!(self, InvoiceStatus::Pending | InvoiceStatus::Underpaid)
    }

    /// True when the invoice is settled from the merchant's point of view.
    pub fn is_settled(self) -> bool {
        matches!(self, InvoiceStatus::Paid | InvoiceStatus::Overpaid)
    }

    pub fn can_transition_to(self, next: InvoiceStatus) -> bool {
        use InvoiceStatus::*;
        matches!(
            (self, next),
            (Pending, Paid)
                | (Pending, Underpaid)
                | (Pending, Overpaid)
                | (Pending, Expired)
                | (Pending, Cancelled)
                | (Underpaid, Expired)
        )
    }
}

impl fmt::Display for InvoiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown invoice status: {0}")]
pub struct UnknownInvoiceStatus(pub String);

impl FromStr for InvoiceStatus {
    type Err = UnknownInvoiceStatus;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(InvoiceStatus::Pending),
            "paid" => Ok(InvoiceStatus::Paid),
            "underpaid" => Ok(InvoiceStatus::Underpaid),
            "overpaid" => Ok(InvoiceStatus::Overpaid),
            "expired" => Ok(InvoiceStatus::Expired),
            "cancelled" => Ok(InvoiceStatus::Cancelled),
            other => Err(UnknownInvoiceStatus(other.to_string())),
        }
    }
}

/// Payment rail a payment arrived on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rail {
    Ark,
    Onchain,
    Lightning,
}

impl Rail {
    pub fn as_str(self) -> &'static str {
        match self {
            Rail::Ark => "ark",
            Rail::Onchain => "onchain",
            Rail::Lightning => "lightning",
        }
    }
}

impl FromStr for Rail {
    type Err = UnknownInvoiceStatus;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ark" => Ok(Rail::Ark),
            "onchain" => Ok(Rail::Onchain),
            "lightning" => Ok(Rail::Lightning),
            other => Err(UnknownInvoiceStatus(other.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    pub id: String,
    pub name: String,
    pub webhook_url: Option<String>,
    pub webhook_secret_hash: Option<String>,
    /// Plaintext until Phase 1 at-rest encryption lands. Never logged.
    pub webhook_secret: Option<String>,
    pub policy_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Invoice {
    pub id: String,
    pub store_id: String,
    pub order_id: Option<String>,
    pub status: InvoiceStatus,
    pub amount_sats: i64,
    pub fiat_currency: Option<String>,
    pub fiat_amount: Option<String>,
    pub rate: Option<String>,
    pub rate_source: Option<String>,
    pub rate_locked_at: Option<i64>,
    pub ark_address: String,
    pub onchain_address: String,
    pub expires_at: i64,
    pub paid_at: Option<i64>,
    pub paid_sats: i64,
    pub confs_required: i64,
    pub metadata_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Payment {
    pub id: String,
    pub invoice_id: String,
    pub rail: Rail,
    pub amount_sats: i64,
    pub vtxo_id: Option<String>,
    pub txid: Option<String>,
    pub detected_at: i64,
    pub confirmed_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_can_reach_every_outcome() {
        use InvoiceStatus::*;
        for next in [Paid, Underpaid, Overpaid, Expired, Cancelled] {
            assert!(
                Pending.can_transition_to(next),
                "pending should reach {next}"
            );
        }
    }

    #[test]
    fn underpaid_only_expires() {
        use InvoiceStatus::*;
        assert!(Underpaid.can_transition_to(Expired));
        assert!(!Underpaid.can_transition_to(Paid));
        assert!(!Underpaid.can_transition_to(Cancelled));
        assert!(!Underpaid.can_transition_to(Overpaid));
    }

    #[test]
    fn terminal_states_are_terminal() {
        use InvoiceStatus::*;
        for terminal in [Paid, Overpaid, Expired, Cancelled] {
            assert!(terminal.is_terminal());
            for next in [Pending, Paid, Underpaid, Overpaid, Expired, Cancelled] {
                assert!(
                    !terminal.can_transition_to(next),
                    "{terminal} must not reach {next}"
                );
            }
        }
        assert!(!Pending.is_terminal());
        assert!(!Underpaid.is_terminal());
    }

    #[test]
    fn status_string_roundtrip() {
        for status in [
            InvoiceStatus::Pending,
            InvoiceStatus::Paid,
            InvoiceStatus::Underpaid,
            InvoiceStatus::Overpaid,
            InvoiceStatus::Expired,
            InvoiceStatus::Cancelled,
        ] {
            assert_eq!(status.as_str().parse::<InvoiceStatus>().unwrap(), status);
        }
        assert!("bogus".parse::<InvoiceStatus>().is_err());
    }

    #[test]
    fn settled_only_for_paid_and_overpaid() {
        assert!(InvoiceStatus::Paid.is_settled());
        assert!(InvoiceStatus::Overpaid.is_settled());
        assert!(!InvoiceStatus::Underpaid.is_settled());
        assert!(!InvoiceStatus::Pending.is_settled());
    }

    #[test]
    fn rail_roundtrip() {
        for rail in [Rail::Ark, Rail::Onchain, Rail::Lightning] {
            assert_eq!(rail.as_str().parse::<Rail>().unwrap(), rail);
        }
    }
}
