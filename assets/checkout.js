(function () {
"use strict";
var id = decodeURIComponent(location.pathname.split("/").filter(Boolean).pop() || "");
function $(x) { return document.getElementById(x); }
var els = { method: $("method"), store: $("store-name"), amount: $("amount"), sats: $("sats"), cd: $("countdown"), addr: $("address"), st: $("status"), fee: $("ln-fee"), banner: $("test-banner"), back: $("back"), backLink: $("back-link"), open: $("open-wallet"), copy: $("copy-btn"), help: $("tab-help"), choice: $("ark-choice"), pickLn: $("pick-ln"), pickArk: $("pick-ark"), lead: $("lead") };
// Two ways to pay. Lightning is the default whenever there is a Lightning invoice;
// one box under the code switches to Ark. There is no regular (on-chain) Bitcoin
// payment and no combined "one code" link.
var s = { tab: "ark", data: null, ln: null, deadline: 0, poll: null, delay: 1000, t0: Date.now(),
  // part-paid top-up, connection and expiry guards
  part: null, topupDeadline: 0, copyValue: "", fails: 0, lastOk: Date.now(), lost: false, offExp: false };
var WAITING = "Waiting for your payment. This page updates by itself.";
var methodName = { ln: "Lightning", ark: "Ark" };
var helpText = {
  ln: "Lightning is fast and usually costs a tiny fee. Your wallet must support Lightning.",
  ark: "Ark is fast and adds no extra fee. Your wallet must support Ark."
};
var leadText = {
  ln: "Scan this code with your wallet app, or tap the button below if you are on your phone.",
  ark: "Scan this code with your wallet app. It needs to support Ark."
};
function setStatus(t, k) { els.st.textContent = t; els.st.className = "status" + (k ? " " + k : ""); }
// A headline, a sentence and an optional note, built from text nodes only.
function setStatusRich(head, body, note, k) {
  els.st.textContent = "";
  var h = document.createElement("strong"); h.textContent = head; els.st.appendChild(h);
  if (body) els.st.appendChild(document.createTextNode(body));
  if (note) { var n = document.createElement("em"); n.textContent = note; els.st.appendChild(n); }
  els.st.className = "status" + (k ? " " + k : "");
}
// QR from the vendored generator (assets/qr.js, same-origin so the CSP holds).
// Rendered as an <img> data URI: no innerHTML. On failure fall back to text.
function drawQr(text) {
  var box = $("qr");
  try {
    if (!text || !window.ebpQrSvg) throw 0;
    var img = document.createElement("img");
    img.alt = "QR code for this payment";
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(window.ebpQrSvg(text));
    box.replaceChildren(img); box.classList.add("on");
  } catch (e) {
    box.textContent = "Use the button below to pay."; box.classList.remove("on");
  }
}
// A link that opens the customer's wallet app, for the chosen way to pay.
// "1,000 sats ($0.40)" when the server knows a dollar price (and the shop is not already pricing in dollars).
function fmtSats(n) {
  n = Number(n);
  var out = n.toLocaleString("en-US") + " sats";
  var rate = s.data && Number(s.data.usd_per_btc);
  if (!(rate > 0) || !isFinite(n) || (s.data.fiat_currency || "").toUpperCase() === "USD") return out;
  var usd = Math.abs(n) / 1e8 * rate;
  return out + " (" + (usd > 0 && usd < 0.01 ? "<$0.01" : "$" + usd.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })) + ")";
}
function walletLink(shown) {
  if (!shown) return "";
  if (s.tab === "ln") return "lightning:" + shown;
  return "";
}
function render() {
  var m = { ln: s.ln && s.ln.bolt11, ark: s.data && s.data.ark_address };
  var shown = (s.data ? m[s.tab] : "") || "";
  els.addr.textContent = shown || "—";
  var link = walletLink(shown);
  // What is scanned, copied and shown is the same thing.
  s.copyValue = shown;
  drawQr(s.copyValue);
  // The two choices (Lightning on top, Ark below) are offered only while there is a
  // Lightning invoice, and not during a top-up (a top-up can only be paid over Ark).
  // Both are always shown; a check mark says which one is selected.
  var offerArk = !!(s.ln && !s.part);
  els.choice.classList.toggle("hidden", !offerArk);
  [["ln", els.pickLn], ["ark", els.pickArk]].forEach(function (p) {
    var on = s.tab === p[0];
    p[1].classList.toggle("on", on);
    p[1].setAttribute("aria-checked", on ? "true" : "false");
  });
  if (s.data) {
    els.lead.textContent = "";
    var lb = document.createElement("b"); lb.textContent = "Pay from your Bitcoin wallet app. ";
    els.lead.appendChild(lb); els.lead.appendChild(document.createTextNode(leadText[s.tab]));
  }
  if (link) { els.open.href = link; els.open.classList.remove("hidden"); } else { els.open.classList.add("hidden"); }
  // Ark: the code has no amount in it, so the payer must type it. Make that the loudest thing on the card.
  var arkDue = s.part ? s.part.remaining : (s.data ? Number(s.data.amount_sats) : 0);
  var showArk = !!shown && s.tab === "ark" && arkDue > 0;
  $("ark-amount").classList.toggle("hidden", !showArk);
  if (showArk) { $("ark-sats").textContent = arkDue.toLocaleString("en-US") + " sats"; s.arkDue = arkDue; }
  if (s.part && shown && s.tab === "ark") {
    els.method.textContent = "Paying the rest with Ark. Your wallet asks for the amount: type in " + s.part.remaining.toLocaleString("en-US") + " sats.";
  } else if (s.part && shown) {
    els.method.textContent = "Paying the rest with " + methodName[s.tab] + ": " + fmtSats(s.part.remaining) + ".";
  } else {
    els.method.textContent = shown ? "Paying with " + methodName[s.tab] + (s.tab === "ln" && offerArk ? ". Does your wallet support Ark instead? Choose \u201cArk (also fast)\u201d above." : ".") : "";
  }
  els.help.textContent = helpText[s.tab] || "";
  els.help.classList.toggle("hidden", !helpText[s.tab] || !offerArk || s.tab === "ln");
  if (s.tab === "ln" && s.ln && typeof s.ln.fee_estimate_sat === "number" && s.ln.fee_estimate_sat > 0) {
    // The Lightning invoice already includes the fee (the customer pays it).
    els.fee.textContent = "Lightning adds a small network fee of about " + fmtSats(s.ln.fee_estimate_sat) +
      ", so your wallet will show " + fmtSats(Number(s.data.amount_sats) + s.ln.fee_estimate_sat) +
      ". The fee goes to the network, not to this store.";
    els.fee.classList.remove("hidden");
  } else { els.fee.classList.add("hidden"); }
}
function select(k) {
  s.tab = k;
  render();
}
els.pickLn.addEventListener("click", function () { select("ln"); });
els.pickArk.addEventListener("click", function () { select("ark"); });
function copyText(t, btn, label) {
  if (!t || t === "—") return;
  var done = function () { btn.textContent = "Copied!"; setTimeout(function () { btn.textContent = label; }, 1500); };
  try { navigator.clipboard.writeText(t).then(done, function () {}); } catch (e) {}
}
// The main button and the visible payment details copy the same thing.
els.addr.addEventListener("click", function () { copyText(els.addr.textContent, els.copy, "Copy payment details"); });
els.copy.addEventListener("click", function () { copyText(s.copyValue || els.addr.textContent, els.copy, "Copy payment details"); });
$("copy-ark-amount").addEventListener("click", function () { copyText(String(s.arkDue || ""), $("copy-ark-amount"), "Copy the number"); });
// Once a payment is finished (paid, expired or cancelled) the pay details go
// away, so nobody sends money to a request that is no longer open.
function closeCard() { $("pay-card").classList.add("hidden"); els.cd.textContent = ""; s.deadline = 0; s.topupDeadline = 0; }
function goBackLabel(text) {
  s.backText = text;
  if (s.back) { els.backLink.textContent = text; els.back.classList.remove("hidden"); }
}
// The big amount: the whole price, or (while part-paid) only what is still missing.
function setTotals() {
  var d = s.data; if (!d) return;
  if (s.part) {
    els.amount.textContent = fmtSats(s.part.remaining) + " to go";
    els.sats.textContent = fmtSats(s.part.received) + " of " + fmtSats(d.amount_sats) + " received";
  } else {
    els.amount.textContent = d.fiat_amount ? d.fiat_currency + " " + d.fiat_amount : fmtSats(d.amount_sats);
    els.sats.textContent = (d.fiat_amount ? "about " : "") + fmtSats(d.amount_sats);
  }
}
fetch("/i/" + encodeURIComponent(id) + "/json", { headers: { "accept": "application/json" } })
  .then(function (r) { return r.ok ? r.json() : null; })
  .then(function (d) {
    if (!d) { setStatus("We could not find this payment. Please go back to the shop and try again.", "err"); return; }
    s.data = d; s.deadline = d.expires_at_unix || 0;
    if (d.lightning && d.lightning.bolt11) s.ln = d.lightning;
    // Lightning first when there is an invoice for it; otherwise Ark is the only way to pay.
    s.tab = s.ln ? "ln" : "ark";
    if (d.test_mode) {
      els.banner.classList.remove("hidden");
    } else if (d.test_mode === false) {
      els.banner.textContent = "REAL MONEY. Experimental: keep amounts small.";
      els.banner.removeAttribute("title");
      els.banner.classList.remove("hidden");
    }
    els.store.textContent = d.store_name || "This store";
    // Only plain http(s) links are ever followed (the server checks too).
    if (d.return_url && /^https?:\/\//i.test(d.return_url)) {
      s.back = d.return_url;
      els.backLink.href = d.return_url;
      // If a status answer already chose the wording (it can beat this one), keep it.
      goBackLabel(s.backText || "Back to " + (d.store_name || "the shop"));
    }
    var satTip = "sats are tiny pieces of a bitcoin: 100 million sats make one bitcoin.";
    setTotals();
    els.sats.title = satTip;
    s.lastOk = Date.now();
    render();
  })
  .catch(function () { setStatus("We could not load this payment. Please check your connection, or go back to the shop and try again.", "err"); });
