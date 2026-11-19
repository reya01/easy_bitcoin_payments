//! Passkeys (WebAuthn) as a second step, next to the authenticator app.
//!
//! What it does and why: instead of typing a 6-digit code, the owner approves
//! with a passkey kept in a password manager (Bitwarden, for example), on a
//! phone, or on a security key. A passkey only answers for the real web address
//! of this server, so a look-alike page cannot collect it the way it can collect
//! a typed code. This file is the checking part only: it never decides who is
//! signed in. The routes that use it come later and always sit behind the
//! password brake.
//!
//! How (background detail): the standard WebAuthn "none attestation" flow with
//! ES256 (ECDSA P-256 with SHA-256), checked with `ring`. A registration is
//! accepted only if it was made for this server's address and the challenge we
//! issued; an assertion only if its signature verifies against a stored public key
//! and its use counter has not gone backwards (synced passkeys keep the counter
//! at 0, which is allowed). We do not ask for or trust attestation (that would
//! rule out password managers) and we do not require hardware-bound keys. Only
//! public values are stored, so a stolen database cannot sign in.

use std::net::IpAddr;
use std::sync::Mutex;

use rand::RngCore;
use ring::digest;
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::auth::ct_eq;
use crate::error::AppError;

/// Most passkeys one server keeps.
pub const MAX_PASSKEYS: i64 = 10;
/// How long a challenge we issued stays valid.
const CHALLENGE_SECS: i64 = 120;
/// Most challenges waiting at once (a flood cannot grow memory).
const MAX_PENDING: usize = 32;
const USER_HANDLE_KEY: &str = "passkey_user_handle";
const MAX_CBOR_DEPTH: u32 = 8;

const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_BE: u8 = 0x08;
const FLAG_BS: u8 = 0x10;
const FLAG_AT: u8 = 0x40;

const MALFORMED: &str =
    "Your browser's passkey answer was not in the expected form. Nothing was changed.";
const NEEDS_DOMAIN: &str = "Passkeys need this server to have a web address with a name (like pay.example.com) on https://, not a bare IP address.";

fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.to_string())
}

// ---------------------------------------------------------------------------
// base64url (what browsers and WebAuthn use), no padding
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[(n & 63) as usize] as char);
        }
    }
    out
}

/// Decode base64url (trailing `=` tolerated). `None` for anything else.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = (match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        }) as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            out.push(((buf >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// A small CBOR reader (only what WebAuthn answers use)
// ---------------------------------------------------------------------------

/// One decoded CBOR value. Definite lengths only; no floats, no tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cbor {
    Uint(u64),
    Neg(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(Cbor, Cbor)>),
    Bool(bool),
    Null,
}

/// Decode one item from the start of `d`; returns it and how many bytes it used.
fn read_item(d: &[u8], depth: u32) -> Option<(Cbor, usize)> {
    if depth > MAX_CBOR_DEPTH {
        return None;
    }
    let first = *d.first()?;
    let major = first >> 5;
    let ai = first & 0x1f;
    let (arg, mut pos): (u64, usize) = match ai {
        0..=23 => (ai as u64, 1),
        24 => (*d.get(1)? as u64, 2),
        25 => (u16::from_be_bytes(d.get(1..3)?.try_into().ok()?) as u64, 3),
        26 => (u32::from_be_bytes(d.get(1..5)?.try_into().ok()?) as u64, 5),
        27 => (u64::from_be_bytes(d.get(1..9)?.try_into().ok()?), 9),
        _ => return None,
    };
    match major {
        0 => Some((Cbor::Uint(arg), pos)),
        1 => {
            if arg > i64::MAX as u64 {
                return None;
            }
            Some((Cbor::Neg(-1 - (arg as i64)), pos))
        }
        2 | 3 => {
            let len = usize::try_from(arg).ok()?;
            let end = pos.checked_add(len)?;
            let bytes = d.get(pos..end)?;
            if major == 2 {
                Some((Cbor::Bytes(bytes.to_vec()), end))
            } else {
                Some((Cbor::Text(String::from_utf8(bytes.to_vec()).ok()?), end))
            }
        }
        4 => {
            let n = usize::try_from(arg).ok()?;
            // Every item takes at least one byte, so a count beyond what is left is a lie.
            if n > d.len() - pos {
                return None;
            }
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                let (item, used) = read_item(&d[pos..], depth + 1)?;
                pos += used;
                items.push(item);
            }
            Some((Cbor::Array(items), pos))
        }
        5 => {
            let n = usize::try_from(arg).ok()?;
            if n.checked_mul(2)? > d.len() - pos {
                return None;
            }
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let (k, used) = read_item(&d[pos..], depth + 1)?;
                pos += used;
                let (v, used) = read_item(&d[pos..], depth + 1)?;
                pos += used;
                entries.push((k, v));
            }
            Some((Cbor::Map(entries), pos))
        }
        7 => match arg {
            20 => Some((Cbor::Bool(false), pos)),
            21 => Some((Cbor::Bool(true), pos)),
            22 => Some((Cbor::Null, pos)),
            _ => None,
        },
        _ => None,
    }
}

fn map_get<'a>(entries: &'a [(Cbor, Cbor)], key: &Cbor) -> Option<&'a Cbor> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

// ---------------------------------------------------------------------------
// This server's address as WebAuthn sees it
// ---------------------------------------------------------------------------

/// The "relying party": the web address passkeys are made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rp {
    /// The host name (for example `pay.example.com`).
    pub id: String,
    /// The exact origin a browser reports (`https://pay.example.com`).
    pub origin: String,
}

