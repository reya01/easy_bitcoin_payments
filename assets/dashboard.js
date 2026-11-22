function pwHelper(inputId, btnId, countId) {
  var pw = document.getElementById(inputId), b = document.getElementById(btnId), c = document.getElementById(countId);
  function upd() {
    var n = pw.value.length;
    c.textContent = n >= 16 ? n + " characters. Long enough." : n + " of 16 characters";
  }
  pw.addEventListener("input", upd);
  b.addEventListener("click", function () {
    var show = pw.type === "password";
    pw.type = show ? "text" : "password";
    b.textContent = show ? "Hide" : "Show";
  });
  upd();
}
pwHelper("pw-new", "pw-new-show", "pw-new-count");
(function () {
"use strict";
var csrf = "";
var twofaOn = false, twofaDeclined = false, paused = false, passkeysOn = false, loginPasskey = null;
var $ = function (id) { return document.getElementById(id); };
var views = ["login", "home", "orders", "alerts", "hooks", "set"];

function el(tag, text, cls) { var e = document.createElement(tag); if (text !== undefined) e.textContent = text; if (cls) e.className = cls; return e; }
function clear(n) { while (n.firstChild) n.removeChild(n.firstChild); }
function row(a, b, cls) { var r = el("div", "", "row"); r.appendChild(el("span", a)); r.appendChild(el("span", b, cls || "")); return r; }
// Where the guides live (rendered on GitHub). One map, so every reference is a real link.
var DOCS_BASE = "https://github.com/reya01/easy_bitcoin_payments/blob/main/docs/";
var DOCS = {
  forgot: DOCS_BASE + "IF_SOMETHING_LOOKS_WRONG.md#2-you-think-someone-saw-your-recovery-words-or-your-admin-password",
  wrong: DOCS_BASE + "IF_SOMETHING_LOOKS_WRONG.md",
  known: DOCS_BASE + "KNOWN_ISSUES.md"
};
function docLink(key, text) { var a = el("a", text); a.href = DOCS[key]; a.target = "_blank"; a.rel = "noopener noreferrer"; return a; }
document.querySelectorAll("[data-doc]").forEach(function (a) { a.href = DOCS[a.getAttribute("data-doc")]; });
$("forgot-link").href = DOCS.forgot;
// Messages are announced to screen readers: problems as alerts, the rest as status.
// "Known issues in the docs" becomes a real link.
function say(id, text, bad) {
  var m = $(id); m.textContent = "";
  text = String(text); var phrase = "Known issues in the docs", i = text.indexOf(phrase);
  if (i >= 0) {
    m.appendChild(document.createTextNode(text.slice(0, i)));
    m.appendChild(docLink("known", phrase));
    m.appendChild(document.createTextNode(text.slice(i + phrase.length)));
  } else { m.textContent = text; }
  m.className = "msg " + (bad ? "bad" : "ok");
  m.setAttribute("role", bad ? "alert" : "status");
}
// One in-page confirmation for every action that cannot be undone. (The browser's own
// confirm() box is blocked in some in-app browsers, which made guarded buttons do nothing.)
// Resolves true only when the confirm button is pressed.
function askConfirm(o) {
  return new Promise(function (resolve) {
    var before = document.activeElement;
    var back = el("div", "", ""); back.id = "dlg-back";
    var dlg = el("div", "", "dlg"); dlg.setAttribute("role", "alertdialog"); dlg.setAttribute("aria-modal", "true");
    dlg.setAttribute("aria-labelledby", "dlg-title"); dlg.setAttribute("aria-describedby", "dlg-body");
    var h = el("h2", o.title); h.id = "dlg-title";
    var b = el("p", o.body); b.id = "dlg-body";
    var btns = el("div", "", "btns");
    var no = el("button", o.cancel || "Cancel"); no.type = "button";
    var yes = el("button", o.ok || "Confirm", o.danger === false ? "primary" : "danger"); yes.type = "button";
    btns.appendChild(no); btns.appendChild(yes);
    dlg.appendChild(h); dlg.appendChild(b); dlg.appendChild(btns); back.appendChild(dlg);
    function done(v) {
      document.removeEventListener("keydown", key, true);
      if (back.parentNode) back.parentNode.removeChild(back);
      try { if (before && before.focus) before.focus(); } catch (e) {}
      resolve(v);
    }
    function key(e) {
      if (e.key === "Escape") { e.preventDefault(); done(false); }
      else if (e.key === "Tab") {
        // keep focus inside the box
        var first = no, last = yes;
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    }
    no.addEventListener("click", function () { done(false); });
    yes.addEventListener("click", function () { done(true); });
    back.addEventListener("click", function (e) { if (e.target === back) done(false); });
    document.addEventListener("keydown", key, true);
    document.body.appendChild(back);
    no.focus();
  });
}
// A card that could not load says so, with a way to try again, instead of staying silent.
function retryCard(box, text, again) {
  clear(box); box.classList.remove("hidden");
  box.appendChild(el("p", text, "bad"));
  var b = el("button", "Try again", "small"); b.type = "button";
  b.addEventListener("click", function () { b.disabled = true; again(); });
  box.appendChild(b);
}
function when(s) { if (!s) return ""; var d = new Date(s); return isNaN(d) ? s : d.toLocaleString(); }

// Long lists show one short page at a time, newest first, with Newer / Older
// buttons. Each list asks for one row more than a page so it knows whether an
// older page exists.
var PAGE = { orders: 0, deliveries: 0, audit: 0, sweeps: 0, upkeep: 0 };
var PAGE_SIZE = { orders: 15, deliveries: 15, audit: 15, sweeps: 10, upkeep: 10 };
function pageQuery(key) { return "limit=" + (PAGE_SIZE[key] + 1) + "&offset=" + (PAGE[key] * PAGE_SIZE[key]); }
// True (after asking again for page 1) when the page we were on no longer exists.
function pageGone(key, count, reload) {
  if (count === 0 && PAGE[key] > 0) { PAGE[key] = 0; reload(); return true; }
  return false;
}
// Draws the Newer / Older bar under a list (nothing when one page is all there is).
function pagerFor(box, key, count, reload) {
  var more = count > PAGE_SIZE[key];
  if (PAGE[key] === 0 && !more) return false;
  var bar = el("div", "", "pager"); bar.style.cssText = "display:flex;justify-content:space-between;align-items:center;margin-top:12px";
  var newer = el("button", "\u2190 Newer", "small"); newer.type = "button"; newer.disabled = PAGE[key] === 0;
  newer.addEventListener("click", function () { PAGE[key] = Math.max(0, PAGE[key] - 1); reload(); });
  var older = el("button", "Older \u2192", "small"); older.type = "button"; older.disabled = !more;
  older.addEventListener("click", function () { PAGE[key]++; reload(); });
  bar.appendChild(newer); bar.appendChild(el("span", "Page " + (PAGE[key] + 1), "muted")); bar.appendChild(older);
  box.appendChild(bar);
}

function api(method, path, body) {
  var spends = body !== undefined && body !== null && Object.prototype.hasOwnProperty.call(body, "code");
  return apiCall(method, path, body).then(function (j) { if (spends) resetPkInputs(); return j; }, function (e) { if (spends) resetPkInputs(); throw e; });
}
function apiCall(method, path, body) {
  var h = { "accept": "application/json" };
  if (body !== undefined) h["content-type"] = "application/json";
  if (method !== "GET") h["x-ebp-csrf"] = csrf;
  return fetch(path, { method: method, headers: h, credentials: "same-origin", body: body === undefined ? undefined : JSON.stringify(body) })
    .catch(function () { throw new Error("Can't reach your server right now. Check your internet connection and that the server is running, then try again."); })
    .then(function (r) {
      return r.text().then(function (t) {
        var j = null; try { j = t ? JSON.parse(t) : null; } catch (e) {}
        if (r.status === 401 && path.indexOf("/admin/login") !== 0) { signedOut(); throw new Error("Please sign in again."); }
        if (!r.ok) throw new Error((j && ((j.error && j.error.message) || j.message)) || (r.status === 429 ? "Too many wrong tries in a short time. Wait a few minutes, then try again." : r.status >= 500 ? "The server had a problem. Try again in a minute; if it keeps happening, see Known issues in the docs." : "The server did not accept that (error " + r.status + ")."));
        return j;
      });
    });
}

// Settings sections start folded; open the one a link or button is sending the owner to.
function openFold(id) {
  try { var e = $(id); var d = e && e.closest("details.fold"); if (d) d.open = true; } catch (x) {}
}
// Leaving Settings folds every section again, so coming back always starts tidy.
function foldSettings() {
  document.querySelectorAll("#v-set details").forEach(function (d) { d.open = false; });
  exFoldOpened = false;
}
function show(name) {
  if (name !== "set") foldSettings();
  views.forEach(function (v) { $("v-" + v).classList.toggle("hidden", v !== name); });
  var tabs = document.querySelectorAll("#tabs button");
  for (var i = 0; i < tabs.length; i++) tabs[i].classList.toggle("on", tabs[i].getAttribute("data-v") === name);
  if (name === "home") { PAGE.sweeps = 0; PAGE.upkeep = 0; loadHome(); }
  if (name === "orders") { PAGE.orders = 0; loadOrders(); }
  if (name === "alerts") loadAlerts();
  if (name === "hooks") { PAGE.deliveries = 0; loadDeliveries(); loadKeylock(); }
  // The emergency answer is fetched ahead, so the section is usually ready when it is opened.
  if (name === "set") { PAGE.audit = 0; loadAudit(); loadPayout(); loadExit(); }
  if (name === "home" || name === "set" || name === "alerts") loadPhone();
  if (name === "home" || name === "set") loadTwofa();
  if (name === "home") loadRecon();
}
function signedOut() { var n = $("login-note"); if (pwChanged) { n.textContent = "Password changed. Sign in with your new password."; n.classList.remove("hidden"); pwChanged = false; } else { n.classList.add("hidden"); } csrf = ""; twofaOn = false; twofaDeclined = false; paused = false; passkeysOn = false; loginPasskey = null; $("login-pk").classList.add("hidden"); $("code-box").classList.add("hidden"); $("code").value = ""; $("tabs").classList.add("hidden"); $("logout").classList.add("hidden"); show("login"); }
function signedIn(token) { csrf = token; $("tabs").classList.remove("hidden"); $("logout").classList.remove("hidden"); $("base").textContent = location.origin; show(location.hash === "#test" ? "orders" : "home"); if (location.hash === "#test") { try { $("tp-sats").focus(); } catch (e) {} } }

function boot() {
  fetch("/admin/session", { credentials: "same-origin" }).then(function (r) { return r.json(); }).then(function (s) {
    if (s.authenticated) { twofaOn = !!s.twofa; twofaDeclined = !!s.twofa_declined; paused = !!s.paused; passkeysOn = !!s.passkeys; return signedIn(s.csrf); }
    $("nopw").classList.toggle("hidden", !!s.password_set);
    show("login");
  }).catch(function () { show("login"); });
}

$("login-form").addEventListener("submit", function (e) {
  e.preventDefault();
  $("login-err").classList.add("hidden");
  $("login-btn").disabled = true;
  var body = { password: $("pw").value };
  if (!$("code-box").classList.contains("hidden")) body.code = $("code").value;
  api("POST", "/admin/login", body).then(function (r) {
    if (r.code_required) {
      $("code-box").classList.remove("hidden"); $("code").focus();
      loginPasskey = r.passkey || null;
      $("login-pk").classList.toggle("hidden", !(loginPasskey && passkeysCanSignIn()));
      return;
    }
    $("pw").value = ""; $("code").value = ""; $("code-box").classList.add("hidden"); $("login-pk").classList.add("hidden"); loginPasskey = null; signedIn(r.csrf);
  })
    .catch(function (e) {
      var m = $("login-err");
      m.textContent = /code/i.test(e.message) ? e.message : "That password didn't work.";
      m.classList.remove("hidden");
    })
    .then(function () { $("login-btn").disabled = false; });
});
$("logout").addEventListener("click", function () { api("POST", "/admin/logout", {}).then(signedOut).catch(signedOut); });
document.querySelectorAll("#tabs button").forEach(function (b) { b.addEventListener("click", function () { show(b.getAttribute("data-v")); }); });
document.addEventListener("click", function (e) {
  var id = e.target && e.target.getAttribute && e.target.getAttribute("data-copy");
  if (id && navigator.clipboard) navigator.clipboard.writeText($(id).textContent).then(function () { e.target.textContent = "Copied"; });
});

// ---- Updates (Home tab: the Version row and the Update card) ----
// EBP never updates itself. With the update helper installed on the server
// (scripts/install-update-helper.sh), the owner can update from here: they
// decide twice (Start update, then Update now after seeing the signature result
// and both versions), and the second decision needs the admin password and, if
// two-step sign-in is on, a 6-digit code. Every check runs in the helper.
var UPD = { busy: false, waitId: "", target: "", timer: null, started: 0 };
var UPD_REASONS = {
  older: "This version is older than the one you run. EBP never goes backwards.",
  same_version: "You already run this version.",
  signature_failed: "The signature check failed: this image was not built and signed by this project's release workflow. Do not update, and tell the project.",
  digest_changed: "The release's image changed since you checked it. Press Start update to check it again.",
  commit_mismatch: "The release's files and its image come from different commits. Do not update, and tell the project.",
  no_commit: "The signature did not say which commit built the image, so the files could not be matched to it.",
  no_digest: "Could not download this release's image address from GitHub. Check that the server has internet, then try again later.",
  bad_digest: "The release's image address is not in the expected form, so it was not used.",
  no_cosign: "The signature checker could not be set up (it needs internet). Try again later, or update from the Console.",
  fetch_failed: "Could not reach GitHub to fetch the release files. Try again later.",
  no_tag: "This release's files are not on GitHub.",
  no_running_version: "The helper could not read which version runs now, so it refused (it never skips the check against going backwards). Try again in a minute.",
  arch: "The signed image is only built for x86-64 servers. Update from the Console: run cd /opt/ebp, then ./scripts/update.sh --from-source.",
  checkout_failed: "The EBP folder could not switch to the release's files (it may have local changes). Update from the Console to see why.",
  start_failed: "The new version did not start, so the old image setting was put back. In the Console run ./scripts/start.sh, then ask for help."
};
function updReason(code) { return UPD_REASONS[code] || ("The update helper stopped (" + (code || "unknown reason") + ")."); }
function updLink(text, href) { var a = el("a", text); a.href = href; a.target = "_blank"; a.rel = "noopener noreferrer"; return a; }
function updStop() { if (UPD.timer) { clearTimeout(UPD.timer); UPD.timer = null; } }

function loadVersion() {
  api("GET", "/api/v1/admin/version").then(function (v) {
    renderVersionRow(v);
    if (!UPD.busy) renderUpdate(v);
  }).catch(function () { if (!UPD.busy) retryCard($("update"), "We couldn't check for updates.", loadVersion); });
}

function renderVersionRow(v) {
  var h = $("health");
  var r = el("div", "", "row"); r.id = "version-row";
  r.appendChild(el("span", "Version"));
  var right = el("span", v.current + (v.update_available && v.latest ? " · " : (v.check_enabled ? " · up to date" : "")));
  if (v.update_available && v.latest) {
    var a = el("a", "Update available"); a.href = "#update";
    a.addEventListener("click", function (e) { e.preventDefault(); $("update").scrollIntoView({ behavior: "smooth" }); });
    right.appendChild(a);
  }
  r.appendChild(right);
  var old = $("version-row"); if (old && old.parentNode) old.parentNode.removeChild(old);
  h.appendChild(r);
}

function renderUpdate(v) {
  var u = $("update"); clear(u);
  var st = v.helper_status || null;
  // Just updated: say so once (for a day), with the result.
  if (st && st.state === "done" && st.version === v.current && (Date.now() / 1000 - st.at) < 86400) {
    u.classList.remove("hidden");
    u.appendChild(el("strong", "Updated to version " + v.current, "ok"));
    u.appendChild(el("p", "The update finished. Check above that the wallet is connected before you take payments again.", "muted"));
    if (!v.update_available) return;
    clear(u);
  }
  if (st && st.state === "failed") {
    u.classList.remove("hidden");
    u.appendChild(el("strong", "The last update did not finish", "bad"));
    u.appendChild(el("p", updReason(st.code)));
    if (!v.update_available) return;
  }
  if (!v.update_available || !v.latest) { if (!u.firstChild) u.classList.add("hidden"); return; }
  u.classList.remove("hidden");
  var L = v.latest;
  u.appendChild(el("strong", (L.security ? "Security update available: " : "Update available: ") + "version " + L.version, L.security ? "bad" : ""));
  u.appendChild(el("p", "You run " + v.current + ". Your server never updates itself; you choose when. " + (v.open_orders > 0 ? "You have " + v.open_orders + " open order(s); it is best to wait until they finish. " : "No open orders right now, so this is a good moment. ") + "Your data is backed up automatically before an update.", "muted"));
  if (/^https:\/\/github\.com\//.test(L.url)) u.appendChild(updLink("What changed in this version", L.url));
  if (v.host === "render") {
    u.appendChild(el("p", "To update: open your Render dashboard, choose this service, then Manual Deploy → Deploy latest commit.", "muted"));
    return;
  }
  if (!v.helper_version) {
    u.appendChild(el("p", "The one-tap update is not set up on this server. Open the option below to set it up or to update from the Console.", "muted"));
    u.appendChild(updFallback(L, true));
    return;
  }
  // A finished check for this exact version: show the comparison and the second decision.
  if (st && st.state === "ready" && st.code === "ok" && st.version === L.version && st.running === v.current && st.digest) {
    renderConfirm(v, st);
    return;
  }
  if (st && st.state === "refused" && st.version === L.version) {
    u.appendChild(el("p", "The last check was refused: " + updReason(st.code) + " Nothing was changed.", "bad"));
  }
  var btns = el("div", "", "btns");
  btns.appendChild(button("Start update", function (b) { startCheck(v, b); }, "primary"));
  u.appendChild(btns);
  u.appendChild(el("p", "Start update only checks the new version first (about a minute). Nothing changes until you confirm on the next screen.", "muted"));
  u.appendChild(updFallback(L, false));
}

// The Console commands, folded away: only for when the update from this page did not work.
function updFallback(L, needHelper) {
  var d = document.createElement("details");
  d.className = "updfold";
  d.appendChild(el("summary", "Did the automatic update fail? Tap here"));
  d.appendChild(el("p", "This is the backup way, for when the update button did not work or is not there.", "muted"));
  if (needHelper) {
    d.appendChild(el("p", "Install the update helper once, so this page can update the server from now on. Open your server's Console (at your hosting company), log in as root, and run these two lines, one at a time (/opt/ebp is where the server was set up; if you chose another folder, go there instead):", "muted"));
    d.appendChild(el("div", "cd /opt/ebp", "secret"));
    d.appendChild(el("div", "./scripts/install-update-helper.sh", "secret"));
    d.appendChild(el("p", "When it says the helper is installed, refresh this admin page to see the change: the Update card will then show a Start update button.", "muted"));
  }
  d.appendChild(el("p", "To update from the Console instead, run these three lines, one at a time. The second only checks the new version's signature; the third asks you to type yes before anything changes (/opt/ebp is where the server was set up; if you chose another folder, go there instead):", "muted"));
  d.appendChild(el("div", "cd /opt/ebp", "secret"));
  d.appendChild(el("div", "./scripts/update.sh --check " + L.version, "secret"));
  d.appendChild(el("div", "./scripts/update.sh " + L.version, "secret"));
  d.appendChild(el("p", "Afterwards, refresh this admin page to see the new version.", "muted"));
  return d;
}

// First decision: ask the helper to check, then wait for its answer.
function startCheck(v, b) {
  b.disabled = true;
  var u = $("update");
  api("POST", "/admin/update/check", { version: v.latest.version }).then(function (r) {
    UPD.busy = true; UPD.waitId = r.request_id; UPD.target = v.latest.version; UPD.started = Date.now();
    clear(u);
    u.appendChild(el("strong", "Checking version " + v.latest.version));
    u.appendChild(el("p", "The helper is checking the new version's signature with GitHub and Sigstore. This takes about a minute. Nothing changes yet.", "muted"));
    u.appendChild(button("Cancel", function () { updStop(); UPD.busy = false; loadVersion(); }, "small"));
    pollCheck();
  }).catch(function (e) { b.disabled = false; u.appendChild(el("p", e.message, "bad")); });
}
function pollCheck() {
  updStop();
  UPD.timer = setTimeout(function () {
    api("GET", "/api/v1/admin/version").then(function (v) {
      var st = v.helper_status;
      if (st && st.request_id === UPD.waitId && st.state !== "checking") { UPD.busy = false; renderVersionRow(v); renderUpdate(v); return; }
      if (Date.now() - UPD.started > 4 * 60 * 1000) {
        UPD.busy = false; var u = $("update"); clear(u);
        u.appendChild(el("strong", "The update helper did not answer", "bad"));
        u.appendChild(el("p", "It may not be running. In your server's Console run: systemctl status ebp-update-helper.path. Nothing was changed.", "muted"));
        u.appendChild(button("Try again", function () { loadVersion(); }, "small"));
        return;
      }
      pollCheck();
    }).catch(function () { pollCheck(); });
  }, 3000);
}

// The comparison and the second decision.
function renderConfirm(v, st) {
  var u = $("update"); clear(u); u.classList.remove("hidden");
  u.appendChild(el("strong", "Ready to update: please check, then confirm"));
  var cmp = el("div", "", "upd-compare");
  var now = el("div"); now.appendChild(el("span", "Now", "muted")); now.appendChild(el("strong", st.running || v.current));
  var neu = el("div"); neu.appendChild(el("span", "New", "muted")); neu.appendChild(el("strong", st.version));
  cmp.appendChild(now); cmp.appendChild(el("span", "→", "muted")); cmp.appendChild(neu);
  u.appendChild(cmp);
  u.appendChild(el("p", "✓ Signature OK: built and signed by this project's release workflow for v" + st.version + (st.commit ? ", from commit " + st.commit.slice(0, 12) : "") + ".", "ok"));
  u.appendChild(el("p", "Image (digest):", "muted"));
  u.appendChild(el("div", st.digest, "secret"));
  var repo = v.repo_url || "https://github.com/reya01/easy_bitcoin_payments";
  var links = el("p", "Inspect it further on GitHub: ", "muted");
  links.appendChild(updLink("the release page", (v.latest && /^https:\/\/github\.com\//.test(v.latest.url)) ? v.latest.url : repo + "/releases/tag/v" + st.version));
  links.appendChild(document.createTextNode(" (compare the digest and read the notes), and "));
  links.appendChild(st.run_url ? updLink("the workflow run that built and signed it", st.run_url) : updLink("the commit and its checks", repo + "/commit/" + st.commit));
  links.appendChild(document.createTextNode("."));
  u.appendChild(links);
  var bk = el("p", "Download a fresh backup first. ", "muted");
  var ba = el("a", "Go to Download a backup"); ba.href = "#";
  ba.addEventListener("click", function (e) { e.preventDefault(); show("set"); openFold("fold-backup"); });
  bk.appendChild(ba); u.appendChild(bk);
  u.appendChild(field("upd-pw", "Your admin password", "password"));
  var cw = el("div"); cw.className = "needcode" + (twofaOn ? "" : " hidden");
  cw.appendChild(field("upd-code", "Code from your authenticator app", "tel", "6 digits"));
  u.appendChild(cw);
  var msg = el("p", "", "bad hidden");
  var btns = el("div", "", "btns");
  btns.appendChild(button("Update now", function (b) {
    b.disabled = true; msg.classList.add("hidden");
    var body = { version: st.version, digest: st.digest, password: $("upd-pw").value };
    if (twofaOn) body.code = $("upd-code").value.trim();
    api("POST", "/admin/update/apply", body).then(function () {
      UPD.busy = true; UPD.target = st.version; UPD.started = Date.now();
      renderUpdating();
    }).catch(function (e) { b.disabled = false; msg.textContent = e.message; msg.classList.remove("hidden"); });
  }, "primary"));
  btns.appendChild(button("Cancel", function () { clear(u); u.classList.add("hidden"); }, "small"));
  u.appendChild(btns); u.appendChild(msg);
}

// Second decision made: the helper checks once more, then switches. EBP restarts.
function renderUpdating() {
  var u = $("update"); clear(u);
  u.appendChild(el("strong", "Updating to version " + UPD.target));
  u.appendChild(el("p", "The helper checks the release once more, then switches the server. The page reconnects by itself in about a minute; keep it open.", "muted"));
  pollApply();
}
function pollApply() {
  updStop();
  UPD.timer = setTimeout(function () {
    fetch("/api/v1/admin/version", { credentials: "same-origin", headers: { accept: "application/json" } })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (v) {
        var st = v && v.helper_status;
        if (v && v.current === UPD.target) {
          updStop(); UPD.busy = false;
          var u = $("update"); clear(u);
          u.appendChild(el("strong", "Updated to version " + UPD.target, "ok"));
          u.appendChild(el("p", "Done. This page now reloads to show the new version. If it asks you to sign in, sign in again.", "muted"));
          setTimeout(function () { location.reload(); }, 3000);
          return;
        }
        if (st && (st.state === "refused" || st.state === "failed") && st.version === UPD.target) {
          updStop(); UPD.busy = false;
          var u2 = $("update"); clear(u2);
          u2.appendChild(el("strong", st.state === "refused" ? "Update refused" : "The update did not finish", "bad"));
          u2.appendChild(el("p", updReason(st.code) + (st.state === "refused" ? " Nothing was changed." : "")));
          u2.appendChild(button("Back", function () { loadVersion(); }, "small"));
          return;
        }
        if (Date.now() - UPD.started > 6 * 60 * 1000) {
          updStop(); UPD.busy = false;
          var u3 = $("update"); clear(u3);
          u3.appendChild(el("strong", "This is taking longer than expected", "bad"));
          u3.appendChild(el("p", "Reload this page in a minute. If Home does not show the new version, in your server's Console look at: cat updates/status/last-run.log", "muted"));
          return;
        }
        pollApply();
      })
      .catch(function () { pollApply(); });
  }, 3000);
}

// ---- Weekly check against your own wallet ----
// Dollars per bitcoin, from the server (null = unknown or turned off: then only sats are shown).
var USD = null;
// "1,000 sats ($0.40)". Same wording rules as the checkout page and the server's messages.
function fmtSats(n) {
  n = Number(n);
  var out = n.toLocaleString("en-US") + " sats";
  if (!(USD > 0) || !isFinite(n)) return out;
  var usd = Math.abs(n) / 1e8 * USD;
  var txt = usd > 0 && usd < 0.01 ? "<$0.01" : "$" + usd.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return out + " (" + (n < 0 ? "-" : "") + txt + ")";
}
function loadRecon() {
  api("POST", "/admin/reconcile/status", {}).then(function (r) {
    var box = $("recon"); clear(box);
    if (!r.paid_orders && !r.swept_sats) { box.classList.add("hidden"); return; }
    box.classList.remove("hidden");
    // Checked within the last week: fold it away to one line so it does not take
    // up room on the Home page. It opens out again on a tap, and by itself when due.
    var host = box;
    if (r.due) {
      box.appendChild(el("strong", "Time for your weekly wallet check", "bad"));
    } else {
      host = document.createElement("details");
      host.style.margin = "0";
      host.appendChild(el("summary", r.last_checked_at
        ? "Weekly wallet check: done " + when(r.last_checked_at * 1000) + ". Tap to check again."
        : "Weekly wallet check: your first one is due " + (r.next_due_at ? "on " + new Date(r.next_due_at * 1000).toLocaleDateString(undefined, { month: "long", day: "numeric" }) : "in about a week") + ". Tap to check now."));
      box.appendChild(host);
    }
    var days = r.days + " days";
    if (r.swept_sats > 0) {
      host.appendChild(el("p", "Open your own wallet. Does it show about " + fmtSats(r.swept_sats) + " received from this server in the last " + days + "?"));
    } else {
      host.appendChild(el("p", "This server says it has not sent anything on to your wallet in the last " + days + ". Open your own wallet and check that nothing arrived from it either."));
    }
    host.appendChild(el("p", "This is the one check that does not depend on this server telling the truth. Payments are sent on in batches, so the amount in your wallet can lag a little behind your orders.", "muted"));
    host.appendChild(el("p", "For reference: in the last " + days + " this server marked " + r.paid_orders + " order" + (r.paid_orders === 1 ? "" : "s") + " paid (" + fmtSats(r.paid_sats) + ").", "muted"));
    if (r.last_checked_at) host.appendChild(el("p", "Last checked " + when(r.last_checked_at * 1000) + ".", "muted"));
    var btns = el("div", "", "btns");
    var ok = el("button", "I checked: it matches"); ok.type = "button"; ok.className = r.due ? "primary" : "";
    ok.addEventListener("click", function () { ok.disabled = true; api("POST", "/admin/reconcile/done", {}).then(loadRecon).catch(function () { ok.disabled = false; }); });
    btns.appendChild(ok);
    host.appendChild(btns);
    var d = document.createElement("details");
    d.appendChild(el("summary", "It doesn't match"));
    d.appendChild(el("p", "1. In Settings, press \"Pause new payments\". Then in your shop, turn off the Bitcoin payment method (WooCommerce: Settings, Payments, Bitcoin, untick Enable).", "muted"));
    d.appendChild(el("p", "2. Do not delete anything on the server; it may help work out what happened.", "muted"));
    var p3 = el("p", "3. Turn the server off at your hosting company, then follow ", "muted");
    p3.appendChild(docLink("wrong", "\"If something looks wrong\""));
    p3.appendChild(document.createTextNode(" before you do anything else. Never type your recovery words anywhere while fixing this."));
    d.appendChild(p3);
    host.appendChild(d);
  }).catch(function () { retryCard($("recon"), "We couldn't check your backup.", loadRecon); });
}

// ---- Phone alerts (optional, via the free ntfy app) ----
function phoneBtn(label, fn, plain) {
  var b = el("button", label); b.type = "button"; if (plain) b.className = "plain";
  b.addEventListener("click", function () { b.disabled = true; fn(b); });
  return b;
}
function phoneAct(path, body, done) {
  api("POST", path, body || {}).then(function (r) { done(null, r); }).catch(function (e) { done(e); });
}
function renderPhone(box, a, onHome) {
  clear(box);
  if (a.state === "undecided") {
    box.appendChild(el("strong", "Get alerts on your phone?"));
    box.appendChild(el("p", "If something needs you (the payment wallet is unreachable, a payment can't be matched to an order, a withdrawal fails) we can send a short message to your phone. Messages never contain amounts or addresses. It's free and takes about a minute.", "muted"));
    var btns = el("div", "", "btns");
    // Home only asks the question. The steps (topic name, test alert) live in
    // Settings, so after "Set up" we take the owner there instead of letting the card vanish.
    var hmsg = el("p", "", "msg hidden"); hmsg.id = "phone-home-msg";
    var both = [];
    function hfail(e) {
      both.forEach(function (x) { x.disabled = false; });
      hmsg.className = "msg bad"; hmsg.textContent = (e && e.message) ? e.message : "That didn't work. Please try again.";
    }
    var yes = phoneBtn("Set up phone alerts", function () {
      both.forEach(function (x) { x.disabled = true; });
      phoneAct("/admin/alerts/enable", {}, function (e) {
        if (e) return hfail(e);
        loadPhone(true);
      });
    });
    var no = phoneBtn("No thanks", function () {
      both.forEach(function (x) { x.disabled = true; });
      phoneAct("/admin/alerts/disable", {}, function (e) { if (e) return hfail(e); loadPhone(); });
    }, true);
    both.push(yes, no);
    btns.appendChild(yes); btns.appendChild(no);
    box.appendChild(btns);
    box.appendChild(hmsg);
    return;
  }
  if (onHome) return;
  if (a.state === "off") {
    box.appendChild(el("p", "Phone alerts are off. You will only see problems when you open this dashboard.", "muted"));
    var b2 = el("div", "", "btns");
    b2.appendChild(phoneBtn("Turn phone alerts on", function () { phoneAct("/admin/alerts/enable", {}, function () { loadPhone(); }); }));
    box.appendChild(b2);
    return;
  }
  var host = a.server.replace(/^https?:\/\//, "");
  box.appendChild(el("p", "1. On your phone, install the free \"ntfy\" app (App Store or Google Play).", "muted"));
  box.appendChild(el("p", "2. Open it, tap +, and type this topic name exactly" + (a.server !== a.default_server ? " (and use the server " + host + ")" : "") + ":", "muted"));
  var t = el("div", a.topic, "secret"); t.id = "phone-topic"; box.appendChild(t);
  var c = el("button", "Copy"); c.type = "button"; c.setAttribute("data-copy", "phone-topic"); box.appendChild(c);
  box.appendChild(el("p", "Keep this topic private: anyone who knows it could read your alerts.", "muted"));
  var link = "ntfy://" + host + "/" + a.topic;
  var q = el("p", "On the same phone as the ntfy app, you can instead tap: ", "muted");
  var ln = el("a", "open in ntfy"); ln.href = link; q.appendChild(ln);
  box.appendChild(q);
  var msg = el("p", "", "msg hidden"); msg.id = "phone-msg";
  var row2 = el("div", "", "btns");
  row2.appendChild(phoneBtn("Send a test alert", function (b) {
    phoneAct("/admin/alerts/test", {}, function (e) {
      b.disabled = false; msg.classList.remove("hidden");
      msg.className = "msg " + (e ? "bad" : "ok");
      msg.textContent = e ? e.message : "Sent. It should appear on your phone in a few seconds.";
    });
  }));
  // Turning working alerts off, or using an own server, asks for a code, but only
  // while two-step sign-in is on (the box is hidden otherwise, like the others).
  var cw = field("phone-code", "Code from your authenticator app (needed to turn alerts off or to use your own server)", "tel", "6 digits");
  cw.className = "needcode" + (twofaOn ? "" : " hidden");
  function phoneCode() { var i = $("phone-code"); return i ? i.value : ""; }
  box.appendChild(cw);
  row2.appendChild(phoneBtn("Turn phone alerts off", function (b) {
    phoneAct("/admin/alerts/disable", { code: phoneCode() }, function (e) {
      if (e) { b.disabled = false; msg.classList.remove("hidden"); msg.className = "msg bad"; msg.textContent = e.message; } else loadPhone();
    });
  }, true));
  box.appendChild(row2);
  box.appendChild(msg);
  var al = document.createElement("label"); al.textContent = "Tell me about payments"; al.setAttribute("for", "phone-activity"); box.appendChild(al);
  var sel = document.createElement("select"); sel.id = "phone-activity";
  [["daily", "Once a day (recommended)"], ["each", "Every payment"], ["off", "Not at all"]].forEach(function (o) {
    var op = document.createElement("option"); op.value = o[0]; op.textContent = o[1]; if (a.activity === o[0]) op.selected = true; sel.appendChild(op);
  });
  sel.addEventListener("change", function () {
    phoneAct("/admin/alerts/activity", { mode: sel.value }, function (e) {
      msg.classList.remove("hidden"); msg.className = "msg " + (e ? "bad" : "ok");
      msg.textContent = e ? e.message : "Saved. These messages only say how many payments arrived, never amounts.";
    });
  });
  box.appendChild(sel);
  box.appendChild(el("p", "Once a day means one short message, and only on days with payments. It reminds you to compare with your own wallet. Messages never contain amounts or addresses.", "muted"));
  var d = document.createElement("details");
  d.appendChild(el("summary", "Use my own ntfy server (advanced)"));
  var inp = document.createElement("input"); inp.type = "text"; inp.id = "phone-server"; inp.placeholder = "https://ntfy.example.com";
  d.appendChild(inp);
  var sv = phoneBtn("Save server", function (b) {
    phoneAct("/admin/alerts/enable", { server: inp.value, code: phoneCode() }, function (e) {
      b.disabled = false;
      if (e) { msg.classList.remove("hidden"); msg.className = "msg bad"; msg.textContent = e.message; } else loadPhone();
    });
  }, true);
  d.appendChild(sv);
  box.appendChild(d);
}
function loadPhone(openSteps) {
  api("POST", "/admin/alerts/status", {}).then(function (a) {
    var h = $("phone-home"), s = $("phone-set");
    renderPhone(h, a, true);
    h.classList.toggle("hidden", a.state !== "undecided");
    renderPhone(s, a, false);
    if (openSteps && a.state === "on") {
      show("alerts"); openFold("phone-set");
      try { s.scrollIntoView({ block: "start" }); } catch (e) {}
    }
  }).catch(function () {
    retryCard($("phone-home"), "We couldn't check your alert settings.", loadPhone);
    retryCard($("phone-set"), "We couldn't check your alert settings.", loadPhone);
  });
}

var pwChanged = false;
var EBP_MODE = "";
var KIND = { sweep: "Transfer to your wallet", sweep_ark: "Transfer to your wallet (Ark)", sweep_onchain: "Transfer to your wallet (on-chain)", sweep_refused: "Transfer blocked: payout address problem", renew: "Routine wallet upkeep", consolidate: "Tidy-up", offboard: "Withdrawal" };
// Optional first-payment check on Home: shown until one payment has been paid
// or the owner skips it. Never blocks anything. "Skip" is remembered only in this browser.
var TEST_SKIP_KEY = "ebp_test_payment_skipped";
function testSkipped() { try { return localStorage.getItem(TEST_SKIP_KEY) === "1"; } catch (e) { return false; } }
function loadTestPrompt() {
  var c = $("test-prompt");
  if (testSkipped()) { c.classList.add("hidden"); return; }
  Promise.all([api("GET", "/api/v1/invoices?status=paid&limit=1"), api("GET", "/api/v1/invoices?status=overpaid&limit=1")]).then(function (r) {
    clear(c);
    if (r[0].length || r[1].length) { c.classList.add("hidden"); return; }
    c.appendChild(el("h3", "Test payment: not done yet (optional)"));
    c.appendChild(el("p", "Pay yourself 1,000 sats over Lightning to check that your server really receives money. It takes about a minute. You need a Lightning wallet with a few spare sats for the network fee.", "muted"));
    c.appendChild(el("p", EBP_MODE === "live" ? "This server uses real money. The 1,000 sats are a real payment: they land on your server like any other sale and are paid out to your own wallet later." : "This is a test server, so the coins are play money.", "muted"));
    var b = el("div", "", "btns");
    var go = el("button", "Try a test payment", "primary"); go.type = "button";
    go.addEventListener("click", function () { show("orders"); try { $("tp-sats").value = "1000"; tpPreview(); $("tp-sats").focus(); } catch (e) {} });
    var sk = el("button", "Skip for now", ""); sk.type = "button";
    sk.addEventListener("click", function () { try { localStorage.setItem(TEST_SKIP_KEY, "1"); } catch (e) {} c.classList.add("hidden"); });
    b.appendChild(go); b.appendChild(sk); c.appendChild(b);
    c.classList.remove("hidden");
  }).catch(function () { c.classList.add("hidden"); });
}

function loadHome() {
  api("GET", "/api/v1/admin/status").then(function (s) {
    var h = $("health"); clear(h);
    var mb = $("mode-banner");
    EBP_MODE = s.mode || "";
    if (s.mode === "live") { mb.textContent = "REAL MONEY. Experimental: keep amounts small."; mb.classList.remove("hidden"); }
    else if (s.mode === "test") { mb.textContent = "TEST MODE: play money (signet). Nothing here is real."; mb.classList.remove("hidden"); }
    loadVersion();
    USD = s.usd_per_btc > 0 ? Number(s.usd_per_btc) : null;
    tpPreview();
    loadTestPrompt();
    var w = s.wallet || {};
    NEXT_ROUND_AT = w.next_round_at || null;
    var go = { late: "alerts", webhooks: "hooks", reserve: "set" };
    var sym = { ok: "\u2713 ", warn: "! ", bad: "\u2715 " };
    (s.checks || []).forEach(function (k) {
      var d = el("div", "", "check");
      d.appendChild(el("div", (sym[k.level] || "") + k.title, k.level));
      if (k.level !== "ok") d.appendChild(el("div", k.detail, "muted"));
      if (go[k.id] && k.level !== "ok") { d.style.cursor = "pointer"; d.addEventListener("click", function () { show(go[k.id]); }); }
      if (k.id === "transfer_unknown") {
        // The server could not tell whether a transfer went out. The owner checks their own
        // wallet and says which; nothing is sent until then, so nothing can be sent twice.
        var tb = el("div", "", "");
        [["arrived", "It arrived"], ["not_arrived", "It did not arrive"]].forEach(function (o) {
          var b = el("button", o[1], "small"); b.type = "button"; b.style.marginTop = "8px"; b.style.marginRight = "8px";
          b.addEventListener("click", function () {
            askConfirm({ title: o[1] + "?", body: o[0] === "arrived" ? "You checked your payout wallet and the money is there." : "You checked your payout wallet and the money is not there. The server will then send it again with the next transfer, so be sure: if it did arrive, it would be sent twice.", cancel: "Not yet", ok: "Yes", danger: false }).then(function (yes) {
              if (!yes) return;
              b.disabled = true;
              api("POST", "/admin/transfer-outcome", { outcome: o[0] }).then(loadHome).catch(function (e) { b.textContent = e.message; b.disabled = false; });
            });
          });
          tb.appendChild(b);
        });
        d.appendChild(tb);
      }
      h.appendChild(d);
    });
    if (w.offchain_sats !== undefined) {
      var transit = Number(w.in_transit_sats || 0);
      h.appendChild(row("Held on your server", fmtSats(Number(w.offchain_sats) + transit)));
      if (transit > 0) h.appendChild(el("div", fmtSats(transit) + " of this is still arriving or being renewed. It is yours; it just cannot be moved for a few minutes.", "muted"));
    }
    var c = s.counters || {}, a = $("attention"); clear(a);
    var n = 0;
    function add(label, count, view) { if (count > 0) { n++; var r = row(label, String(count), "bad"); r.style.cursor = "pointer"; r.addEventListener("click", function () { show(view); }); a.appendChild(r); } }
    add("Payments that need you", c.unmatched_payments_open || 0, "alerts");
    add("Notifications failing to reach your shop", c.webhooks_failed || 0, "hooks");
    if (!n) a.appendChild(el("span", "Nothing needs your attention.", "ok"));
    a.appendChild(row("Open orders", String(c.invoices_pending || 0)));
    var b = $("alert-badge"); var open = c.unmatched_payments_open || 0; b.textContent = String(open); b.classList.toggle("hidden", !open);
    loadSettlementLists();
  }).catch(function (e) { var h = $("health"); clear(h); h.appendChild(el("span", e.message, "bad")); });
}

// When the Ark server's next round starts (set by loadHome while a renewal is waiting for one).
var NEXT_ROUND_AT = null;
function nextRoundText() {
  var t = NEXT_ROUND_AT ? Date.parse(NEXT_ROUND_AT) : NaN;
  if (isNaN(t)) return "";
  var mins = Math.ceil((t - Date.now()) / 60000);
  if (mins <= 0) return "Next round: starting any moment now";
  if (mins === 1) return "Next round in: 1 minute";
  if (mins < 60) return "Next round in: " + mins + " minutes";
  var h = Math.floor(mins / 60), m = mins % 60;
  return "Next round in: " + h + (h === 1 ? " hour" : " hours") + (m ? " " + m + (m === 1 ? " minute" : " minutes") : "");
}
var settleLine = function (x) {
  var r = row(when(x.created_at) + " \u00b7 " + (KIND[x.kind] || x.kind), fmtSats(x.amount_sats) + " \u00b7 " + x.status);
  if (x.kind === "renew" && (x.status === "requested" || x.status === "waiting") && nextRoundText()) {
    var wr = document.createElement("div");
    wr.appendChild(r); wr.appendChild(el("div", nextRoundText() + ". The Ark server renews coins in rounds, and on real money they are about an hour apart. Nothing is wrong while this waits.", "muted"));
    return wr;
  }
  if ((x.status !== "failed" && x.status !== "unknown") || !x.detail) return r;
  // Say WHY it failed, in the server's own words, right under the line.
  var w = document.createElement("div");
  w.appendChild(r); w.appendChild(el("div", "Why: " + x.detail, "muted"));
  return w;
};
function loadSettlementList(group, key, boxId) {
  api("GET", "/api/v1/admin/settlements?group=" + group + "&" + pageQuery(key)).then(function (list) {
    var box = $(boxId); clear(box);
    var again = function () { loadSettlementList(group, key, boxId); };
    if (pageGone(key, list.length, again)) return;
    list.slice(0, PAGE_SIZE[key]).forEach(function (x) { box.appendChild(settleLine(x)); });
    if (!list.length) box.appendChild(el("span", "None yet.", "muted"));
    pagerFor(box, key, list.length, again);
  }).catch(function (e) { $(boxId).textContent = e.message; });
}
function loadSettlementLists() {
  loadSettlementList("transfers", "sweeps", "sweeps");
  loadSettlementList("upkeep", "upkeep", "upkeep");
}

$("tp-go").addEventListener("click", function () {
  var sats = parseInt($("tp-sats").value, 10);
  if (!(sats > 0)) { say("tp-msg", "Enter a whole number of sats, like 1000.", true); return; }
  var b = $("tp-go"); b.disabled = true; $("tp-link-box").classList.add("hidden");
  api("POST", "/api/v1/invoices", { sats: sats, metadata: { source: "admin test payment" } }).then(function (inv) {
    $("tp-link").href = inv.checkout_url; $("tp-link-box").classList.remove("hidden");
    say("tp-msg", "Created. Open the checkout page and pay it from your wallet; it will show up below once paid.");
    loadOrders();
  }).catch(function (e) { say("tp-msg", e.message, true); }).then(function () { b.disabled = false; });
});

// Under the test-payment amount: what it is worth in dollars (only when a price is known).
function tpPreview() {
  var n = parseInt($("tp-sats").value, 10);
  $("tp-usd").textContent = n > 0 && USD > 0 ? "That is " + fmtSats(n) + "." : "";
}
$("tp-sats").addEventListener("input", tpPreview);

var smallCoin = 1300, minFloor = 1500, minKind = "ark", minTimer = null, exitMin = 50000, reserveFloor = 5000;
function plainSats(n) { return Number(n).toLocaleString("en-US") + " sats"; }

// The smallest amount that is sent on. Each kind of payout address has a hard
// floor; under it the page explains why (the server refuses it too).
function floorWhy(kind, n) {
  var fl = plainSats(minFloor);
  if (kind === "onchain") return plainSats(n) + " is lower than the smallest amount we allow for on-chain sends, which is " + fl + ". Every on-chain withdrawal pays a network fee that can be high and changes a lot from day to day, so on a small amount the fee could take a big part of it. Please type " + fl + " or more (we recommend much more). If you would rather send small amounts often, use an Ark payout address: it is almost free.";
  return plainSats(n) + " is lower than the smallest amount we allow, which is " + fl + ". Very small coins (under about " + plainSats(smallCoin) + ") can get stuck and fail to send for a while when the Ark server updates its software. So the server waits until at least " + fl + " has built up, then sends all of it in one go. Please type " + fl + " or more.";
}
function minWarn(n) {
  if (!(n > 0)) return "";
  if (n < minFloor) return floorWhy(minKind, n);
  if (minKind === "onchain" && n < minFloor * 4) return "That is low: each on-chain withdrawal pays a network fee that can be high and changes a lot. We recommend much more.";
  return "";
}
function minPreview() {
  var n = parseInt($("min-new").value, 10);
  $("min-usd").textContent = minWarn(n);
}
$("min-new").addEventListener("input", function () { clearTimeout(minTimer); $("min-usd").textContent = ""; minTimer = setTimeout(minPreview, 700); });
// (No preview on "change", for the same reason as the box above.)

function loadOrders() {
  api("GET", "/api/v1/invoices?" + pageQuery("orders")).then(function (list) {
    if (pageGone("orders", list.length, loadOrders)) return;
    var o = $("orders"); clear(o);
    list.slice(0, PAGE_SIZE.orders).forEach(function (i) {
      var label = (i.fiat_amount ? i.fiat_currency + " " + i.fiat_amount : fmtSats(i.amount_sats));
      o.appendChild(row(label + " \u00b7 " + when(i.created_at), i.status, i.status === "paid" || i.status === "overpaid" ? "ok" : ""));
    });
    if (!list.length) o.appendChild(el("span", "No payments yet.", "muted"));
    pagerFor(o, "orders", list.length, loadOrders);
  }).catch(function (e) { $("orders").textContent = e.message; });
}

// Both CSV buttons: fetch the file with the session, then save it under a dated name.
function downloadCsv(btnId, msgId, url, prefix) {
  var b = $(btnId); b.disabled = true; $(msgId).className = "msg hidden";
  fetch(url, { credentials: "same-origin" })
    .catch(function () { throw new Error("Can't reach your server right now. Try again in a minute."); })
    .then(function (r) {
      if (r.status === 401) { signedOut(); throw new Error("Please sign in again."); }
      if (!r.ok) throw new Error("The server could not make the file (error " + r.status + ").");
      return r.blob();
    })
    .then(function (blob) {
      var a = document.createElement("a"), url = URL.createObjectURL(blob), d = new Date();
      var pad = function (n) { return (n < 10 ? "0" : "") + n; };
      a.href = url; a.download = prefix + d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate()) + ".csv";
      document.body.appendChild(a); a.click(); document.body.removeChild(a);
      setTimeout(function () { URL.revokeObjectURL(url); }, 10000);
      say(msgId, "Your file is downloading.");
    })
    .catch(function (e) { say(msgId, e.message, true); })
    .then(function () { b.disabled = false; });
}
$("csv-go").addEventListener("click", function () { downloadCsv("csv-go", "csv-msg", "/api/v1/invoices.csv", "ebp-payments-"); });
$("tcsv-go").addEventListener("click", function () { downloadCsv("tcsv-go", "tcsv-msg", "/api/v1/admin/settlements.csv", "ebp-transfers-"); });

var ALERT_WHY = {
  invoice_closed: "money arrived for an order that was already paid or closed (the customer may have paid twice) \u2014 refund the extra or handle it by hand",
  invoice_expired: "money arrived after the payment window closed \u2014 fulfil the order by hand or refund",
  paid_after_expiry: "money arrived after the payment window closed \u2014 fulfil the order by hand or refund",
  topup_window_closed: "a top-up arrived too late to count toward the order \u2014 refund it or fulfil by hand",
  could_not_be_recorded: "money arrived for this order but could not be counted toward it, so the order was not marked paid. The money is in your wallet. Fulfil the order by hand or refund it",
  onchain_rail_disabled: "money arrived by regular (on-chain) Bitcoin, which this shop does not accept. It was not counted toward any order. Refund it or handle it by hand"
};

function loadAlerts() {
  api("GET", "/api/v1/unmatched-payments").then(function (list) {
    var u = $("unmatched"); clear(u);
    list.forEach(function (p) {
      var c = el("div", "", "card");
      c.appendChild(row(fmtSats(p.amount_sats) + " · " + ({ ark: "Ark", onchain: "regular Bitcoin", lightning: "Lightning" }[p.rail] || p.rail), p.resolved_at ? "handled" : "open", p.resolved_at ? "ok" : "bad"));
      c.appendChild(el("div", "Why: " + (ALERT_WHY[p.reason] || String(p.reason).replace(/_/g, " ")) + " · " + when(p.detected_at) + (p.order_id ? " · order " + p.order_id : ""), "muted"));
      if (!p.resolved_at) {
        var b = el("button", "Mark as handled", "small"); b.type = "button"; b.style.marginTop = "8px";
        b.addEventListener("click", function () { b.disabled = true; api("POST", "/api/v1/unmatched-payments/" + encodeURIComponent(p.id) + "/resolve", {}).then(loadAlerts).catch(function (e) { b.textContent = e.message; }); });
        c.appendChild(b);
      }
      u.appendChild(c);
    });
    if (!list.length) u.appendChild(el("div", "Nothing here — good.", "card muted"));
  }).catch(function (e) { $("unmatched").textContent = e.message; });
}

function loadDeliveries() {
  api("GET", "/api/v1/webhook-deliveries?" + pageQuery("deliveries")).then(function (list) {
    if (pageGone("deliveries", list.length, loadDeliveries)) return;
    var d = $("deliveries"); clear(d);
    list.slice(0, PAGE_SIZE.deliveries).forEach(function (x) {
      var r = row(x.event + " · " + when(x.created_at), x.status, x.status === "delivered" ? "ok" : x.status === "failed" ? "bad" : "");
      if (x.status === "failed" || x.status === "delivered") {
        var b = el("button", "Resend", "small"); b.type = "button"; b.style.marginLeft = "8px";
        b.addEventListener("click", function () {
          b.disabled = true;
          api("POST", "/api/v1/webhook-deliveries/" + encodeURIComponent(x.id) + "/resend", {}).then(function () {
            say("deliveries-msg", "Queued. It will be sent to your shop again within a few seconds.");
            loadDeliveries();
          }).catch(function (e) { b.disabled = false; say("deliveries-msg", "Could not queue it: " + e.message, true); });
        });
        r.appendChild(b);
      }
      d.appendChild(r);
    });
    if (!list.length) d.appendChild(el("span", "No notifications sent yet.", "muted"));
    pagerFor(d, "deliveries", list.length, loadDeliveries);
  }).catch(function (e) { $("deliveries").textContent = e.message; });
}

function hook(body, doneMsg) {
  api("PUT", "/api/v1/stores/webhook", body).then(function (r) {
    say("hook-msg", doneMsg);
    var box = $("hook-secret-box");
    if (r && r.webhook_secret) { $("hook-secret").textContent = r.webhook_secret; box.classList.remove("hidden"); } else box.classList.add("hidden");
  }).catch(function (e) { say("hook-msg", e.message, true); });
}
$("hook-save").addEventListener("click", function () { var u = $("hook-url").value.trim(); if (!u) return say("hook-msg", "Enter your shop's webhook address first.", true); hook({ url: u }, "Saved."); });
$("hook-rotate").addEventListener("click", function () {
  var u = $("hook-url").value.trim(); if (!u) return say("hook-msg", "Enter the webhook address first.", true);
  askConfirm({ title: "Make a new signing secret?", body: "The current secret stops working immediately. Your WooCommerce site (a different website from this admin page) will not hear about payments until the new secret is pasted into its Bitcoin settings.", cancel: "Keep current", ok: "Make new secret" })
    .then(function (yes) { if (yes) hook({ url: u, rotate_secret: true }, "New secret made. Update your shop with it."); });
});
$("hook-off").addEventListener("click", function () {
  askConfirm({ title: "Turn off payment notifications?", body: "This payment server will stop telling your WooCommerce site (a different website from this admin page) when a payment arrives, so paid orders there will stay unpaid until you save the address again.", cancel: "Keep on", ok: "Turn off" })
    .then(function (yes) { if (yes) hook({ url: null }, "Notifications turned off."); });
});

$("hook-test").addEventListener("click", function () {
  var btn = $("hook-test"); btn.disabled = true;
  say("test-msg", "Sending…");
  api("POST", "/api/v1/stores/webhook/test", {}).then(function () {
    var tries = 0;
    (function check() {
      api("GET", "/api/v1/webhook-deliveries?limit=10").then(function (list) {
        var t = null;
        for (var i = 0; i < list.length; i++) if (list[i].event === "webhook.test") { t = list[i]; break; }
        if (t && t.status === "delivered") { say("test-msg", "Your shop received the test. You're connected."); btn.disabled = false; loadDeliveries(); return; }
        if (t && t.status === "failed") { say("test-msg", "Your shop did not accept it" + (t.last_error ? " (" + String(t.last_error).slice(0, 120) + ")" : "") + ". Check the address and the signing secret.", true); btn.disabled = false; loadDeliveries(); return; }
        if (++tries > 15) { say("test-msg", "No answer yet. It keeps retrying; see the delivery log below.", true); btn.disabled = false; loadDeliveries(); return; }
        if (t && t.attempts > 0 && t.last_error) say("test-msg", "Not there yet, retrying… (" + String(t.last_error).slice(0, 80) + ")", true);
        setTimeout(check, 2000);
      }).catch(function (e) { say("test-msg", e.message, true); btn.disabled = false; });
    })();
  }).catch(function (e) { say("test-msg", e.message, true); btn.disabled = false; });
});

$("rotate").addEventListener("click", function () {
  askConfirm({ title: "Make a new API key?", body: "The current key stops working immediately, so your WooCommerce site (a different website from this admin page) cannot take payments until the new key is pasted into its Bitcoin settings. The new key is shown only once.", cancel: "Keep current", ok: "Make new key" }).then(function (yes) {
    if (!yes) return;
    api("POST", "/api/v1/stores/rotate-key", {}).then(function (r) {
      $("newkey").textContent = r.api_key || ""; $("key-box").classList.remove("hidden"); say("rotate-msg", "Done. Copy the key now — it can't be shown again.");
    }).catch(function (e) { say("rotate-msg", e.message, true); });
  });
});

$("pw-save").addEventListener("click", function () {
  api("POST", "/admin/password", { current: $("pw-cur").value, "new": $("pw-new").value, code: $("pw-code").value }).then(function () {
    $("pw-cur").value = ""; $("pw-new").value = ""; pwChanged = true; signedOut();
  }).catch(function (e) { say("pw-msg", e.message, true); });
});

$("bk-go").addEventListener("click", function () {
  var btn = $("bk-go"); btn.disabled = true; say("bk-msg", "Preparing your backup… this can take a few seconds.");
  var included = false;
  fetch("/admin/backup", { method: "POST", credentials: "same-origin",
    headers: { "content-type": "application/json", "x-ebp-csrf": csrf },
    body: JSON.stringify({ admin_password: $("bk-admin").value, code: $("bk-code").value }) })
    .then(function (r) {
      if (!r.ok) return r.text().then(function (t) {
        var j = null; try { j = JSON.parse(t); } catch (e) {}
        if (r.status === 401) { signedOut(); throw new Error("Please sign in again."); }
        throw new Error((j && j.error && j.error.message) || (j && j.message) || ("Backup failed (" + r.status + ")"));
      });
      included = r.headers.get("x-ebp-wallet-included") === "true";
      var cd = r.headers.get("content-disposition") || "";
      var m = /filename="([^"]+)"/.exec(cd);
      return r.blob().then(function (b) { return { blob: b, name: m ? m[1] : "ebp-backup.ebpbackup" }; });
    })
    .then(function (f) {
      var a = document.createElement("a");
      a.href = URL.createObjectURL(f.blob); a.download = f.name;
      document.body.appendChild(a); a.click(); a.remove();
      setTimeout(function () { URL.revokeObjectURL(a.href); }, 10000);
      $("bk-admin").value = ""; $("bk-code").value = "";
      say("bk-msg", "Saved as " + f.name + ". " + (included ? "It includes your wallet files." : "Your wallet runs outside this app, so its files are NOT inside; your recovery words are its backup.") + " Keep it somewhere safe, away from this server.");
    })
    .catch(function (e) { say("bk-msg", e.message, true); })
    .then(function () { btn.disabled = false; resetPkInputs(); });
});

$("ck-go").addEventListener("click", function () {
  var f = $("ck-file").files[0], pw = $("ck-pw").value;
  if (!f) { say("ck-msg", "Choose the backup file first.", true); return; }
  if (!pw) { say("ck-msg", "Type the password for that backup.", true); return; }
  var btn = $("ck-go"); btn.disabled = true; say("ck-msg", "Opening the backup\u2026 this can take a few seconds.");
  var pwBytes = new TextEncoder().encode(pw);
  var head = new Uint8Array(4); new DataView(head.buffer).setUint32(0, pwBytes.length, false);
  fetch("/admin/backup/check", { method: "POST", credentials: "same-origin",
    headers: { "content-type": "application/octet-stream", "x-ebp-csrf": csrf },
    body: new Blob([head, pwBytes, f]) })
    .then(function (r) {
      return r.text().then(function (t) {
        var j = null; try { j = JSON.parse(t); } catch (e) {}
        if (r.status === 401) { signedOut(); throw new Error("Please sign in again."); }
        if (r.status === 413) throw new Error("That file is too large to be one of this app\u2019s backups.");
        if (!r.ok) throw new Error((j && j.error && j.error.message) || (j && j.message) || ("Check failed (" + r.status + ")"));
        return j;
      });
    })
    .then(function (j) {
      $("ck-pw").value = "";
      var holds = [j.has_database ? "your order history and settings" : null, j.has_key ? "the key that protects them" : null, j.wallet_included ? "your wallet files" : null].filter(Boolean).join(", ");
      say("ck-msg", "This backup is good. It opens and holds " + j.file_count + " files: " + holds + "." + (j.wallet_included ? "" : " Your wallet runs outside this app, so its files are not inside; your recovery words are its backup."));
    })
    .catch(function (e) { say("ck-msg", e.message, true); })
    .then(function () { btn.disabled = false; });
});

// What "Move my money now" tells the owner. Money that could not be part of the send is named
// with its amount and the reason, so it never looks like part of the till went missing.
function sweepMessage(r) {
  var why = { renewing: " are being renewed by the Ark server and are locked for now.", collecting: " from a Lightning payment is still being collected.", confirming: " is waiting for network confirmations." };
  var rest = (r.held_back || []).filter(function (h) { return why[h.kind] && Number(h.sats) > 0; }).map(function (h) { return fmtSats(h.sats) + why[h.kind]; });
  var tail = rest.length ? " " + rest.join(" ") + " Try again in a few minutes to move the rest." : "";
  if (r.swept_sats) return "Sent " + fmtSats(r.swept_sats) + " to your payout address. It can take a few minutes to arrive." + (rest.length ? " Not included:" + tail : "");
  if (rest.length) return "Nothing could be moved right now." + tail;
  return r.note || "Nothing to move right now.";
}
$("sweep-go").addEventListener("click", function () {
  askConfirm({ title: "Move your money now?", body: "Everything in the till is sent to your payout address now.", cancel: "Not now", ok: "Move it", danger: false }).then(function (yes) {
    if (!yes) return;
    $("sweep-go").disabled = true;
    api("POST", "/admin/sweep", {}).then(function (r) {
      say("sweep-msg", sweepMessage(r));
    }).catch(function (e) { say("sweep-msg", e.message, true); }).then(function () { $("sweep-go").disabled = false; });
  });
});

function loadPayout() {
  api("GET", "/api/v1/payout").then(function (p) {
    var KINDNAME = { ark: "an Ark address", onchain: "an on-chain Bitcoin address" };
    $("po-now").textContent = p.address ? "Current address (" + KINDNAME[p.kind] + "): " + p.address + "." : "No payout address yet — money stays in the till until you add one.";
    var kinds = $("po-kinds"); clear(kinds);
    [["Ark address (recommended)", (p.live ? "starts with ark1… (your Ark wallet must use ark.second.tech)" : "starts with tark1… on this play-money server") + " — instant and almost free, so only a small amount is ever held on this server."],
     ["On-chain Bitcoin address", (p.live ? "starts with bc1… (older wallets: 1… or 3…)" : "starts with tb1… on this play-money server") + " — the most widely understood option. Withdrawals wait for network fees to be low. Every withdrawal goes to this same address, which is easy to use but lets outsiders link your payouts together (a privacy trade-off)."]].forEach(function (k) {
      var d = document.createElement("div"); var b = document.createElement("strong"); b.textContent = k[0] + ": "; d.appendChild(b); d.appendChild(document.createTextNode(k[1])); kinds.appendChild(d);
    });
    $("sweep-box").classList.toggle("hidden", !p.address);
    $("min-box").classList.toggle("hidden", !p.address);
    $("fold-when").classList.toggle("hidden", !p.address);
    if (p.address) {
      minKind = p.kind === "onchain" ? "onchain" : "ark";
      minFloor = p.sweep_min_floor_sats || 1500;
      smallCoin = p.small_coin_sats || 1300;
      exitMin = p.exit_min_sats || 50000;
      var sm = p.sweep_min_sats || minFloor;
      $("min-sub").textContent = "The server sends your money on once the till holds " + fmtSats(sm) + (p.sweep_min_is_custom ? " (your setting)." : ".");
      $("min-now").textContent = "The server sends all of the money in the till to your payout address once it holds at least " + fmtSats(sm) + (p.sweep_min_is_custom ? " (your setting)" : " (the default, which is also the lowest we allow)") + ". Changing it needs your admin password and takes effect right away; it cannot send money anywhere new.";
      $("min-why").textContent = minKind === "onchain"
        ? "We recommend much more than the lowest amount: every on-chain withdrawal pays a network fee that can be high and changes a lot from day to day, so fewer, bigger withdrawals cost far less. It is your decision. If you expect small amounts, an Ark payout address is almost free to send to."
        : plainSats(minFloor) + " is the lowest to ensure reliability.";
      $("min-new").min = String(minFloor);
      $("min-reset").classList.toggle("hidden", !p.sweep_min_is_custom);
      $("sweep-small").textContent = smallCoin.toLocaleString("en-US");
    }
    var pend = p.pending_address;
    $("po-pending").classList.toggle("hidden", !pend);
    $("po-cancel-box").classList.toggle("hidden", !pend);
    if (pend) { var m = $("po-pending"); m.className = "msg bad"; m.textContent = "A change to " + pend + " takes effect " + when(p.pending_effective_at * 1000) + ". Until then the current address is used."; }
    $("po-form").classList.toggle("hidden", !!p.set_by_server_config);
    if (p.set_by_server_config) $("po-now").textContent += " (set in the server's configuration; change it there)";
  }).catch(function (e) { $("po-now").textContent = e.message; });
}
$("po-go").addEventListener("click", function () {
  var b = $("po-go"); b.disabled = true;
  api("POST", "/admin/payout", { address: $("po-addr").value, admin_password: $("po-pw").value, code: $("po-code").value }).then(function (r) {
    $("po-addr").value = ""; $("po-pw").value = ""; $("po-code").value = "";
    say("po-msg", r.applied_now ? "Saved. Payouts now go to the new address." : "Saved. For safety it takes effect in about " + r.change_delay_hours + " hours; you can cancel until then.");
    loadPayout();
  }).catch(function (e) { say("po-msg", e.message, true); }).then(function () { b.disabled = false; });
});
function saveMin(value, btn) {
  btn.disabled = true;
  api("POST", "/admin/payout/min", { kind: minKind, min_sats: value, admin_password: $("min-pw").value, code: $("min-code").value }).then(function (r) {
    $("min-new").value = ""; $("min-usd").textContent = ""; $("min-pw").value = ""; $("min-code").value = "";
    say("min-msg", value === null ? "Saved. Back to the default: " + plainSats(r.sweep_min_floor_sats) + "." : "Saved. The server now sends everything once the till holds at least " + plainSats(r.sweep_min_sats) + "." + (minWarn(value) ? " " + minWarn(value) : ""));
    loadPayout();
  }).catch(function (e) { say("min-msg", e.message, true); }).then(function () { btn.disabled = false; });
}
$("min-go").addEventListener("click", function () {
  clearTimeout(minTimer);
  var raw = $("min-new").value.trim();
  var n = Number(raw);
  if (!raw || !Number.isInteger(n) || n < 1) { say("min-msg", "Type a whole number of sats, for example " + (minKind === "onchain" ? "100000" : "3000") + ".", true); return; }
  if (n < minFloor) { say("min-msg", floorWhy(minKind, n), true); return; }
  saveMin(n, $("min-go"));
});
$("min-reset").addEventListener("click", function () { saveMin(null, $("min-reset")); });
$("po-cancel").addEventListener("click", function () {
  api("POST", "/admin/payout/cancel", {}).then(function () { say("po-msg", "Cancelled. Nothing changed."); loadPayout(); }).catch(function (e) { say("po-msg", e.message, true); });
});

// ---- Pause new payments (added because of the PHP2Shell bugs that can take over a PHP shop; see docs/dev/SHOP_COMPROMISE.md) ----
// Counts the owner's own pause/resume clicks, so a slower, older answer from the
// start-up session check can never overwrite what they just did.
var pauseClicks = 0;
function setPaused(p, btn, code, msgEl) {
  if (btn) btn.disabled = true;
  pauseClicks++;
  api("POST", "/admin/pause", { paused: p, code: code || "" }).then(function () { paused = p; renderPause(); })
    .catch(function (e) {
      if (msgEl) { msgEl.className = "msg bad"; msgEl.textContent = e.message; } else say("pause-msg", e.message, true);
      if (btn) btn.disabled = false;
    });
}
// The Resume button, with a code box that shows only while two-step sign-in is on.
// (Pausing never asks for a code: it is the emergency button.)
function resumeRow() {
  var w = el("div");
  var cw = el("div"); cw.className = "needcode" + (twofaOn ? "" : " hidden");
  var lab = el("label", "Code from your authenticator app"); var inp = document.createElement("input");
  inp.type = "text"; inp.inputMode = "numeric"; inp.autocomplete = "off"; inp.placeholder = "6 digits";
  inp.id = "resume-code-" + (++resumeBoxes); lab.setAttribute("for", inp.id);
  cw.appendChild(lab); cw.appendChild(inp); w.appendChild(cw);
  var msg = el("p", "", "msg hidden");
  var b = el("div", "", "btns");
  b.appendChild(button("Resume payments", function (btn) { msg.className = "msg hidden"; setPaused(false, btn, inp.value, msg); }, "primary"));
  w.appendChild(b); w.appendChild(msg);
  return w;
}
var resumeBoxes = 0;
function renderPause() {
  var box = $("pause-card"), top = $("paused-card"); clear(box); clear(top);
  top.classList.toggle("hidden", !paused);
  if (paused) {
    top.appendChild(el("strong", "New Bitcoin payments are paused", "bad"));
    top.appendChild(el("p", "Customers can't start a new Bitcoin payment. Payments already started still finish.", "muted"));
    top.appendChild(resumeRow());
    box.appendChild(el("p", "Paused. Customers can't start new Bitcoin payments; ones already started still finish.", "muted"));
    box.appendChild(resumeRow());
  } else {
    box.appendChild(el("p", "Bitcoin payments are on. If you think your shop was hacked, or something looks wrong, pause here first: customers can't start a new Bitcoin payment (ones already started still finish). You can resume any time.", "muted"));
    var pb = el("div", "", "btns"); pb.appendChild(button("Pause new payments", function (b) { setPaused(true, b); })); box.appendChild(pb);
  }
  var m = el("p", "", "msg hidden"); m.id = "pause-msg"; box.appendChild(m);
}

// ---- Limit the shop's key to the shop's address ----
function loadKeylock() {
  api("POST", "/admin/keylock/status", {}).then(showKeylock).catch(function () {});
}
function showKeylock(r) {
  if (document.activeElement !== $("kl-addr")) $("kl-addr").value = (r.addresses || []).join(", ");
  var b = $("kl-blocked");
  if (r.last_blocked_address) { b.textContent = "A request with your shop key from " + r.last_blocked_address + " (" + when(r.last_blocked_at * 1000) + ") was refused. If that is your shop, add this address."; b.classList.remove("hidden"); }
  else b.classList.add("hidden");
}
$("kl-save").addEventListener("click", function () {
  var btn = $("kl-save"); btn.disabled = true;
  api("POST", "/admin/keylock/save", { addresses: $("kl-addr").value, code: $("kl-code").value }).then(function (r) {
    $("kl-code").value = "";
    showKeylock(r);
    say("kl-msg", (r.addresses || []).length ? "Saved. Only these addresses can use your shop's key. Test your shop checkout now." : "Saved. No limit: the key works from anywhere.");
  }).catch(function (e) { say("kl-msg", e.message, true); }).then(function () { btn.disabled = false; });
});

// ---- Two-step sign-in ----
function applyTwofa(on, declined) {
  twofaOn = on; twofaDeclined = declined;
  var f = document.querySelectorAll(".needcode");
  for (var i = 0; i < f.length; i++) f[i].classList.toggle("hidden", !on);
}
function loadTwofa() {
  var clicksAtStart = pauseClicks;
  fetch("/admin/session", { credentials: "same-origin" }).then(function (r) { return r.json(); }).then(function (s) {
    applyTwofa(!!s.twofa, !!s.twofa_declined);
    passkeysOn = !!s.passkeys; decoratePasskeyInputs();
    if (clicksAtStart === pauseClicks) { paused = !!s.paused; renderPause(); }
    renderPrompt();
    if (!twofaBusy) renderTwofaCard();
  }).catch(function () {});
}
var twofaBusy = false;
function field(id, label, type, ph) {
  var w = el("div");
  var l = el("label", label); l.setAttribute("for", id); w.appendChild(l);
  var i = document.createElement("input"); i.id = id; i.type = type || "text"; i.autocomplete = "off"; i.spellcheck = false;
  if (type === "tel") { i.inputMode = "numeric"; i.autocomplete = "one-time-code"; }
  if (ph) i.placeholder = ph;
  w.appendChild(i); return w;
}
function button(text, fn, cls) { var b = el("button", text); b.type = "button"; if (cls) b.className = cls; b.addEventListener("click", function () { fn(b); }); return b; }
var LESS_SECURE = "Password only is less secure: anyone who learns your password can sign in and change where your money goes. You can turn on two-step sign-in any time in Settings.";

function renderPrompt() {
  var box = $("twofa-prompt"); clear(box);
  if (twofaOn || twofaDeclined || !csrf) { box.classList.add("hidden"); return; }
  box.classList.remove("hidden");
  box.appendChild(el("strong", "Add a second lock to your account"));
  box.appendChild(el("p", "Right now only your password protects this page. With two-step sign-in you also type a 6-digit code from an app on your phone, so someone who finds out your password still can't get in or change where your money goes. It takes about two minutes.", "muted"));
  var btns = el("div", "", "btns");
  btns.appendChild(button("Turn on (recommended)", function () { show("set"); openFold("twofa-card"); startTwofaForm(); $("twofa-card").scrollIntoView(); }, "primary"));
  btns.appendChild(button("Skip, use password only", function () {
    clear(box);
    box.appendChild(el("strong", "Use password only?"));
    var w = el("p", LESS_SECURE, "muted warnbox"); box.appendChild(w);
    var b2 = el("div", "", "btns");
    b2.appendChild(button("Turn on instead (recommended)", function () { show("set"); openFold("twofa-card"); startTwofaForm(); $("twofa-card").scrollIntoView(); }, "primary"));
    b2.appendChild(button("Skip anyway", function (b) {
      b.disabled = true;
      api("POST", "/admin/2fa/skip", {}).then(function () { applyTwofa(false, true); renderPrompt(); renderTwofaCard(); }).catch(function (e) { b.disabled = false; w.textContent = e.message; });
    }));
    box.appendChild(b2);
  }));
  box.appendChild(btns);
}

function renderTwofaCard() {
  var box = $("twofa-card"); clear(box);
  if (twofaOn) {
    box.appendChild(el("p", "On. Signing in needs your password and a second step: a 6-digit code from your authenticator app, or a passkey if you add one below. Changing your payout address, password or downloading a backup asks for it too.", "muted"));
    box.appendChild(el("p", "What is the difference? An app code is 6 numbers your phone app makes every 30 seconds, and you type them in. A passkey is a saved key in your password manager, phone or security key, and you approve it with a tap or a fingerprint instead of typing. Either one, together with your password, is two-step sign-in. Your backup codes work for both.", "muted"));
    box.appendChild(el("strong", "Authenticator app code"));
    var left = el("p", "", "muted"); box.appendChild(left);
    var passkeyNote = el("p", "", "muted hidden"); box.appendChild(passkeyNote);
    api("POST", "/admin/2fa/status", {}).then(function (s) {
      left.textContent = "Backup codes left: " + s.recovery_left + " of 10.";
      if (s.passkeys > 0) { passkeyNote.textContent = "To turn the app code off, first turn off your passkeys below: passkeys need it."; passkeyNote.classList.remove("hidden"); }
    }).catch(function () {});
    var out = el("div"); box.appendChild(out);
    var reveal = el("div", "", "btns");
    box.appendChild(reveal).appendChild(button("Turn off app code", function () {
      // The form below has its own confirm button, so this one goes away (a second
      // button next to it looked like two ways to do the same thing).
      reveal.classList.add("hidden");
      clear(out);
      out.appendChild(el("p", "This stops asking for the 6-digit code. If you have passkeys, turn those off first. With nothing left, anyone who learns your password can sign in and change where your money goes.", "muted warnbox"));
      out.appendChild(field("tf-off-pw", "Admin password", "password"));
      out.appendChild(field("tf-off-code", "Code from your authenticator app (or a backup code)", "tel", "6 digits"));
      var b = el("div", "", "btns");
      b.appendChild(button("Turn off app code", function (btn) {
        btn.disabled = true;
        api("POST", "/admin/2fa/disable", { admin_password: $("tf-off-pw").value, code: $("tf-off-code").value })
          .then(function () { applyTwofa(false, true); renderPrompt(); renderTwofaCard(); })
          .catch(function (e) { say("tf-msg", e.message, true); btn.disabled = false; });
      }));
      b.appendChild(button("Cancel", function () { renderTwofaCard(); }));
      out.appendChild(b);
      var m = el("p", "", "msg hidden"); m.id = "tf-msg"; out.appendChild(m);
    }));
    var pk = el("div", "", "passkeys"); pk.id = "pk-box"; box.appendChild(pk);
    loadPasskeys();
    return;
  }
  box.appendChild(el("p", twofaDeclined ? "Off (password only). Anyone who learns your password can sign in. Turn it on to also require a 6-digit code from an app on your phone." : "Off. Turn it on to also require a 6-digit code from an app on your phone when you sign in.", "muted"));
  box.appendChild(el("p", "Passkeys (a tap or fingerprint instead of typing a code) can be added once the app code is on.", "muted"));
  box.appendChild(el("div", "", "btns")).appendChild(button("Turn on", startTwofaForm, "primary"));
}

// ---- Passkeys (beta) ----
function passkeysCanSignIn() { return !!(window.PublicKeyCredential && navigator.credentials && navigator.credentials.get); }
function requestOptions(o) {
  return { publicKey: {
    challenge: b64uToBuf(o.challenge), rpId: o.rpId, timeout: o.timeout, userVerification: o.userVerification,
    allowCredentials: (o.allowCredentials || []).map(function (c) { return { type: c.type, id: b64uToBuf(c.id) }; })
  } };
}
function assertionBody(challenge, cred) {
  return {
    challenge: challenge, id: cred.id,
    client_data_json: bufToB64u(cred.response.clientDataJSON),
    authenticator_data: bufToB64u(cred.response.authenticatorData),
    signature: bufToB64u(cred.response.signature)
  };
}
function passkeyUseErrorText(e) {
  var n = e && e.name;
  if (n === "NotAllowedError") return "The passkey was cancelled or timed out. You can try again, or type your app code instead.";
  return (e && e.message) || "That did not work.";
}
// Sign in: the password was right, now the passkey instead of a code.
$("login-pk-btn").addEventListener("click", function () {
  var o = loginPasskey; if (!o) return;
  var m = $("login-err"); m.classList.add("hidden");
  var btn = $("login-pk-btn"); btn.disabled = true;
  navigator.credentials.get(requestOptions(o)).then(function (cred) {
    return api("POST", "/admin/login/passkey", assertionBody(o.challenge, cred));
  }).then(function (r) {
    $("pw").value = ""; $("code").value = ""; $("code-box").classList.add("hidden"); $("login-pk").classList.add("hidden"); loginPasskey = null; signedIn(r.csrf);
  }).catch(function (e) {
    m.textContent = passkeyUseErrorText(e) + (/expired/i.test(e && e.message || "") ? " Press Sign in again to start over." : "");
    m.classList.remove("hidden");
  }).then(function () { btn.disabled = false; });
});
// Approve one action: a "Use a passkey instead" button under every code box. A good
// touch fills the box with a one-use ticket (shown as dots), which the action's own
// request sends in place of a code. It is wiped after any request that carried a code.
function pkSpecialInputs() { return document.querySelectorAll('input[id$="-code"]:not(#tf-code), input[id^="resume-code-"]'); }
// Back to a normal code box: drops a passkey approval, so an app code (or a backup code) can be typed.
function pkShowCodeBox(inp, clearValue) {
  if (clearValue) inp.value = "";
  delete inp.dataset.pk; inp.type = inp.dataset.pkType || "text";
  var w = inp.nextElementSibling;
  if (w && w.classList.contains("pkuse")) {
    var n = w.querySelector(".pknote"), s = w.querySelector(".pkswitch");
    if (n) { n.textContent = ""; n.classList.add("hidden"); }
    if (s) s.classList.add("hidden");
  }
}
function resetPkInputs() {
  var all = document.querySelectorAll("input[data-pk]");
  for (var i = 0; i < all.length; i++) pkShowCodeBox(all[i], true);
  var notes = document.querySelectorAll(".pkuse .pknote");
  for (var j = 0; j < notes.length; j++) { notes[j].textContent = ""; notes[j].classList.add("hidden"); }
  var sw = document.querySelectorAll(".pkuse .pkswitch");
  for (var k = 0; k < sw.length; k++) sw[k].classList.add("hidden");
}
function decoratePasskeyInputs() {
  var inputs = pkSpecialInputs();
  if (!passkeysOn || !passkeysCanSignIn()) {
    var gone = document.querySelectorAll(".pkuse");
    for (var g = 0; g < gone.length; g++) gone[g].remove();
    return;
  }
  for (var i = 0; i < inputs.length; i++) {
    var inp = inputs[i];
    if (inp.nextElementSibling && inp.nextElementSibling.classList.contains("pkuse")) continue;
    var wrap = el("div", "", "pkuse"), note = el("p", "", "muted pknote hidden");
    var back = button("Use an app code instead", (function (box) { return function () { pkShowCodeBox(box, true); box.focus(); }; })(inp), "small pkswitch hidden");
    wrap.appendChild(button("Use a passkey instead", pkApprove(inp, note, back), "small"));
    wrap.appendChild(back);
    wrap.appendChild(note);
    wrap.appendChild(el("p", "Passkey not working? Type the 6-digit code from your authenticator app (or a backup code) in the box above instead.", "muted pkhint"));
    inp.insertAdjacentElement("afterend", wrap);
    inp.addEventListener("input", function (e) { if (e.target.dataset.pk) pkShowCodeBox(e.target, false); });
  }
}
function pkApprove(inp, note, back) {
  return function (btn) {
    note.classList.remove("hidden"); note.textContent = "Waiting for your passkey…";
    btn.disabled = true;
    var options;
    api("POST", "/admin/passkeys/stepup/begin", {}).then(function (r) {
      options = r.options;
      return navigator.credentials.get(requestOptions(options));
    }).then(function (cred) {
      return api("POST", "/admin/passkeys/stepup/finish", assertionBody(options.challenge, cred));
    }).then(function (r) {
      inp.dataset.pkType = inp.type === "password" ? "text" : inp.type;
      inp.value = "pk:" + r.ticket; inp.type = "password"; inp.dataset.pk = "1";
      note.textContent = "Passkey approved. It works for one action, for two minutes.";
      back.classList.remove("hidden");
    }).catch(function (e) { note.textContent = passkeyUseErrorText(e) + (e && e.name === "NotAllowedError" ? "" : " You can also type your app code in the box above."); })
      .then(function () { btn.disabled = false; });
  };
}
try { new MutationObserver(function () { decoratePasskeyInputs(); }).observe(document.body, { childList: true, subtree: true }); } catch (e) {}
function b64uToBuf(s) {
  s = String(s).replace(/-/g, "+").replace(/_/g, "/");
  while (s.length % 4) s += "=";
  var bin = atob(s), a = new Uint8Array(bin.length);
  for (var i = 0; i < bin.length; i++) a[i] = bin.charCodeAt(i);
  return a.buffer;
}
function bufToB64u(buf) {
  var a = new Uint8Array(buf), s = "";
  for (var i = 0; i < a.length; i++) s += String.fromCharCode(a[i]);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function passkeysSupported() { return !!(window.PublicKeyCredential && navigator.credentials && navigator.credentials.create); }
function creationOptions(o) {
  return { publicKey: {
    challenge: b64uToBuf(o.challenge), rp: o.rp,
    user: { id: b64uToBuf(o.user.id), name: o.user.name, displayName: o.user.displayName },
    pubKeyCredParams: o.pubKeyCredParams, timeout: o.timeout, attestation: o.attestation,
    authenticatorSelection: o.authenticatorSelection,
    excludeCredentials: (o.excludeCredentials || []).map(function (c) { return { type: c.type, id: b64uToBuf(c.id) }; })
  } };
}
function passkeyErrorText(e) {
  var n = e && e.name;
  if (n === "NotAllowedError") return "The passkey was cancelled or timed out. Nothing was added.";
  if (n === "InvalidStateError") return "This passkey is already added.";
  if (n === "NotSupportedError") return "Your browser or device cannot make this kind of passkey.";
  return (e && e.message) || "That did not work.";
}
function loadPasskeys() {
  var box = $("pk-box"); if (!box) return;
  api("POST", "/admin/passkeys/status", {}).then(function (d) { renderPasskeys(d || {}); })
    .catch(function () { clear(box); box.appendChild(el("p", "Passkeys could not be loaded. Reload the page to try again.", "muted")); });
}
function pkFormButtons(go, label, again) {
  var b = el("div", "", "btns");
  b.appendChild(button(label, go, "primary"));
  b.appendChild(button("Cancel", again));
  return b;
}
function renderPasskeys(d) {
  var box = $("pk-box"); if (!box) return; clear(box);
  var keys = d.passkeys || [];
  passkeysOn = keys.length > 0; decoratePasskeyInputs();
  box.appendChild(el("strong", "Passkeys (beta)"));
  if (d.unavailable) { box.appendChild(el("p", "Passkeys cannot be used on this server's address. " + d.unavailable, "muted")); return; }
  if (!passkeysSupported()) { box.appendChild(el("p", "This browser cannot make passkeys. Try a recent Chrome, Edge, Safari or Firefox.", "muted")); return; }
  box.appendChild(el("p", keys.length ? "These passkeys can be used with your password to sign in. Add a second one on another device or in another place, in case one is lost." : "None yet. A passkey can live in your password manager (for example Bitwarden), on your phone, or on a security key.", "muted"));
  var list = el("div", "", "pklist");
  keys.forEach(function (k) {
    var r = el("div", "", "row pkrow");
    var what = k.synced ? "kept in a password manager or your account (works on your other devices)" : "only on the device that made it";
    r.appendChild(el("span", k.name + " · " + what + " · " + (k.last_used_at ? "last used " + when(new Date(k.last_used_at * 1000).toISOString()) : "not used yet")));
    r.appendChild(button("Remove", function () { showPasskeyRemove(k); }, "small"));
    list.appendChild(r);
  });
  box.appendChild(list);
  var btns = el("div", "", "btns"); box.appendChild(btns);
  if (keys.length < (d.max || 10)) btns.appendChild(button("Add a passkey", showPasskeyAdd, "primary"));
  if (keys.length) btns.appendChild(button("Turn off passkeys", showPasskeysOff));
  var m = el("p", "", "msg hidden"); m.id = "pk-msg"; box.appendChild(m);
}
function showPasskeyAdd() {
  var box = $("pk-box"); clear(box);
  box.appendChild(el("strong", "Add a passkey"));
  box.appendChild(el("p", "First confirm it is you. Then your browser asks where to keep the passkey (for example Bitwarden or your phone) and you approve it.", "muted"));
  box.appendChild(field("pk-pw", "Admin password", "password"));
  box.appendChild(field("pk-code", "Code from your authenticator app (or a backup code)", "tel", "6 digits"));
  box.appendChild(field("pk-name", "A name for this passkey, so you can tell them apart", "text", "For example: Bitwarden on my laptop"));
  box.appendChild(pkFormButtons(function (btn) {
    btn.disabled = true;
    var challenge = "";
    api("POST", "/admin/passkeys/begin", { admin_password: $("pk-pw").value, code: $("pk-code").value }).then(function (r) {
      challenge = r.options.challenge;
      return navigator.credentials.create(creationOptions(r.options));
    }).then(function (cred) {
      return api("POST", "/admin/passkeys/finish", {
        challenge: challenge, id: bufToB64u(cred.rawId), name: $("pk-name").value,
        client_data_json: bufToB64u(cred.response.clientDataJSON),
        attestation_object: bufToB64u(cred.response.attestationObject)
      });
    }).then(function () { loadPasskeys(); setTimeout(function () { if ($("pk-msg")) say("pk-msg", "Passkey added. Test it by signing out and back in once you can, and keep your backup codes.", false); }, 300); })
      .catch(function (e) { say("pk-msg", passkeyErrorText(e), true); btn.disabled = false; });
  }, "Add passkey", loadPasskeys));
  var m = el("p", "", "msg hidden"); m.id = "pk-msg"; box.appendChild(m);
}
function showPasskeyRemove(k) {
  var box = $("pk-box"); clear(box);
  box.appendChild(el("strong", "Remove " + k.name + "?"));
  box.appendChild(el("p", "This passkey will no longer work for signing in. Your other passkeys and your app code are not affected.", "muted"));
  box.appendChild(field("pk-pw", "Admin password", "password"));
  box.appendChild(field("pk-code", "Code from your authenticator app (or a backup code)", "tel", "6 digits"));
  box.appendChild(pkFormButtons(function (btn) {
    btn.disabled = true;
    api("POST", "/admin/passkeys/remove", { admin_password: $("pk-pw").value, code: $("pk-code").value, id: k.id })
      .then(function () { loadPasskeys(); })
      .catch(function (e) { say("pk-msg", e.message, true); btn.disabled = false; });
  }, "Remove passkey", loadPasskeys));
  var m = el("p", "", "msg hidden"); m.id = "pk-msg"; box.appendChild(m);
}
function showPasskeysOff() {
  var box = $("pk-box"); clear(box);
  box.appendChild(el("strong", "Turn off passkeys"));
  box.appendChild(el("p", "This removes all your passkeys. Signing in will go back to your password plus the 6-digit code from your authenticator app (or a backup code). Your app code stays on.", "muted"));
  box.appendChild(field("pk-pw", "Admin password", "password"));
  box.appendChild(field("pk-code", "Code from your authenticator app (or a backup code)", "tel", "6 digits"));
  box.appendChild(pkFormButtons(function (btn) {
    btn.disabled = true;
    api("POST", "/admin/passkeys/disable", { admin_password: $("pk-pw").value, code: $("pk-code").value })
      .then(function () { renderTwofaCard(); })
      .catch(function (e) { say("pk-msg", e.message, true); btn.disabled = false; });
  }, "Turn off passkeys", loadPasskeys));
  var m = el("p", "", "msg hidden"); m.id = "pk-msg"; box.appendChild(m);
}

function startTwofaForm() {
  twofaBusy = true;
  var box = $("twofa-card"); clear(box);
  box.appendChild(el("p", "First, confirm it's you.", "muted"));
  box.appendChild(field("tf-pw", "Admin password", "password"));
  var b = el("div", "", "btns");
  b.appendChild(button("Continue", function (btn) {
    btn.disabled = true;
    api("POST", "/admin/2fa/start", { admin_password: $("tf-pw").value }).then(function (r) { showTwofaSetup(r); })
      .catch(function (e) { say("tf-msg", e.message, true); btn.disabled = false; });
  }, "primary"));
  b.appendChild(button("Cancel", function () { twofaBusy = false; renderTwofaCard(); }));
  box.appendChild(b);
  var m = el("p", "", "msg hidden"); m.id = "tf-msg"; box.appendChild(m);
}

function showTwofaSetup(r) {
  var box = $("twofa-card"); clear(box);
  box.appendChild(el("p", "1. Install an authenticator app on your phone if you don't have one (for example Google Authenticator, Microsoft Authenticator, Authy or 1Password).", "muted"));
  box.appendChild(el("p", "2. In the app, add a new account. Scan this picture, or type the key under it.", "muted"));
  var qr = el("div", "", "qrbox");
  try {
    var img = document.createElement("img"); img.alt = "QR code to add this account to your authenticator app";
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(window.ebpQrSvg(r.uri));
    qr.appendChild(img);
  } catch (e) {}
  box.appendChild(qr);
  var key = el("div", r.secret.replace(/(.{4})/g, "$1 ").trim(), "secret"); key.id = "tf-key"; box.appendChild(key);
  var row2 = el("div", "", "btns");
  var copy = el("button", "Copy key", "small"); copy.type = "button"; copy.setAttribute("data-copy", "tf-key"); row2.appendChild(copy);
  var open = el("a", "Open in my authenticator app"); open.href = r.uri; row2.appendChild(open);
  box.appendChild(row2);
  box.appendChild(el("p", "3. Type the 6-digit code the app now shows (it changes every 30 seconds).", "muted"));
  box.appendChild(field("tf-code", "6-digit code", "tel", "6 digits"));
  var b = el("div", "", "btns");
  b.appendChild(button("Turn on two-step sign-in", function (btn) {
    btn.disabled = true;
    api("POST", "/admin/2fa/confirm", { code: $("tf-code").value }).then(function (res) { applyTwofa(true, false); showRecovery(res.recovery_codes); renderPrompt(); })
      .catch(function (e) { say("tf-msg", e.message, true); btn.disabled = false; });
  }, "primary"));
  b.appendChild(button("Cancel", function () { twofaBusy = false; renderTwofaCard(); }));
  box.appendChild(b);
  var m = el("p", "", "msg hidden"); m.id = "tf-msg"; box.appendChild(m);
}

function showRecovery(codes) {
  var box = $("twofa-card"); clear(box);
  box.appendChild(el("strong", "Two-step sign-in is on."));
  box.appendChild(el("p", "Save these 10 backup codes somewhere safe, like your password manager. If you lose your phone, each one lets you sign in once. They are shown only now.", "muted"));
  var c = el("div", codes.join("\n"), "secret codes"); c.id = "tf-codes"; box.appendChild(c);
  var b = el("div", "", "btns");
  var copy = el("button", "Copy codes", "small"); copy.type = "button"; copy.setAttribute("data-copy", "tf-codes"); b.appendChild(copy);
  b.appendChild(button("I saved them", function () { twofaBusy = false; renderTwofaCard(); }, "primary"));
  box.appendChild(b);
}

function loadAudit() {
  api("GET", "/api/v1/audit-log?" + pageQuery("audit")).then(function (list) {
    if (pageGone("audit", list.length, loadAudit)) return;
    var a = $("audit"); clear(a);
    list.slice(0, PAGE_SIZE.audit).forEach(function (x) { a.appendChild(row(String(x.action).replace(/[._]/g, " "), when(x.created_at))); });
    if (!list.length) a.appendChild(el("span", "Nothing yet.", "muted"));
    pagerFor(a, "audit", list.length, loadAudit);
  }).catch(function (e) { $("audit").textContent = e.message; });
}

// ---- Emergency withdrawal (Settings). Folded away; nothing starts on its own.
var exState = null, exTimer = null, exAddrShown = "", exAddrBusy = false;

// A quick look at the address while typing. It cannot check the ending of an address
// (the server does that when you press Start, before anything is spent), but it
// catches the common mistakes at once: an Ark address, a test address on real money
// (or the other way round), a silent payment address, or something that is not an address.
function exCheckDest(raw) {
  var v = (raw || "").trim(), l = v.toLowerCase();
  var live = EBP_MODE === "live", test = EBP_MODE === "test";
  var example = live ? " It starts with bc1, 1 or 3." : test ? " On this play-money server it starts with tb1." : "";
  if (!v) return { ok: false, empty: true, text: "Type the regular Bitcoin (on-chain) address where the money should go." + example };
  if (/^t?ark1/.test(l)) return { ok: false, text: "That is an Ark address. An emergency withdrawal leaves Ark, so it needs a regular Bitcoin (on-chain) address." + example };
  if (/^t?sp1/.test(l)) return { ok: false, text: "Silent payment addresses cannot be used here. Use a regular Bitcoin (on-chain) address." + example };
  if (live && (/^(tb1|bcrt1)/.test(l) || /^[mn2][1-9A-HJ-NP-Za-km-z]{25,34}$/.test(v))) return { ok: false, text: "That is a test-network address, but this server uses real Bitcoin." + example };
  if (test && (/^bc1/.test(l) || /^[13][1-9A-HJ-NP-Za-km-z]{25,34}$/.test(v))) return { ok: false, text: "That is a real-Bitcoin address, but this server uses play money." + example };
  var bech = /^(bc1|tb1|bcrt1)[a-z0-9]{8,87}$/.test(l);
  var legacy = /^[13mn2][1-9A-HJ-NP-Za-km-z]{25,34}$/.test(v);
  if (!bech && !legacy) return { ok: false, text: "That does not look like a Bitcoin address. Copy it again from your wallet, all of it." + example };
  return { ok: true, text: "Looks like a regular Bitcoin address. The server checks it fully when you press Start, before anything is spent." };
}
function exChecklist() {
  var s = exState, box = $("ex-checks"); if (!s) return;
  var d = exCheckDest($("ex-dest").value);
  var needed = s.deposit_needed_sats;
  var waiting = s.onchain_pending_sats > 0 && s.fee && (s.onchain_confirmed_sats || 0) + s.onchain_pending_sats >= s.fee.broadcast_sat;
  var items = [
    { ok: needed === 0 && s.fundable !== false, text: needed === 0 && s.fundable === false ? "Fee money is on the server, but the wallet does not count it as usable yet. This page tries again every minute."
      : needed === 0 ? "Fee money is on the server and confirmed."
      : needed == null ? "The fee money needed could not be worked out just now. This page tries again every minute."
      : waiting ? "Fee money was sent and is waiting for 1 confirmation. This can take a while; nothing else is needed."
      : "Fee money: send at least " + fmtSats(needed) + " more to the address in Step 1." },
    { ok: d.ok, text: d.text },
    { ok: !!$("ex-pw").value, text: $("ex-pw").value ? "Admin password typed." : "Type your admin password." },
    { ok: !!$("ex-code").value.trim(), text: $("ex-code").value.trim() ? "Code typed." : "Type the code from your authenticator app." },
    { ok: $("ex-ok").checked, text: $("ex-ok").checked ? "You confirmed you understand." : "Tick the box above the button." }
  ];
  var left = items.filter(function (i) { return !i.ok; }).length;
  clear(box);
  var head = el("p", left ? "The button stays grey until these are done (" + left + " to go):" : "Everything is ready. Pressing the button starts the withdrawal.", left ? "muted" : "ok");
  box.appendChild(head);
  items.forEach(function (i) { box.appendChild(el("div", (i.ok ? "\u2713 " : "\u2717 ") + i.text, i.ok ? "ok" : "warn")); });
}
function exReady() {
  var s = exState;
  var d = exCheckDest($("ex-dest").value);
  var note = $("ex-dest-note");
  note.textContent = d.empty ? "A regular Bitcoin (on-chain) address, not an Ark address, from a wallet you hold outside this server. The money is sent straight there." : d.text;
  note.className = "msg " + (d.empty ? "muted" : d.ok ? "ok" : "bad");
  $("ex-start").disabled = !(s && s.allowed && s.two_step_on && s.deposit_needed_sats === 0 && s.fundable !== false && !s.in_progress_since && !(s.live || []).length &&
    $("ex-ok").checked && $("ex-pw").value && $("ex-code").value.trim() && d.ok);
  exChecklist();
}
function exFetchAddr() {
  if (exAddrBusy) return; exAddrBusy = true;
  $("ex-get-addr").classList.add("hidden");
  api("POST", "/admin/exit/deposit-address", {}).then(function (r) {
    exAddrShown = r.address;
    $("ex-addr").textContent = r.address;
    var q = $("ex-qr"); clear(q);
    try {
      var img = document.createElement("img"); img.alt = "QR code for the deposit address";
      img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(window.ebpQrSvg("bitcoin:" + r.address));
      q.appendChild(img);
    } catch (e) {}
  }).catch(function (e) {
    $("ex-addr").textContent = "Could not get the address: " + e.message;
    $("ex-get-addr").classList.remove("hidden");
  }).then(function () { exAddrBusy = false; });
}
var exPrepared = false;
var exFoldOpened = false;
// The start section stays folded until it is needed. It holds the address, password and code
// fields too, so it opens by itself while a withdrawal is running (Send needs them).
// While the emergency withdrawal is on, the row is always listed (title and one line, like the
// rest of Settings); inside it says why it cannot be started yet, if that is so.
function syncStartFold(opted, running) {
  var fold = $("ex-start-fold");
  fold.classList.toggle("hidden", !(opted || running));
  if (running && !exFoldOpened) { fold.open = true; exFoldOpened = true; }
}
// The status answer comes from the wallet and can take a few seconds. Until the first answer the
// section says so, instead of showing only the warning box.
function exLoaded() { var l = $("ex-loading"); if (l) l.classList.add("hidden"); }
function loadExit() {
  api("POST", "/admin/exit/status", {}).then(function (s) {
    exLoaded();
    exState = s;
    var f = $("ex-facts"); clear(f);
    // The emergency withdrawal is opt-in: off until the owner turns it on. One
    // that is already running always shows, whatever the switch says.
    var opted = !!s.opted_in;
    var runningNow = !!s.in_progress_since || (s.live || []).length > 0;
    var showExit = opted || runningNow;
    reserveFloor = s.reserve_floor_sats || 5000;
    $("ex-off-box").classList.toggle("hidden", showExit);
    $("ex-on-box").classList.toggle("hidden", !opted);
    $("ex-off-fold").classList.toggle("hidden", !opted);
    $("ex-facts-box").classList.toggle("hidden", !showExit);
    $("exo-floor").textContent = reserveFloor.toLocaleString("en-US");
    $("exo-twostep").classList.toggle("hidden", !!s.two_step_on);
    $("exo-state").textContent = "Turned on. The server keeps at least " + plainSats(reserveFloor) + " of regular Bitcoin for the network fees and never sends that part out" + (s.onchain_confirmed_sats == null ? "." : (s.onchain_confirmed_sats >= reserveFloor ? ". It has " + plainSats(s.onchain_confirmed_sats) + " now: enough." : ". It has " + plainSats(s.onchain_confirmed_sats) + " now: please add " + plainSats(s.reserve_short_sats == null ? reserveFloor - s.onchain_confirmed_sats : s.reserve_short_sats) + " more (see below).")) ;
    $("exo-state").className = "msg " + (s.onchain_confirmed_sats != null && s.onchain_confirmed_sats < reserveFloor ? "warn" : "ok");
    if (!showExit) {
      $("ex-refusal").className = "msg hidden";
      $("ex-deposit").classList.add("hidden");
      $("ex-start-box").classList.add("hidden");
      $("ex-running").classList.add("hidden");
      $("ex-auth").classList.add("hidden");
      $("ex-left-box").classList.toggle("hidden", !(s.onchain_confirmed_sats > 0));
      $("ex-left-note").textContent = "This takes back all of the fee money on the server (the regular Bitcoin set aside for fees), sending it to the address you typed.";
      $("ex-left").textContent = "Send leftover to my address";
      if (!$("ex-dest").value && s.destination) $("ex-dest").value = s.destination;
      if (!$("exl-dest").value && s.destination) $("exl-dest").value = s.destination;
      syncStartFold(false, false);
      exReady();
      return;
    }
    f.appendChild(row("Ark money on this server", s.ark_sats == null ? "unknown" : fmtSats(s.ark_sats)));
    f.appendChild(row("Network fees to take it out (estimate)", s.fee ? fmtSats(s.fee.total_sat) : "unknown"));
    f.appendChild(row("Fee money on the server (confirmed)", s.onchain_confirmed_sats == null ? "unknown" : fmtSats(s.onchain_confirmed_sats) + (s.onchain_pending_sats ? " (+" + fmtSats(s.onchain_pending_sats) + " waiting for a confirmation)" : "")));
    f.appendChild(row("Must start before", s.deadline ? when(s.deadline * 1000) : "unknown"));
    f.appendChild(row("Ark server answering right now", s.asp_connected === true ? "yes" : s.asp_connected === false ? "no" : "unknown"));
    var live = s.live || [];
    var running = !!s.in_progress_since || live.length > 0;
    var r = $("ex-refusal");
    if (!running && !s.two_step_on) { r.textContent = "Turn on two-step sign-in (above) first: an emergency withdrawal needs your password and a code from your authenticator app."; r.className = "msg bad"; }
    else if (!running && s.refusal) { r.textContent = s.refusal; r.className = s.unpriceable ? "msg warn" : "msg bad"; }
    else { r.className = "msg hidden"; }
    r.title = s.unpriceable ? "Coins that arrived over Lightning cannot be priced for an emergency withdrawal until a refresh turns them into standard coins. A refresh costs at most a few hundred sats and nothing leaves your server." : "";
    if (s.unpriceable && !running && !exPrepared && !s.preparing_since) {
      exPrepared = true;
      api("POST", "/admin/exit/prepare", {}).then(function () { setTimeout(loadExit, 3000); }).catch(function () {});
    }
    var canGo = !running && s.allowed && s.two_step_on;
    // Fee money can be set aside ahead of time, even when a withdrawal is not
    // allowed right now (not enough on the server, fee unknown), but not on
    // servers where the withdrawal is switched off.
    var fund = !running && !s.switched_off && !canGo;
    $("ex-deposit").classList.toggle("hidden", !(canGo || fund));
    if (fund) {
      var tgt = s.reserve_target_sats || 5000;
      var cf = s.onchain_confirmed_sats || 0, pd = s.onchain_pending_sats || 0;
      var short = Math.max(0, tgt - cf);
      $("ex-fund-title").textContent = "Keep fee money on the server";
      var frows = $("ex-fee-rows"); clear(frows);
      frows.appendChild(row("On the server now (confirmed)", fmtSats(cf)));
      if (pd > 0) frows.appendChild(row("Sent, waiting for 1 confirmation", fmtSats(pd)));
      frows.appendChild(row("Still to send", short > 0 ? fmtSats(short) : "nothing"));
      var fv = $("ex-fee-verdict");
      if (short === 0) { fv.textContent = "\u2713 Set aside: the fee money is on the server and confirmed."; fv.className = "msg ok"; }
      else if (cf + pd >= tgt) { fv.textContent = "Sent, waiting for 1 confirmation. This page checks again every minute."; fv.className = "msg warn"; }
      else { fv.textContent = "Please send " + fmtSats(short) + " to the address below, so an emergency withdrawal can be paid for."; fv.className = "msg warn"; }
      var fn = $("ex-deposit-need");
      fn.textContent = "";
      $("ex-addr-box").classList.toggle("hidden", !(short > 0));
      if (short > 0 && !exAddrShown) exFetchAddr();
    } else if (canGo) {
      $("ex-fund-title").textContent = "Step 1: put the fee money on the server";
      $("ex-deposit-need").title = "";
    }
    if (canGo && s.fee) {
      var needed = s.deposit_needed_sats, conf = s.onchain_confirmed_sats || 0, pend = s.onchain_pending_sats || 0;
      var rows = $("ex-fee-rows"); clear(rows);
      rows.appendChild(row("Fee money the server needs", fmtSats(s.fee.broadcast_sat)));
      rows.appendChild(row("On the server now (confirmed)", fmtSats(conf)));
      if (pend > 0) rows.appendChild(row("Sent, waiting for 1 confirmation", fmtSats(pend)));
      rows.appendChild(row("Still to send", needed > 0 ? fmtSats(needed) : "nothing"));
      var v = $("ex-fee-verdict");
      if (needed === 0 && s.fundable === false) { v.textContent = "Not usable yet: the wallet does not count the fee money as usable. This page checks again every minute."; v.className = "msg warn"; }
      else if (needed === 0) { v.textContent = "\u2713 Enough: the fee money is on the server and confirmed."; v.className = "msg ok"; }
      else if (conf + pend >= s.fee.broadcast_sat) { v.textContent = "\u2717 Not confirmed yet: the money was sent and is waiting for 1 confirmation. This page checks again every minute."; v.className = "msg warn"; }
      else { v.textContent = "\u2717 Not enough yet: send at least " + fmtSats(needed) + " more to the address below."; v.className = "msg warn"; }
      $("ex-deposit-need").textContent = needed > 0
        ? "The fees are paid from the server's own Bitcoin balance, which is normally empty. Send from any wallet or exchange, then wait for 1 confirmation. (The final step's fee, about " + fmtSats(s.fee.claim_sat) + ", comes out of the money itself.)"
        : "";
      $("ex-addr-box").classList.toggle("hidden", !(needed > 0));
      if (needed > 0 && !exAddrShown) exFetchAddr();
    }
    $("ex-start-box").classList.toggle("hidden", !canGo);
    $("ex-auth").classList.toggle("hidden", !(canGo || running));
    $("ex-dest").placeholder = EBP_MODE === "live" ? "bc1\u2026" : EBP_MODE === "test" ? "tb1\u2026" : "";
    if (!$("ex-dest").value && s.destination) $("ex-dest").value = s.destination;
    if (!$("exl-dest").value && s.destination) $("exl-dest").value = s.destination;
    $("ex-running").classList.toggle("hidden", !running);
    var list = $("ex-list"); clear(list);
    if (s.in_progress_since) list.appendChild(row("Started", when(s.in_progress_since * 1000)));
    live.forEach(function (e) { list.appendChild(row("Coin " + e.vtxo_id.slice(0, 10) + "…", e.words)); });
    $("ex-claim").classList.toggle("hidden", !s.claimable);
    // The fee money can be taken back. While the withdrawal is turned on, only what
    // is above the floor goes (the server works out the fee); off, all of it.
    var canBack = s.onchain_confirmed_sats > (opted ? reserveFloor : 0);
    $("ex-left-box").classList.toggle("hidden", running || !(opted || canBack));
    $("ex-left").disabled = !canBack;
    $("ex-left-note").textContent = opted
      ? "This sends back the fee money above " + plainSats(reserveFloor) + " to the address you typed (the network fee comes out of it). At least " + plainSats(reserveFloor) + " stays here while the emergency withdrawal is turned on; turn it off to take all of it back."
      : "This takes back all of the fee money on the server (the regular Bitcoin set aside for fees), sending it to the address you typed.";
    if (opted && !canBack) $("ex-left-note").textContent = "The server keeps " + plainSats(reserveFloor) + " of fee money for an emergency withdrawal, and there is nothing above that to take back right now.";
    $("ex-left").textContent = opted ? "Send back what is above " + plainSats(reserveFloor) : "Send leftover to my address";
    var why = $("ex-start-why");
    if (!running && !canGo) {
      why.textContent = (r.className.indexOf("hidden") < 0 && r.textContent) ? r.textContent : "Not available right now: it needs the fee money set aside and Ark money on the server to take out.";
      why.className = "msg warn";
    } else { why.className = "msg warn hidden"; }
    var nx = $("ex-next"), cfm = s.onchain_confirmed_sats, pnd = s.onchain_pending_sats || 0;
    if (running) { nx.textContent = "Next step: a withdrawal is running. Leave the server on and download a fresh backup (above). Open \u201CStart an emergency withdrawal\u201D below to follow it."; nx.className = "msg warn"; }
    else if (!s.two_step_on) { nx.textContent = "Next step: turn on two-step sign-in (above). An emergency withdrawal needs your password and a code."; nx.className = "msg warn"; }
    else if (cfm != null && cfm < reserveFloor && cfm + pnd >= reserveFloor) { nx.textContent = "Next step: wait. Your fee money was sent and is waiting for 1 confirmation. Nothing else to do."; nx.className = "msg warn"; }
    else if (cfm != null && cfm < reserveFloor) { nx.textContent = "Next step: send at least " + plainSats(reserveFloor - cfm) + " of fee money to the address below."; nx.className = "msg warn"; }
    else { nx.textContent = "You are set. Nothing more to do now. If you ever need it, open \u201CStart an emergency withdrawal\u201D at the bottom."; nx.className = "msg ok"; }
    syncStartFold(opted, running);
    exReady();
  }).catch(function (e) { exLoaded(); say("exo-msg", e.message, true); });
}
$("ex-box").addEventListener("toggle", function () {
  if (exTimer) { clearInterval(exTimer); exTimer = null; }
  if ($("ex-box").open) { loadExit(); exTimer = setInterval(loadExit, 60000); }
});
["ex-ok", "ex-pw", "ex-code", "ex-dest"].forEach(function (id) { $(id).addEventListener("input", exReady); $(id).addEventListener("change", exReady); });
$("ex-get-addr").addEventListener("click", exFetchAddr);
function exAction(path, extra, button, pre) {
  pre = pre || "ex";
  var body = { admin_password: $(pre + "-pw").value, code: $(pre + "-code").value.trim(), destination: $(pre + "-dest").value.trim() };
  for (var k in extra) body[k] = extra[k];
  button.disabled = true;
  return api("POST", path, body).then(function (r) {
    $(pre + "-pw").value = ""; $(pre + "-code").value = "";
    say(pre === "ex" ? "ex-msg" : "exl-msg", r.message || "Done.");
    loadExit();
  }).catch(function (e) { say(pre === "ex" ? "ex-msg" : "exl-msg", e.message, true); }).then(function () { button.disabled = false; exReady(); });
}
$("ex-start").addEventListener("click", function () { exAction("/admin/exit/start", { understood: $("ex-ok").checked }, $("ex-start")); });
$("ex-stop").addEventListener("click", function () {
  askConfirm({ title: "Stop the emergency withdrawal?", body: "Exits that can still be stopped are stopped. Any that are too far along carry on, and network fees already paid are not returned.", cancel: "Keep going", ok: "Stop" })
    .then(function (yes) { if (yes) exAction("/admin/exit/cancel", {}, $("ex-stop")); });
});
$("ex-claim").addEventListener("click", function () { exAction("/admin/exit/claim", {}, $("ex-claim")); });
$("ex-left").addEventListener("click", function () { exAction("/admin/exit/leftover", {}, $("ex-left"), "exl"); });
// Turning the emergency withdrawal on or off (it is off until the owner turns it on).
function exOptIn(on, pwId, codeId, btn) {
  btn.disabled = true;
  api("POST", "/admin/exit/optin", { on: on, admin_password: $(pwId).value, code: $(codeId).value.trim() }).then(function (r) {
    $(pwId).value = ""; $(codeId).value = "";
    say("exo-msg", r.message || "Done.");
    loadExit(); loadPayout();
  }).catch(function (e) { say("exo-msg", e.message, true); }).then(function () { btn.disabled = false; });
}
$("exo-on").addEventListener("click", function () { exOptIn(true, "exo-pw", "exo-code", $("exo-on")); });
$("exo-off-go").addEventListener("click", function () {
  askConfirm({ title: "Turn off the emergency withdrawal?", body: "Everything will be sent on to your payout address as usual, and the fee money can be taken back. You can turn it on again any time.", cancel: "Keep it on", ok: "Turn off" })
    .then(function (yes) { if (yes) exOptIn(false, "exoff-pw", "exoff-code", $("exo-off-go")); });
});

boot();
})();
