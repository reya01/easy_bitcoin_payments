//! Guard: every route that checks the admin password must sit inside the password
//! brake, the group marked BRAKED-GROUP-START / BRAKED-GROUP-END in src/routes/mod.rs.
//!
//! Why: the brake (a few tries a minute and an hour per client, see src/ratelimit.rs) is
//! what stops a stolen sign-in session from also guessing the admin password. It is
//! applied to a group of routes, so a new screen that checks the password is only safe
//! if it is registered inside that group. The Home tab's "Update now" was once added
//! outside it; this test makes that mistake fail the build instead.
//!
//! How: it reads the source. A handler counts as "checks the password" when its function
//! calls one of PASSWORD_CHECKS, or calls another function of the same file that does.
//! Then every `.route(...)` registered outside the marked group must not be such a handler.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

/// Calls that check the admin password itself.
const PASSWORD_CHECKS: &[&str] = &["verify_login_slow(", "check_password(", "verify_login("];

/// (function name, its text) for each top-level function of a file, test module left out.
fn functions(src: &str) -> Vec<(String, String)> {
    let src = src.split("#[cfg(test)]").next().unwrap_or(src);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in src.lines() {
        let head = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        let head = head.strip_prefix("async ").unwrap_or(head);
        if let Some(rest) = head.strip_prefix("fn ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            out.push((name, String::new()));
        }
        if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

/// Does `text` call a function called `name` (not just contain it inside a longer word)?
fn calls(text: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    text.match_indices(&needle).any(|(i, _)| {
        text[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

/// The functions of one file that check the password, directly or through another
/// function of the same file.
fn password_checkers(fns: &[(String, String)]) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = fns
        .iter()
        .filter(|(_, body)| PASSWORD_CHECKS.iter().any(|t| body.contains(t)))
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let before = set.len();
        for (name, body) in fns {
            if set.contains(name) {
                continue;
            }
            let reaches = set.iter().any(|c| calls(body, c));
            if reaches {
                set.insert(name.clone());
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

struct Route {
    path: String,
    module: String,
    handler: String,
    braked: bool,
}

/// Every `.route("/path", post(module::handler))` in src/routes/mod.rs, and whether it
/// sits between the brake markers.
fn routes(mod_rs: &str) -> Vec<Route> {
    let start = mod_rs
        .find("BRAKED-GROUP-START")
        .expect("the BRAKED-GROUP-START marker is missing from src/routes/mod.rs");
    let end = mod_rs
        .find("BRAKED-GROUP-END")
        .expect("the BRAKED-GROUP-END marker is missing from src/routes/mod.rs");
    assert!(start < end, "the brake markers are the wrong way round");

    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = mod_rs[at..].find(".route(") {
        let pos = at + i;
        at = pos + ".route(".len();
        let rest = &mod_rs[at..];
        // This route's own text ends where the next route starts.
        let window = &rest[..rest.find(".route(").unwrap_or(rest.len())];
        let Some(q1) = window.find('"') else { continue };
        let Some(q2) = window[q1 + 1..].find('"') else {
            continue;
        };
        let path = window[q1 + 1..q1 + 1 + q2].to_string();
        let after = &window[q1 + q2 + 2..];
        let Some(verb) = [after.find("post("), after.find("get(")]
            .into_iter()
            .flatten()
            .min()
        else {
            continue;
        };
        let call = &after[verb..];
        let Some(sep) = call.find("::") else { continue };
        let ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        };
        let module: String = {
            let before: String = call[..sep]
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            before.chars().rev().collect()
        };
        let handler = ident(&call[sep + 2..]);
        if module.is_empty() || handler.is_empty() {
            continue;
        }
        out.push(Route {
            path,
            module,
            handler,
            braked: pos > start && pos < end,
        });
    }
    out
}

#[test]
fn every_route_that_checks_the_admin_password_sits_behind_the_brake() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/routes");

    // module (file name) -> its functions that check the password
    let mut checkers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let module = path.file_stem().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(&path).unwrap();
        checkers.insert(module, password_checkers(&functions(&src)));
    }
    let routes = routes(&fs::read_to_string(dir.join("mod.rs")).unwrap());
    assert!(
        routes.len() > 40,
        "only {} routes were read from src/routes/mod.rs; has the way routes are written changed? Fix this test.",
        routes.len()
    );

    // The guard itself still sees the password checks it exists to catch.
    for (module, handler) in [
        ("admin_session", "login"),
        ("admin_session", "change_password"),
        ("backup", "download"),
        ("payout", "change"),
        ("exit", "start"),
        ("exit", "claim"),
        ("twofa", "start"),
        ("twofa", "disable"),
        ("update_helper", "apply"),
    ] {
        assert!(
            checkers
                .get(module)
                .is_some_and(|set| set.contains(handler)),
            "{module}::{handler} should be seen as checking the password; the guard is out of date"
        );
    }

    let loose: Vec<String> = routes
        .iter()
        .filter(|r| {
            checkers
                .get(&r.module)
                .is_some_and(|set| set.contains(&r.handler))
        })
        .filter(|r| !r.braked)
        .map(|r| format!("{} ({}::{})", r.path, r.module, r.handler))
        .collect();
    assert!(
        loose.is_empty(),
        "these routes check the admin password but are outside the password brake; \
         move them into the group between BRAKED-GROUP-START and BRAKED-GROUP-END in \
         src/routes/mod.rs: {loose:?}"
    );

    // Every one of those is registered inside the group (so the markers still wrap them).
    let braked: BTreeSet<&str> = routes
        .iter()
        .filter(|r| r.braked)
        .map(|r| r.path.as_str())
        .collect();
    for path in [
        "/admin/login",
        "/admin/backup",
        "/admin/payout",
        "/admin/password",
        "/admin/update/apply",
    ] {
        assert!(
            braked.contains(path),
            "{path} is not inside the brake group"
        );
    }
}

#[test]
fn the_guard_catches_a_password_route_left_outside() {
    // A pretend router file: one route inside the markers, one outside.
    let mod_rs = r#"
        .route("/a/open", post(screens::open_one))
        // BRAKED-GROUP-START
        .route("/a/inside", post(screens::inside))
        // BRAKED-GROUP-END
    "#;
    let found = routes(mod_rs);
    assert_eq!(found.len(), 2);
    assert!(!found[0].braked && found[1].braked);
    assert_eq!(
        (found[0].module.as_str(), found[0].handler.as_str()),
        ("screens", "open_one")
    );

    // A handler that reaches the password check through a helper counts as checking it.
    let src = "
async fn helper(p: String) { verify_login_slow(None, p).await; }
pub async fn open_one() { helper(String::new()).await; }
pub async fn inside() {}
pub async fn unrelated() { restart_helper(); }
";
    let set = password_checkers(&functions(src));
    assert!(set.contains("helper") && set.contains("open_one"));
    assert!(!set.contains("inside") && !set.contains("unrelated"));
}