/// Work out the relying party from the server's public web address.
/// A bare IP address cannot hold passkeys, and a plain `http://` address only
/// works on `localhost`.
pub fn rp_from_base_url(base: &str) -> Result<Rp, AppError> {
    let lower = base.trim().to_ascii_lowercase();
    let (scheme, rest) = if let Some(r) = lower.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = lower.strip_prefix("http://") {
        ("http", r)
    } else {
        return Err(bad(NEEDS_DOMAIN));
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
        return Err(bad(NEEDS_DOMAIN));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if let Some(p) = port {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad(NEEDS_DOMAIN));
        }
    }
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if !host_ok || host.parse::<IpAddr>().is_ok() {
        return Err(bad(NEEDS_DOMAIN));
    }
    if scheme == "http" && host != "localhost" && !host.ends_with(".localhost") {
        return Err(bad(NEEDS_DOMAIN));
    }
    let port = match (scheme, port) {
        ("https", Some("443")) | ("http", Some("80")) => None,
        (_, p) => p,
    };
    let origin = match port {
        Some(p) => format!("{scheme}://{host}:{p}"),
        None => format!("{scheme}://{host}"),
    };
    Ok(Rp {
        id: host.to_string(),
        origin,
    })
}

// ---------------------------------------------------------------------------
// What we send the browser
// ---------------------------------------------------------------------------

fn credential_list(ids: &[String]) -> Vec<Value> {
    ids.iter()
        .map(|id| json!({ "type": "public-key", "id": id }))
        .collect()
}

/// Options for `navigator.credentials.create` (binary values as base64url text;
/// the page turns them back into bytes). `existing_ids` stops adding the same
/// passkey twice.
pub fn registration_options(
    rp: &Rp,
    challenge: &[u8],
    user_handle_b64: &str,
    account_name: &str,
    existing_ids: &[String],
) -> Value {
    json!({
        "challenge": b64url_encode(challenge),
        "rp": { "id": rp.id, "name": "Easy Bitcoin Payments" },
        "user": { "id": user_handle_b64, "name": account_name, "displayName": account_name },
        "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
        "timeout": 120_000,
        "attestation": "none",
        "authenticatorSelection": { "residentKey": "preferred", "userVerification": "preferred" },
        "excludeCredentials": credential_list(existing_ids),
    })
}

/// Options for `navigator.credentials.get`.
pub fn assertion_options(rp: &Rp, challenge: &[u8], allow_ids: &[String]) -> Value {
    json!({
        "challenge": b64url_encode(challenge),
        "rpId": rp.id,
        "timeout": 120_000,
        "userVerification": "preferred",
        "allowCredentials": credential_list(allow_ids),
    })
}

// ---------------------------------------------------------------------------
// Checking what the browser sends back
// ---------------------------------------------------------------------------

struct AuthData<'a> {
    rp_id_hash: &'a [u8],
    flags: u8,
    sign_count: u32,
    rest: &'a [u8],
}

fn parse_auth_data(d: &[u8]) -> Option<AuthData<'_>> {
    if d.len() < 37 {
        return None;
    }
    Some(AuthData {
        rp_id_hash: &d[0..32],
        flags: d[32],
        sign_count: u32::from_be_bytes(d[33..37].try_into().ok()?),
        rest: &d[37..],
    })
}

fn check_auth_flags(ad: &AuthData<'_>, rp: &Rp, require_uv: bool) -> Result<(), AppError> {
    let want = digest::digest(&digest::SHA256, rp.id.as_bytes());
    let want: &[u8] = want.as_ref();
    if ad.rp_id_hash != want {
        return Err(bad(
            "That passkey was made for a different web address than this server's.",
        ));
    }
    if ad.flags & FLAG_UP == 0 {
        return Err(bad("The passkey was not approved by a person."));
    }
    if require_uv && ad.flags & FLAG_UV == 0 {
        return Err(bad(
            "The passkey did not check who you are (fingerprint, face or PIN).",
        ));
    }
    Ok(())
}

fn check_client_data(
    rp: &Rp,
    expect_type: &str,
    challenge: &[u8],
    raw: &[u8],
) -> Result<(), AppError> {
    let v: Value = serde_json::from_slice(raw).map_err(|_| bad(MALFORMED))?;
    if v.get("type").and_then(Value::as_str) != Some(expect_type) {
        return Err(bad(MALFORMED));
    }
    let got = v.get("challenge").and_then(Value::as_str).unwrap_or("");
    if !ct_eq(got, &b64url_encode(challenge)) {
        return Err(bad(
            "That passkey answer was for a different request. Try again.",
        ));
    }
    if v.get("origin").and_then(Value::as_str) != Some(rp.origin.as_str()) {
        return Err(bad(&format!(
            "That passkey answer came from a different web address than {}. Open this page at {} and try again.",
            rp.origin, rp.origin
        )));
    }
    if v.get("crossOrigin").and_then(Value::as_bool) == Some(true) {
        return Err(bad("That passkey answer came from inside another page."));
    }
    Ok(())
}

/// A passkey accepted at registration, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCredential {
    /// base64url of the credential id.
    pub credential_id: String,
    /// 65-byte uncompressed P-256 point.
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub backup_eligible: bool,
    pub backed_up: bool,
}