var EXPIRED_TEXT = "This payment request expired. Don't send money to it: anything sent now won't pay this order. The store can make a new one if you still want your items.";
function expiredOffline() {
  s.offExp = true;
  $("pay-card").classList.add("hidden"); els.cd.textContent = "";
  setStatusRich("This payment request expired.", " Don't send money to it: anything sent now won't pay this order. The store can make a new one if you still want your items.", "", "err");
}
setInterval(function () {
  if (!s.deadline) return;
  var nowMs = Date.now();
  // Past the deadline and the server has not answered for a while: never leave a
  // dead QR payable. If a later answer says the request is still open, it wins.
  if (!s.offExp && nowMs / 1000 > s.deadline + 5 && nowMs - s.lastOk > Math.max(3000, 2 * s.delay + 1000)) { expiredOffline(); return; }
  if (s.offExp) return;
  var end = s.topupDeadline || s.deadline;
  var left = Math.max(0, end - Math.floor(nowMs / 1000));
  els.cd.textContent = left > 0 ? (s.topupDeadline ? "Time left to send the rest: " : "Price held for ") + Math.floor(left / 60) + ":" + ("0" + (left % 60)).slice(-2) : "";
}, 1000);
// Poll /status ~1/s while pending, stop on paid/expired, back off (x2, cap
// 30s) after 5 minutes.
function stop() { if (s.poll) { clearTimeout(s.poll); s.poll = null; } }
function again() {
  if (Date.now() - s.t0 > 300000) s.delay = Math.min(s.delay * 2, 30000);
  s.poll = setTimeout(poll, s.delay);
}
function showLost() {
  if (s.lost || s.offExp) return;
  s.lost = true;
  setStatusRich("Can't reach the store's server. Reconnecting\u2026", " Check your internet connection before sending anything.", "", "warn");
}
function failed() { s.fails++; if (s.fails >= 2 && s.data) showLost(); again(); }
// A part payment: how much is missing, and whether the top-up window is still open.
function showPart(b) {
  var remaining = Number(b.remaining_sats), received = Number(b.received_sats) || 0;
  if (b.topup_window_open === false) {
    // The window has closed: nothing sent now would count, so say so and stop asking for money.
    s.part = null; setTotals(); closeCard();
    setStatusRich("Part of your payment arrived, but the rest didn't come in time.", " Contact the store and quote reference " + id + ". They can refund what you sent or take it off your order.", "", "warn");
    return;
  }
  // A Lightning invoice carries the full amount, so it would overpay: a top-up goes over Ark only.
  // (render() hides the two choices while s.part is set; it runs on every answer, so a page that
  // finishes loading after this still ends up right.)
  s.tab = "ark";
  s.part = { remaining: remaining, received: received };
  s.topupDeadline = Number(b.topup_closes_at_unix) || 0;
  setTotals(); render();
  setStatusRich("Part paid: " + fmtSats(remaining) + " to go.", " Send exactly the rest to this same payment before the timer runs out, and it completes on its own.", "Already sent some? Don't send the full amount again. That would overpay.", "warn");
}
// Success picture: one of four mascots, picked from the payment id (a refresh shows the same one).
// The art is part of the server program (/assets/mascot/*, read-only, fixed list); this code only
// builds image elements with those fixed names and never takes a name or address from anywhere else.
function showMascot() {
  var box = $("mascot");
  if (!box || s.mascot) return;
  s.mascot = true;
  var h = 0;
  for (var i = 0; i < id.length; i++) h = (Math.imul(h, 31) + id.charCodeAt(i)) >>> 0;
  var v = h % 4;
  function img(name, cls) { var e = document.createElement("img"); e.src = "/assets/mascot/" + name + ".png"; e.alt = ""; if (cls) e.className = cls; return e; }
  function node(cls, left, top, width, color, delay) {
    var e = document.createElement("div"); e.className = cls;
    if (left) e.style.left = left; if (top) e.style.top = top; if (width) e.style.width = width;
    if (color) e.style.setProperty("--c", color); if (delay) e.style.animationDelay = delay;
    return e;
  }
  function bill(cls) { var e = node("mc-bill " + cls); e.appendChild(document.createElement("b")); return e; }
  if (v === 0) {
    box.appendChild(img("sir-body")); box.appendChild(img("sir-arm", "mc-arm"));
    box.appendChild(bill("t")); box.appendChild(bill("b")); box.appendChild(node("mc-streak"));
  } else if (v === 1) {
    box.appendChild(img("ufo-beam", "mc-beam")); box.appendChild(img("ufo-top", "mc-bob"));
    box.appendChild(node("mc-coin", "44.1%", null, null, null, "0s")); box.appendChild(node("mc-coin", "51.8%", null, null, null, ".8s")); box.appendChild(node("mc-coin", "47.9%", null, null, null, "1.6s"));
  } else if (v === 2) {
    var bow = node("mc-bow"); bow.appendChild(img("fancy-body")); bow.appendChild(img("fancy-hat", "mc-hat")); box.appendChild(bow);
    box.appendChild(node("mc-star tw", "19.4%", "8.2%", "7.6%", "#ffd23f", ".05s")); box.appendChild(node("mc-star tw", "11.8%", "21.8%", "5.3%", "#ff7a8c", ".25s")); box.appendChild(node("mc-star tw", "84.7%", "5.3%", "5.9%", "#5b3df5", ".15s"));
  } else {
    box.appendChild(img("acro", "mc-acro"));
    box.appendChild(node("mc-star pp", "10.6%", "20.6%", "7%", "#ffd23f", ".1s")); box.appendChild(node("mc-star pp", "85.9%", "32.4%", "5.3%", "#ff7a8c", ".5s")); box.appendChild(node("mc-star pp", "79.4%", "8.8%", "5.9%", "#5b3df5", ".9s"));
  }
  box.setAttribute("data-v", ["sir", "ufo", "fancy", "acrobat"][v]);
  box.classList.remove("hidden");
}
function poll() {
  fetch("/i/" + encodeURIComponent(id) + "/status", { headers: { "accept": "application/json" } })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (b) {
      if (!b) throw 0;
      s.fails = 0; s.lastOk = Date.now();
      if (s.lost || s.offExp) {
        // The server answered, so it decides (this also covers a wrong clock on the phone).
        s.lost = false; s.offExp = false;
        $("pay-card").classList.remove("hidden"); setStatus(WAITING, "");
      }
      if (b.status === "paid" || b.status === "overpaid") {
        s.part = null; setTotals();
        setStatus("Paid. Thank you! " + (b.status === "overpaid" ? "You sent a little extra, so please contact the store about the difference. " : "") + "Bitcoin payments cannot be reversed, so any refund is up to the store.", "ok");
        els.backLink.classList.add("done"); goBackLabel("Done: return to " + (s.data && s.data.store_name ? s.data.store_name : "the shop"));
        closeCard();
        showMascot();
        return stop();
      }
      if (b.status === "underpaid") {
        if (typeof b.remaining_sats === "number" && b.remaining_sats > 0) { showPart(b); }
        else { setStatus("We received part of your payment, but not all of it. Please contact the store so they can help.", "warn"); }
        return again();
      }
      if (b.status === "expired") {
        s.part = null; setTotals();
        setStatus(EXPIRED_TEXT + " If you already paid, contact the store.", "err");
        goBackLabel("Back to " + (s.data && s.data.store_name ? s.data.store_name : "the shop") + " to try again");
        closeCard();
        return stop();
      }
      if (b.status === "cancelled") {
        s.part = null; setTotals();
        setStatus("This payment request was cancelled, so please do not send money to it. Go back to the shop and start a new payment. If you already paid, contact the store.", "err");
        goBackLabel("Back to " + (s.data && s.data.store_name ? s.data.store_name : "the shop") + " to try again");
        closeCard();
        return stop();
      }
      again();
    })
    .catch(failed);
}
poll();
})();
