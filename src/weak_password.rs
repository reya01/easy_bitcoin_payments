//! A small block list of weak passwords.
//!
//! Length alone (16 characters) lets through `passwordpassword` and
//! `aaaaaaaaaaaaaaaa`. This catches the obvious cases and nothing more: it is not
//! a strength meter and it never rejects a passphrase of ordinary words. Used for
//! the admin password and for backup passwords.
//!
//! The check works on a simplified copy of the password (lower case, letters and
//! digits only, so `Pass-Word 123` and `password123` look the same) and says no to:
//!
//! * a password that uses only a handful of different characters,
//! * one on the list of very common passwords (also with digits added at the end,
//!   or one common word repeated), and
//! * a plain run along the keyboard, the alphabet or the digits.

/// Very common passwords and words, already simplified (lower case, no spaces or
/// punctuation). Includes the names people reach for first in this product.
const COMMON: &[&str] = &[
    "password",
    "passw0rd",
    "pass",
    "letmein",
    "welcome",
    "admin",
    "administrator",
    "root",
    "login",
    "master",
    "secret",
    "changeme",
    "qwerty",
    "qwertyuiop",
    "asdfghjkl",
    "iloveyou",
    "monkey",
    "dragon",
    "football",
    "baseball",
    "sunshine",
    "princess",
    "shadow",
    "trustno1",
    "superman",
    "batman",
    "starwars",
    "abc123",
    "bitcoin",
    "satoshi",
    "satoshinakamoto",
    "lightning",
    "ark",
    "ebp",
    "ebpadmin",
    "easybitcoinpayments",
    "bitcoinpayments",
    "mybitcoin",
    "hodl",
    "tothemoon",
    "correcthorsebatterystaple",
    "thequickbrownfoxjumpsoverthelazydog",
    "1q2w3e4r5t6y7u8i9o0p",
];

/// Straight runs. A password that is a stretch of one of these (or of its
/// reverse) is too easy.
const RUNS: &[&str] = &[
    "01234567890123456789",
    "abcdefghijklmnopqrstuvwxyz",
    "qwertyuiopasdfghjklzxcvbnm",
    "1qaz2wsx3edc4rfv5tgb6yhn7ujm8ik9ol0p",
];

/// Fewer different characters than this is too easy (`aaaaaaaaaaaaaaaa`).
const MIN_DISTINCT: usize = 5;

/// Runs shorter than this are not flagged (they cannot be 16 characters anyway;
/// this only stops a very short stretch from matching by accident).
const MIN_RUN_LEN: usize = 8;

fn simplify(p: &str) -> String {
    p.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Like `simplify`, but the usual look-alike swaps are undone first, so
/// `P@ssw0rd` is read as `password`.
fn simplify_look_alikes(p: &str) -> String {
    p.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            '@' => 'a',
            '$' => 's',
            '0' => 'o',
            '1' => 'i',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            other => other,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn is_common(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Exactly a common password, or one with digits added at the end.
    let without_digits = s.trim_end_matches(|c: char| c.is_ascii_digit());
    if COMMON.contains(&s) || (!without_digits.is_empty() && COMMON.contains(&without_digits)) {
        return true;
    }
    // One common word, repeated (`passwordpassword`, `adminadminadmin`),
    // also with digits added at the end (`passwordpassword1`).
    [s, without_digits].iter().any(|t| {
        COMMON
            .iter()
            .any(|w| w.len() >= 3 && t.len() >= 2 * w.len() && *t == w.repeat(t.len() / w.len()))
    })
}

fn is_run(s: &str) -> bool {
    if s.chars().count() < MIN_RUN_LEN {
        return false;
    }
    let rev: String = s.chars().rev().collect();
    RUNS.iter()
        .any(|r| r.contains(s) || r.contains(rev.as_str()))
}

/// Why this password is too easy to guess, or `None` if it is fine. The reason
/// finishes the sentence "That password is too easy to guess: …".
pub fn weak_reason(password: &str) -> Option<&'static str> {
    let mut distinct: Vec<char> = password
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < MIN_DISTINCT {
        return Some("it uses only a few different characters");
    }

    let plain = simplify(password);
    let leet = simplify_look_alikes(password);
    if is_common(&plain) || is_common(&leet) {
        return Some("it is on the list of very common passwords");
    }
    if is_run(&plain) {
        return Some("it is a simple run of keys or letters");
    }
    None
}

/// `Err` carrying a plain-words message when the password is on the block list.
pub fn check(password: &str) -> Result<(), crate::error::AppError> {
    match weak_reason(password) {
        None => Ok(()),
        Some(why) => Err(crate::error::AppError::BadRequest(format!(
            "that password is too easy to guess: {why}. Use a few random words instead"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_character_or_a_few_are_refused() {
        assert!(weak_reason("aaaaaaaaaaaaaaaa").is_some());
        assert!(weak_reason("abababababababab").is_some());
        assert!(weak_reason("1111 1111 1111 1111").is_some());
    }

    #[test]
    fn common_passwords_are_refused_however_they_are_dressed() {
        for p in [
            "passwordpassword",
            "PasswordPassword",
            "password1234567890",
            "Password-Password",
            "p@ssw0rdp@ssw0rd",
            "adminadminadminadmin",
            "passwordpassword1",
            "bitcoinbitcoin2026",
            "bitcoinbitcoinbitcoin",
            "correct horse battery staple",
            "Easy Bitcoin Payments",
            "the quick brown fox jumps over the lazy dog",
        ] {
            assert!(weak_reason(p).is_some(), "{p} should be refused");
        }
    }

    #[test]
    fn plain_runs_are_refused() {
        for p in [
            "1234567890123456",
            "6543210987654321",
            "abcdefghijklmnopqrst",
            "qwertyuiopasdfghjkl",
            "lkjhgfdsapoiuytrewq",
        ] {
            assert!(weak_reason(p).is_some(), "{p} should be refused");
        }
    }

    #[test]
    fn ordinary_passphrases_and_random_passwords_pass() {
        for p in [
            "a good long password",
            "sixteen chars!!!",
            "a perfectly fine password",
            "my password is longer than yours",
            "purple-tractor-moonlight-7",
            "Xk3v9-Qm2z-Lp8w-Tn4r",
            "correct horse battery",
            "signet drill smoke password",
            "e2e-token-for-the-test-run",
        ] {
            assert_eq!(weak_reason(p), None, "{p} should be accepted");
        }
    }

    #[test]
    fn the_message_is_plain() {
        let e = check("passwordpassword").unwrap_err().to_string();
        assert!(e.contains("too easy to guess"), "{e}");
        assert!(check("a good long password").is_ok());
    }
}