/// The public key as an uncompressed point, from a COSE key (ES256 only).
fn cose_es256_point(key: &Cbor) -> Result<Vec<u8>, AppError> {
    let Cbor::Map(m) = key else {
        return Err(bad(MALFORMED));
    };
    let kty = map_get(m, &Cbor::Uint(1));
    let alg = map_get(m, &Cbor::Uint(3));
    let crv = map_get(m, &Cbor::Neg(-1));
    let x = map_get(m, &Cbor::Neg(-2));
    let y = map_get(m, &Cbor::Neg(-3));
    match (kty, alg, crv, x, y) {
        (
            Some(Cbor::Uint(2)),
            Some(Cbor::Neg(-7)),
            Some(Cbor::Uint(1)),
            Some(Cbor::Bytes(x)),
            Some(Cbor::Bytes(y)),
        ) if x.len() == 32 && y.len() == 32 => {
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            Ok(point)
        }
        _ => Err(bad(
            "That passkey uses a kind of key this server does not support yet (it needs ES256).",
        )),
    }
}

/// Check a new passkey. `challenge` is the one we issued for this registration
/// (the caller has already taken it from [`Challenges`]); the three strings are
/// the browser's base64url values.
pub fn verify_registration(
    rp: &Rp,
    challenge: &[u8],
    require_uv: bool,
    client_data_b64: &str,
    attestation_b64: &str,
    credential_id_b64: &str,
) -> Result<NewCredential, AppError> {
    let client_data = b64url_decode(client_data_b64).ok_or_else(|| bad(MALFORMED))?;
    check_client_data(rp, "webauthn.create", challenge, &client_data)?;

    let att = b64url_decode(attestation_b64).ok_or_else(|| bad(MALFORMED))?;
    let (obj, used) = read_item(&att, 0).ok_or_else(|| bad(MALFORMED))?;
    if used != att.len() {
        return Err(bad(MALFORMED));
    }
    let Cbor::Map(entries) = obj else {
        return Err(bad(MALFORMED));
    };
    let Some(Cbor::Bytes(auth_data)) = map_get(&entries, &Cbor::Text("authData".into())) else {
        return Err(bad(MALFORMED));
    };
    let ad = parse_auth_data(auth_data).ok_or_else(|| bad(MALFORMED))?;
    check_auth_flags(&ad, rp, require_uv)?;
    if ad.flags & FLAG_AT == 0 {
        return Err(bad("The answer did not contain a new passkey."));
    }
    // aaguid (16) | credential id length (2) | credential id | COSE public key
    let rest = ad.rest;
    if rest.len() < 18 {
        return Err(bad(MALFORMED));
    }
    let id_len = u16::from_be_bytes([rest[16], rest[17]]) as usize;
    let id_end = 18 + id_len;
    if id_len == 0 || id_len > 1023 || rest.len() < id_end {
        return Err(bad(MALFORMED));
    }
    let cred_id = &rest[18..id_end];
    let (cose, _) = read_item(&rest[id_end..], 0).ok_or_else(|| bad(MALFORMED))?;
    let public_key = cose_es256_point(&cose)?;

    let claimed = b64url_decode(credential_id_b64).ok_or_else(|| bad(MALFORMED))?;
    if claimed != cred_id {
        return Err(bad(MALFORMED));
    }
    Ok(NewCredential {
        credential_id: b64url_encode(cred_id),
        public_key,
        sign_count: ad.sign_count,
        backup_eligible: ad.flags & FLAG_BE != 0,
        backed_up: ad.flags & FLAG_BS != 0,
    })
}

/// A stored passkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPasskey {
    pub id: i64,
    pub credential_id: String,
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub name: String,
    pub backup_eligible: bool,
    pub backed_up: bool,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// What a good assertion tells us to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssertionOk {
    pub new_sign_count: u32,
    pub backed_up: bool,
}

/// Check a passkey answer against a stored passkey. `challenge` is the one we
/// issued (already taken from [`Challenges`]).
pub fn verify_assertion(
    rp: &Rp,
    challenge: &[u8],
    require_uv: bool,
    stored: &StoredPasskey,
    client_data_b64: &str,
    authenticator_data_b64: &str,
    signature_b64: &str,
) -> Result<AssertionOk, AppError> {
    let client_data = b64url_decode(client_data_b64).ok_or_else(|| bad(MALFORMED))?;
    check_client_data(rp, "webauthn.get", challenge, &client_data)?;
    let auth_data = b64url_decode(authenticator_data_b64).ok_or_else(|| bad(MALFORMED))?;
    let signature = b64url_decode(signature_b64).ok_or_else(|| bad(MALFORMED))?;
    let ad = parse_auth_data(&auth_data).ok_or_else(|| bad(MALFORMED))?;
    check_auth_flags(&ad, rp, require_uv)?;

    // The signature covers the authenticator data and a hash of the client data.
    let mut signed = auth_data.clone();
    signed.extend_from_slice(digest::digest(&digest::SHA256, &client_data).as_ref());
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &stored.public_key)
        .verify(&signed, &signature)
        .map_err(|_| bad("That passkey's signature did not check out."))?;

    // A counter that does not go up means a copied passkey. Synced passkeys keep it at 0.
    let new_count = ad.sign_count;
    if (new_count != 0 || stored.sign_count != 0) && new_count <= stored.sign_count {
        return Err(bad(
            "That passkey looks like a copy (its use counter went backwards), so it was refused.",
        ));
    }
    Ok(AssertionOk {
        new_sign_count: new_count,
        backed_up: ad.flags & FLAG_BS != 0,
    })
}

// ---------------------------------------------------------------------------
// Challenges we have issued (in memory, single use)
// ---------------------------------------------------------------------------

/// What a challenge was issued for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Adding a passkey in Settings.
    Register,
    /// Signing in after the password was right.
    SignIn,
    /// Approving a protected action.
    StepUp,
}

struct Pending {
    challenge: Vec<u8>,
    purpose: Purpose,
    binding: String,
    made_at: i64,
}

/// Challenges waiting for an answer. Each is single use, expires after two
/// minutes, and is tied to what it was issued for (`binding` is the session or
/// the half-finished sign-in it belongs to).
#[derive(Default)]
pub struct Challenges {
    pending: Mutex<Vec<Pending>>,
}

impl Challenges {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh 32-byte random challenge.
    pub fn issue(&self, purpose: Purpose, binding: &str, now: i64) -> Vec<u8> {
        let mut challenge = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut challenge);
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|x| now - x.made_at <= CHALLENGE_SECS);
        if p.len() >= MAX_PENDING {
            p.remove(0);
        }
        p.push(Pending {
            challenge: challenge.clone(),
            purpose,
            binding: binding.to_string(),
            made_at: now,
        });
        challenge
    }

    /// Use up a challenge (given back by the page as base64url). `None` when it
    /// is unknown, expired, already used, or issued for something else.
    pub fn take(
        &self,
        challenge_b64: &str,
        purpose: Purpose,
        binding: &str,
        now: i64,
    ) -> Option<Vec<u8>> {
        let want = b64url_decode(challenge_b64)?;
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|x| now - x.made_at <= CHALLENGE_SECS);
        let pos = p
            .iter()
            .position(|x| x.challenge == want && x.purpose == purpose && x.binding == binding)?;
        Some(p.remove(pos).challenge)
    }
}

/// The one challenge store for this server (they only live two minutes, in memory).
pub fn challenges() -> &'static Challenges {
    static STORE: std::sync::OnceLock<Challenges> = std::sync::OnceLock::new();
    STORE.get_or_init(Challenges::new)
}

// ---------------------------------------------------------------------------
// Stored passkeys
// ---------------------------------------------------------------------------

fn row_to_passkey(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPasskey> {
    Ok(StoredPasskey {
        id: r.get(0)?,
        credential_id: r.get(1)?,
        public_key: r.get(2)?,
        sign_count: r.get::<_, i64>(3)? as u32,
        name: r.get(4)?,
        backup_eligible: r.get::<_, i64>(5)? != 0,
        backed_up: r.get::<_, i64>(6)? != 0,
        created_at: r.get(7)?,
        last_used_at: r.get(8)?,
    })
}

/// All passkeys, oldest first.
pub fn list(conn: &Connection) -> Result<Vec<StoredPasskey>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, credential_id, public_key, sign_count, name, backup_eligible, backed_up, created_at, last_used_at
         FROM admin_passkeys ORDER BY id",
    )?;
    let rows = stmt.query_map([], row_to_passkey)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn count(conn: &Connection) -> Result<i64, AppError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM admin_passkeys", [], |r| r.get(0))?)
}

/// Is at least one passkey set up?
pub fn any(conn: &Connection) -> Result<bool, AppError> {
    Ok(count(conn)? > 0)
}

pub fn get_by_credential(
    conn: &Connection,
    credential_id: &str,
) -> Result<Option<StoredPasskey>, AppError> {
    Ok(conn
        .query_row(
            "SELECT id, credential_id, public_key, sign_count, name, backup_eligible, backed_up, created_at, last_used_at
             FROM admin_passkeys WHERE credential_id = ?1",
            params![credential_id],
            row_to_passkey,
        )
        .optional()?)
}

/// A short plain name for a passkey (control characters dropped, 40 characters).
pub fn clean_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let short: String = cleaned.chars().take(40).collect();
    if short.is_empty() {
        "Passkey".to_string()
    } else {
        short
    }
}

/// Store a new passkey.
pub fn add(
    conn: &Connection,
    cred: &NewCredential,
    name: &str,
    now: i64,
) -> Result<StoredPasskey, AppError> {
    if count(conn)? >= MAX_PASSKEYS {
        return Err(AppError::Conflict(format!(
            "You already have {MAX_PASSKEYS} passkeys. Remove one before adding another."
        )));
    }
    if get_by_credential(conn, &cred.credential_id)?.is_some() {
        return Err(AppError::Conflict("That passkey is already added.".into()));
    }
    conn.execute(
        "INSERT INTO admin_passkeys
            (credential_id, public_key, sign_count, name, backup_eligible, backed_up, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            cred.credential_id,
            cred.public_key,
            cred.sign_count as i64,
            clean_name(name),
            cred.backup_eligible as i64,
            cred.backed_up as i64,
            now
        ],
    )?;
    get_by_credential(conn, &cred.credential_id)?
        .ok_or_else(|| AppError::Internal("passkey was not saved".into()))
}

/// Remember a good use of a passkey.
pub fn record_use(
    conn: &Connection,
    id: i64,
    new_sign_count: u32,
    backed_up: bool,
    now: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE admin_passkeys SET sign_count = ?1, backed_up = ?2, last_used_at = ?3 WHERE id = ?4",
        params![new_sign_count as i64, backed_up as i64, now, id],
    )?;
    Ok(())
}

/// Remove one passkey. `true` if there was one.
pub fn remove(conn: &Connection, id: i64) -> Result<bool, AppError> {
    Ok(conn.execute("DELETE FROM admin_passkeys WHERE id = ?1", params![id])? > 0)
}

/// Remove every passkey (turning two-step sign-in off from the console, or a restore).
pub fn remove_all(conn: &Connection) -> Result<usize, AppError> {
    Ok(conn.execute("DELETE FROM admin_passkeys", [])?)
}

/// The stable opaque id WebAuthn wants for the one admin account (made once).
pub fn user_handle(conn: &Connection) -> Result<String, AppError> {
    if let Some(v) = crate::settings::get(conn, USER_HANDLE_KEY)? {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let handle = b64url_encode(&raw);
    crate::settings::set(conn, USER_HANDLE_KEY, &handle)?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    fn rp() -> Rp {
        rp_from_base_url("https://pay.example.com").unwrap()
    }

    // --- a software passkey that plays the browser's and authenticator's part ---

    fn cbor_head(major: u8, n: usize) -> Vec<u8> {
        if n < 24 {
            vec![(major << 5) | (n as u8)]
        } else if n < 256 {
            vec![(major << 5) | 24, n as u8]
        } else {
            vec![(major << 5) | 25, (n >> 8) as u8, n as u8]
        }
    }
    fn cbor_bytes(b: &[u8]) -> Vec<u8> {
        let mut v = cbor_head(2, b.len());
        v.extend_from_slice(b);
        v
    }
    fn cbor_text(s: &str) -> Vec<u8> {
        let mut v = cbor_head(3, s.len());
        v.extend_from_slice(s.as_bytes());
        v
    }

    struct Soft {
        pair: EcdsaKeyPair,
        rng: SystemRandom,
        cred_id: Vec<u8>,
    }

    impl Soft {
        fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            let pair =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                    .unwrap();
            Soft {
                pair,
                rng,
                cred_id: (1..=32u8).collect(),
            }
        }

        fn point(&self) -> Vec<u8> {
            self.pair.public_key().as_ref().to_vec()
        }

        fn cose(&self) -> Vec<u8> {
            let p = self.point();
            let mut v = vec![0xA5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
            v.extend_from_slice(&p[1..33]);
            v.extend_from_slice(&[0x22, 0x58, 0x20]);
            v.extend_from_slice(&p[33..65]);
            v
        }

        fn auth_data(&self, rp_id: &str, flags: u8, count: u32, with_cred: bool) -> Vec<u8> {
            let mut v = digest::digest(&digest::SHA256, rp_id.as_bytes())
                .as_ref()
                .to_vec();
            v.push(flags);
            v.extend_from_slice(&count.to_be_bytes());
            if with_cred {
                v.extend_from_slice(&[0u8; 16]);
                v.extend_from_slice(&(self.cred_id.len() as u16).to_be_bytes());
                v.extend_from_slice(&self.cred_id);
                v.extend_from_slice(&self.cose());
            }
            v
        }

        fn client_data(kind: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
            format!(
                r#"{{"type":"{kind}","challenge":"{}","origin":"{origin}","crossOrigin":false}}"#,
                b64url_encode(challenge)
            )
            .into_bytes()
        }

        /// (clientDataJSON, attestationObject, credential id), all base64url.
        fn register(
            &self,
            rp_id: &str,
            origin: &str,
            challenge: &[u8],
            flags: u8,
        ) -> (String, String, String) {
            let client = Self::client_data("webauthn.create", challenge, origin);
            let auth = self.auth_data(rp_id, flags, 0, true);
            let mut att = vec![0xA3];
            att.extend(cbor_text("fmt"));
            att.extend(cbor_text("none"));
            att.extend(cbor_text("attStmt"));
            att.push(0xA0);
            att.extend(cbor_text("authData"));
            att.extend(cbor_bytes(&auth));
            (
                b64url_encode(&client),
                b64url_encode(&att),
                b64url_encode(&self.cred_id),
            )
        }

        /// (clientDataJSON, authenticatorData, signature), all base64url.
        fn assert(
            &self,
            rp_id: &str,
            origin: &str,
            challenge: &[u8],
            flags: u8,
            count: u32,
        ) -> (String, String, String) {
            let client = Self::client_data("webauthn.get", challenge, origin);
            let auth = self.auth_data(rp_id, flags, count, false);
            let mut msg = auth.clone();
            msg.extend_from_slice(digest::digest(&digest::SHA256, &client).as_ref());
            let sig = self.pair.sign(&self.rng, &msg).unwrap();
            (
                b64url_encode(&client),
                b64url_encode(&auth),
                b64url_encode(sig.as_ref()),
            )
        }

        fn stored(&self, sign_count: u32) -> StoredPasskey {
            StoredPasskey {
                id: 1,
                credential_id: b64url_encode(&self.cred_id),
                public_key: self.point(),
                sign_count,
                name: "test".into(),
                backup_eligible: true,
                backed_up: true,
                created_at: 0,
                last_used_at: None,
            }
        }
    }

    const CH: &[u8] = b"0123456789abcdef0123456789abcdef";
    const UP_AT: u8 = FLAG_UP | FLAG_AT;

    // --- base64url and CBOR ---

    #[test]
    fn base64url_round_trips_and_matches_known_values() {
        assert_eq!(b64url_encode(b""), "");
        assert_eq!(b64url_encode(b"f"), "Zg");
        assert_eq!(b64url_encode(b"fo"), "Zm8");
        assert_eq!(b64url_encode(b"foo"), "Zm9v");
        assert_eq!(b64url_encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(b64url_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(b64url_decode("-_8").unwrap(), vec![0xfb, 0xff]);
        assert_eq!(b64url_decode("Zg==").unwrap(), b"f");
        for n in 0..70usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(b64url_decode(&b64url_encode(&data)).unwrap(), data, "{n}");
        }
        assert!(b64url_decode("a").is_none());
        assert!(b64url_decode("ab+/").is_none());
        assert!(b64url_decode("ab cd").is_none());
    }

    #[test]
    fn cbor_reads_the_shapes_webauthn_uses() {
        // {1: 2, -1: h'0102', "k": [true, null, 24]}
        let mut d = vec![0xA3, 0x01, 0x02, 0x20];
        d.extend(cbor_bytes(&[1, 2]));
        d.extend(cbor_text("k"));
        d.extend([0x83, 0xF5, 0xF6, 0x18, 24]);
        let (v, used) = read_item(&d, 0).unwrap();
        assert_eq!(used, d.len());
        let Cbor::Map(m) = v else { panic!("not a map") };
        assert_eq!(map_get(&m, &Cbor::Uint(1)), Some(&Cbor::Uint(2)));
        assert_eq!(map_get(&m, &Cbor::Neg(-1)), Some(&Cbor::Bytes(vec![1, 2])));
        assert_eq!(
            map_get(&m, &Cbor::Text("k".into())),
            Some(&Cbor::Array(vec![
                Cbor::Bool(true),
                Cbor::Null,
                Cbor::Uint(24)
            ]))
        );
    }

    #[test]
    fn cbor_refuses_what_it_does_not_understand() {
        assert!(read_item(&[], 0).is_none());
        assert!(read_item(&[0x5F], 0).is_none(), "indefinite length");
        assert!(read_item(&[0xC1, 0x01], 0).is_none(), "tags");
        assert!(read_item(&[0xF9, 0x00, 0x00], 0).is_none(), "floats");
        assert!(read_item(&[0x42, 0x01], 0).is_none(), "bytes cut short");
        assert!(read_item(&[0x9B, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 0).is_none());
        assert!(
            read_item(&[0x84, 0x01], 0).is_none(),
            "array count beyond the data"
        );
        // Deeply nested arrays are refused, not followed to the bottom.
        let deep = vec![0x81u8; 50];
        assert!(read_item(&deep, 0).is_none());
        assert!(
            read_item(&[0xA1, 0x01], 0).is_none(),
            "map missing its value"
        );
    }

    // --- the server's address ---

    #[test]
    fn the_relying_party_comes_from_the_public_address() {
        let r = rp_from_base_url("https://Pay.Example.com/").unwrap();
        assert_eq!(r.id, "pay.example.com");
        assert_eq!(r.origin, "https://pay.example.com");
        let r = rp_from_base_url("https://pay.example.com:443/shop").unwrap();
        assert_eq!(r.origin, "https://pay.example.com");
        let r = rp_from_base_url("https://pay.example.com:8443").unwrap();
        assert_eq!(r.id, "pay.example.com");
        assert_eq!(r.origin, "https://pay.example.com:8443");
        let r = rp_from_base_url("http://localhost:8080").unwrap();
        assert_eq!(
            (r.id.as_str(), r.origin.as_str()),
            ("localhost", "http://localhost:8080")
        );
    }

    #[test]
    fn addresses_that_cannot_hold_passkeys_are_refused_in_plain_words() {
        for base in [
            "https://203.0.113.9",
            "https://203.0.113.9:8443",
            "https://[2001:db8::1]",
            "http://pay.example.com",
            "http://203.0.113.9:8080",
            "ftp://pay.example.com",
            "pay.example.com",
            "https://",
            "https://user@pay.example.com",
            "https://pay.example.com:abc",
        ] {
            let e = rp_from_base_url(base).unwrap_err().to_string();
            assert!(e.contains("web address"), "{base}: {e}");
        }
    }

    // --- registration ---

    #[test]
    fn a_good_registration_is_accepted_and_keeps_the_public_key() {
        let s = Soft::new();
        let r = rp();
        let (cd, att, id) = s.register(&r.id, &r.origin, CH, UP_AT | FLAG_BE | FLAG_BS | FLAG_UV);
        let c = verify_registration(&r, CH, true, &cd, &att, &id).unwrap();
        assert_eq!(c.public_key, s.point());
        assert_eq!(c.public_key.len(), 65);
        assert_eq!(c.credential_id, b64url_encode(&s.cred_id));
        assert!(c.backup_eligible && c.backed_up);
        assert_eq!(c.sign_count, 0);
    }

    #[test]
    fn a_registration_is_refused_when_anything_is_off() {
        let s = Soft::new();
        let r = rp();
        let other = b"ffffffffffffffffffffffffffffffff";
        let reg =
            |rp_id: &str, origin: &str, ch: &[u8], flags: u8| s.register(rp_id, origin, ch, flags);
        let ok = reg(&r.id, &r.origin, CH, UP_AT);
        let check = |t: (String, String, String), uv: bool| {
            verify_registration(&r, CH, uv, &t.0, &t.1, &t.2)
        };
        assert!(check(ok.clone(), false).is_ok());
        // user verification required but not done
        assert!(check(ok.clone(), true).is_err());
        // answer for a different challenge
        assert!(check(reg(&r.id, &r.origin, other, UP_AT), false).is_err());
        // wrong origin
        let e = check(reg(&r.id, "https://evil.example.net", CH, UP_AT), false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("different web address"), "{e}");
        // made for another site
        assert!(check(reg("evil.example.net", &r.origin, CH, UP_AT), false).is_err());
        // nobody approved
        assert!(check(reg(&r.id, &r.origin, CH, FLAG_AT), false).is_err());
        // no new passkey in the answer
        assert!(check(reg(&r.id, &r.origin, CH, FLAG_UP), false).is_err());
        // credential id that does not match the one inside
        let (cd, att, _) = ok.clone();
        assert!(
            verify_registration(&r, CH, false, &cd, &att, &b64url_encode(b"another id")).is_err()
        );
        // garbage
        assert!(verify_registration(&r, CH, false, "!!", &ok.1, &ok.2).is_err());
        assert!(verify_registration(&r, CH, false, &ok.0, "AAAA", &ok.2).is_err());
        // a get answer is not a create answer
        let (cd, ad, _) = s.assert(&r.id, &r.origin, CH, FLAG_UP, 1);
        assert!(verify_registration(&r, CH, false, &cd, &ad, &ok.2).is_err());
    }

    #[test]
    fn a_registration_for_another_kind_of_key_is_refused() {
        let s = Soft::new();
        let r = rp();
        // Same answer, but the key says it is RS256 (alg -257).
        let client = Soft::client_data("webauthn.create", CH, &r.origin);
        let mut auth = s.auth_data(&r.id, UP_AT, 0, true);
        let at = auth.len() - s.cose().len();
        auth[at + 4] = 0x39; // alg: negative, two-byte value
        auth.insert(at + 5, 0x00); // -257 is 0x39 0x01 0x00
        auth.insert(at + 5, 0x01);
        let mut att = vec![0xA3];
        att.extend(cbor_text("fmt"));
        att.extend(cbor_text("none"));
        att.extend(cbor_text("attStmt"));
        att.push(0xA0);
        att.extend(cbor_text("authData"));
        att.extend(cbor_bytes(&auth));
        let e = verify_registration(
            &r,
            CH,
            false,
            &b64url_encode(&client),
            &b64url_encode(&att),
            &b64url_encode(&s.cred_id),
        );
        assert!(e.is_err());
    }

    // --- sign-in answers ---

    #[test]
    fn a_good_assertion_is_accepted() {
        let s = Soft::new();
        let r = rp();
        let (cd, ad, sig) = s.assert(&r.id, &r.origin, CH, FLAG_UP | FLAG_UV | FLAG_BS, 0);
        let ok = verify_assertion(&r, CH, true, &s.stored(0), &cd, &ad, &sig).unwrap();
        assert_eq!(
            ok,
            AssertionOk {
                new_sign_count: 0,
                backed_up: true
            }
        );
    }

    #[test]
    fn an_assertion_is_refused_when_anything_is_off() {
        let s = Soft::new();
        let r = rp();
        let other_key = Soft::new();
        let (cd, ad, sig) = s.assert(&r.id, &r.origin, CH, FLAG_UP, 0);
        let stored = s.stored(0);
        let run = |cd: &str, ad: &str, sig: &str, uv: bool| {
            verify_assertion(&r, CH, uv, &stored, cd, ad, sig)
        };
        assert!(run(&cd, &ad, &sig, false).is_ok());
        assert!(
            run(&cd, &ad, &sig, true).is_err(),
            "user verification required"
        );
        // a signature from another key
        let (cd2, ad2, sig2) = other_key.assert(&r.id, &r.origin, CH, FLAG_UP, 0);
        assert!(run(&cd2, &ad2, &sig2, false).is_err());
        // a signature over different data (same key, other counter)
        let (_, ad_other, _) = s.assert(&r.id, &r.origin, CH, FLAG_UP, 9);
        assert!(run(&cd, &ad_other, &sig, false).is_err());
        // a damaged signature
        let mut bad_sig = b64url_decode(&sig).unwrap();
        let last = bad_sig.len() - 1;
        bad_sig[last] ^= 0x01;
        assert!(run(&cd, &ad, &b64url_encode(&bad_sig), false).is_err());
        // wrong challenge, origin, site
        let other = b"ffffffffffffffffffffffffffffffff";
        let (c3, a3, s3) = s.assert(&r.id, &r.origin, other, FLAG_UP, 0);
        assert!(run(&c3, &a3, &s3, false).is_err());
        let (c4, a4, s4) = s.assert(&r.id, "https://evil.example.net", CH, FLAG_UP, 0);
        assert!(run(&c4, &a4, &s4, false).is_err());
        let (c5, a5, s5) = s.assert("evil.example.net", &r.origin, CH, FLAG_UP, 0);
        assert!(run(&c5, &a5, &s5, false).is_err());
        // nobody approved
        let (c6, a6, s6) = s.assert(&r.id, &r.origin, CH, 0, 0);
        assert!(run(&c6, &a6, &s6, false).is_err());
        // garbage
        assert!(run("!!", &ad, &sig, false).is_err());
        assert!(run(&cd, "AA", &sig, false).is_err());
    }

    #[test]
    fn the_use_counter_may_stay_at_zero_but_never_go_back() {
        let s = Soft::new();
        let r = rp();
        let try_count = |stored: u32, new: u32| {
            let (cd, ad, sig) = s.assert(&r.id, &r.origin, CH, FLAG_UP, new);
            verify_assertion(&r, CH, false, &s.stored(stored), &cd, &ad, &sig)
        };
        assert!(try_count(0, 0).is_ok(), "synced passkeys stay at 0");
        assert_eq!(try_count(0, 1).unwrap().new_sign_count, 1);
        assert_eq!(try_count(5, 6).unwrap().new_sign_count, 6);
        assert!(try_count(5, 5).is_err(), "same counter again");
        assert!(try_count(5, 4).is_err(), "counter went back");
        assert!(try_count(5, 0).is_err(), "counter reset to zero");
    }

    // --- options we send ---

    #[test]
    fn the_options_ask_for_what_password_managers_give() {
        let r = rp();
        let o = registration_options(&r, CH, "handle", "My Shop", &["abc".into()]);
        assert_eq!(o["rp"]["id"], "pay.example.com");
        assert_eq!(o["attestation"], "none");
        assert_eq!(o["pubKeyCredParams"][0]["alg"], -7);
        assert_eq!(o["authenticatorSelection"]["userVerification"], "preferred");
        assert_eq!(o["excludeCredentials"][0]["id"], "abc");
        assert_eq!(o["challenge"], b64url_encode(CH));
        let a = assertion_options(&r, CH, &["abc".into(), "def".into()]);
        assert_eq!(a["rpId"], "pay.example.com");
        assert_eq!(a["allowCredentials"].as_array().unwrap().len(), 2);
    }

    // --- challenges ---

    #[test]
    fn a_challenge_works_once_for_what_it_was_issued_for() {
        let c = Challenges::new();
        let ch = c.issue(Purpose::Register, "sess1", 1000);
        let b64 = b64url_encode(&ch);
        assert!(
            c.take(&b64, Purpose::SignIn, "sess1", 1001).is_none(),
            "wrong purpose"
        );
        assert!(
            c.take(&b64, Purpose::Register, "sess2", 1001).is_none(),
            "wrong owner"
        );
        assert_eq!(c.take(&b64, Purpose::Register, "sess1", 1001), Some(ch));
        assert!(
            c.take(&b64, Purpose::Register, "sess1", 1002).is_none(),
            "single use"
        );
        assert!(c.take("!!", Purpose::Register, "sess1", 1002).is_none());
    }

    #[test]
    fn a_challenge_expires_and_a_flood_is_capped() {
        let c = Challenges::new();
        let ch = c.issue(Purpose::StepUp, "s", 1000);
        assert!(c
            .take(
                &b64url_encode(&ch),
                Purpose::StepUp,
                "s",
                1000 + CHALLENGE_SECS + 1
            )
            .is_none());
        let first = c.issue(Purpose::StepUp, "s", 5000);
        for _ in 0..(MAX_PENDING + 5) {
            c.issue(Purpose::StepUp, "s", 5000);
        }
        assert!(c.pending.lock().unwrap().len() <= MAX_PENDING);
        assert!(
            c.take(&b64url_encode(&first), Purpose::StepUp, "s", 5001)
                .is_none(),
            "the oldest one is dropped first"
        );
    }

    // --- stored passkeys ---

    fn db() -> crate::db::Db {
        crate::db::Db::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn new_cred(n: u8) -> NewCredential {
        NewCredential {
            credential_id: b64url_encode(&[n; 16]),
            public_key: vec![4; 65],
            sign_count: 0,
            backup_eligible: true,
            backed_up: false,
        }
    }

    #[tokio::test]
    async fn passkeys_can_be_added_listed_used_and_removed() {
        let db = db();
        db.call(|c| {
            assert!(!any(c)?);
            let a = add(c, &new_cred(1), "  Bitwarden \n on laptop ", 100)?;
            assert_eq!(a.name, "Bitwarden on laptop");
            add(c, &new_cred(2), "Phone", 101)?;
            assert_eq!(count(c)?, 2);
            assert_eq!(
                list(c)?.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
                vec!["Bitwarden on laptop", "Phone"]
            );
            // the same passkey twice
            assert!(matches!(
                add(c, &new_cred(1), "again", 102),
                Err(AppError::Conflict(_))
            ));
            record_use(c, a.id, 7, true, 200)?;
            let got = get_by_credential(c, &a.credential_id)?.unwrap();
            assert_eq!(
                (got.sign_count, got.backed_up, got.last_used_at),
                (7, true, Some(200))
            );
            assert!(get_by_credential(c, "nope")?.is_none());
            assert!(remove(c, a.id)?);
            assert!(!remove(c, a.id)?);
            assert_eq!(remove_all(c)?, 1);
            assert!(!any(c)?);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn there_is_a_limit_on_how_many_passkeys_are_kept() {
        let db = db();
        db.call(|c| {
            for n in 0..MAX_PASSKEYS as u8 {
                add(c, &new_cred(n), "k", 1)?;
            }
            assert!(matches!(
                add(c, &new_cred(200), "one more", 1),
                Err(AppError::Conflict(_))
            ));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[test]
    fn names_are_tidied() {
        assert_eq!(clean_name(""), "Passkey");
        assert_eq!(clean_name("   \t "), "Passkey");
        assert_eq!(clean_name("My\u{0}  phone"), "My phone");
        assert_eq!(clean_name(&"x".repeat(100)).chars().count(), 40);
    }

    #[tokio::test]
    async fn the_user_handle_is_made_once_and_kept() {
        let db = db();
        let (a, b) = db
            .call(|c| Ok((user_handle(c)?, user_handle(c)?)))
            .await
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(b64url_decode(&a).unwrap().len(), 32);
    }
}
