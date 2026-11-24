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
pwHelper("admin-pw", "admin-pw-show", "admin-pw-count");
(function () {
  "use strict";

  var els = {
    testBanner: document.getElementById("test-banner"),
    steps: [
      document.getElementById("s1"),
      document.getElementById("s2"),
      document.getElementById("s3"),
      document.getElementById("s4")
    ],
    https: document.getElementById("step-https"),
    code: document.getElementById("step-code"),
    codeInput: document.getElementById("code"),
    btnCode: document.getElementById("btn-code"),
    codeError: document.getElementById("code-error"),
    wallet: document.getElementById("step-wallet"),
    asp: document.getElementById("asp"),
    network: document.getElementById("network"),
    btnWallet: document.getElementById("btn-wallet"),
    walletError: document.getElementById("wallet-error"),
    restore: document.getElementById("step-restore"),
    restorefile: document.getElementById("step-restorefile"),
    backup: document.getElementById("step-backup"),
    words: document.getElementById("words"),
    wordsManual: document.getElementById("words-manual"),
    btnShowWords: document.getElementById("btn-show-words"),
    ceremonyNote: document.getElementById("words-ceremony-note"),
    backupCheck: document.getElementById("backup-check"),
    btnBackup: document.getElementById("btn-backup"),
    backupError: document.getElementById("backup-error"),
    token: document.getElementById("step-token"),
    tokenInput: document.getElementById("token"),
    btnToken: document.getElementById("btn-token"),
    tokenNote: document.getElementById("token-note"),
    tokenError: document.getElementById("token-error"),
    store: document.getElementById("step-store"),
    storeName: document.getElementById("store"),
    btnStore: document.getElementById("btn-store"),
    storeSecrets: document.getElementById("store-secrets"),
    apiKey: document.getElementById("api-key"),
    webhookSecret: document.getElementById("webhook-secret"),
    btnDone: document.getElementById("btn-done"),
    storeError: document.getElementById("store-error"),
    done: document.getElementById("step-done")
  };

  var sections = ["https", "code", "wallet", "restore", "restorefile", "backup", "token", "store", "done"];
  var skipRestoreFile = false;

  function show(name) {
    sections.forEach(function (section) {
      els[section].classList.toggle("hidden", section !== name);
    });
  }

  function markStep(step) {
    els.steps.forEach(function (element, index) {
      element.className = index < step ? "done" : index === step ? "now" : "";
    });
    // The bars in words too: "Step 2 of 4", or "All set" at the end.
    var n = els.steps.length, label = document.getElementById("step-label");
    if (label) label.textContent = step >= n ? "All set" : "Step " + (step + 1) + " of " + n;
  }

  // Payout wallet picker: choosing a kind shows the address box with the right
  // hint. The server still checks the address properly; this only catches the
  // obvious mix-up (an Ark address in the on-chain box, or the reverse).
  // Wording follows the server's mode (set in resume()): a play-money server
  // only takes tark1/tb1 addresses, a real-money one only ark1/bc1/1/3.
  var payoutHintsFor = {
    test: {
      ark: { label: "Your Ark address", ph: "tark1…",
        hint: "In your Ark wallet (Arké or Noah, on signet) open Receive and copy the Ark address. On this play-money server it starts with tark1." },
      onchain: { label: "Your Bitcoin address", ph: "tb1…",
        hint: "In your signet wallet open Receive and copy a fresh address. On this play-money server it starts with tb1. Use a wallet you control, not an exchange deposit address." }
    },
    live: {
      ark: { label: "Your Ark address", ph: "ark1…",
        hint: "In your Ark wallet (Arké or Noah) open Receive and copy the Ark address. It starts with ark1. The wallet must use Second's Ark server (ark.second.tech)." },
      onchain: { label: "Your Bitcoin address", ph: "bc1… (or 1… / 3…)",
        hint: "In your wallet (Sparrow, Nunchuk or Liana) open Receive and copy a fresh address. It starts with bc1 (older wallets: 1 or 3). Use a wallet you control, not an exchange deposit address." }
    }
  };
  var payoutHints = payoutHintsFor.test;
  var arkPrefixText = "tark1";
  function payoutKind() { return document.getElementById("payout-kind").value; }
  function looksArk(a) { return /^t?ark1/i.test(a); }
  function payoutValue() {
    var kind = payoutKind(), a = document.getElementById("payout").value.trim();
    return kind && a ? a : "";
  }
  document.getElementById("payout-kind").addEventListener("change", function () {
    var kind = payoutKind(), box = document.getElementById("payout-box");
    if (!kind) { box.classList.add("hidden"); return; }
    var h = payoutHints[kind];
    document.getElementById("payout-label").textContent = h.label;
    document.getElementById("payout").placeholder = h.ph;
    document.getElementById("payout-hint").textContent = h.hint;
    box.classList.remove("hidden");
  });

  // Check the address with the server as soon as it is typed. The answer is
  // advice, not a gate: "failed" shows a danger box with a tick-box, and
  // ticking it lets the owner go on (for example while testing).
  var payoutVerdict = null, payoutVerdictFor = "", payoutCheckSeq = 0;
  function showVerdict(v) {
    var line = document.getElementById("payout-verdict"), risk = document.getElementById("payout-risk");
    line.classList.add("hidden"); risk.classList.add("hidden");
    line.style.color = "";
    if (!v) return;
    if (v.verdict === "failed") {
      // The server's refusal reads as a sentence fragment ("that is a test-network
      // address ... on iPhone"): start it with a capital and end it with a stop so
      // the text after it does not run on.
      var why = String(v.message || "").trim();
      why = why.charAt(0).toUpperCase() + why.slice(1);
      if (!/[.!?]$/.test(why)) why += ".";
      document.getElementById("payout-risk-why").textContent = " " + why;
      risk.classList.remove("hidden");
    } else {
      line.textContent = (v.verdict === "verified" ? "\u2714 " : "\u26A0 ") + v.message;
      line.style.color = v.verdict === "verified" ? "var(--ok)" : "var(--warn)";
      line.classList.remove("hidden");
    }
  }
  function checkPayout() {
    var addr = document.getElementById("payout").value.trim(), seq = ++payoutCheckSeq;
    document.getElementById("payout-risk-ok").checked = false;
    if (!payoutKind() || !addr) { payoutVerdict = null; payoutVerdictFor = ""; showVerdict(null); return Promise.resolve(null); }
    return api("POST", "/setup/check-payout", { address: addr }).then(function (v) {
      if (seq !== payoutCheckSeq) return payoutVerdict;
      payoutVerdict = v; payoutVerdictFor = addr; showVerdict(v); return v;
    }).catch(function () {
      if (seq !== payoutCheckSeq) return payoutVerdict;
      // Could not even ask: say so, never "verified".
      payoutVerdict = { verdict: "unverified", message: "Could not verify: the check did not run, so we do not know whether this address is valid." };
      payoutVerdictFor = addr; showVerdict(payoutVerdict); return payoutVerdict;
    });
  }
  document.getElementById("payout").addEventListener("change", checkPayout);
  document.getElementById("payout-kind").addEventListener("change", checkPayout);

  function fail(element, message) {
    element.textContent = message;
    element.classList.remove("hidden");
  }

  function clearFail(element) {
    element.classList.add("hidden");
  }

  // Setup code (generated at boot, or EBP_SETUP_TOKEN): the status call says
  // when one is needed; we ask on the page (no browser pop-up), verify it
  // right away, and send it with every wizard action. Kept in memory only.
  var setupToken = "";
  var lastStatus = null;

  // The server sends errors as {error: {code, message}} (API routes) or as
  // {error: "code", message: "..."} (setup-code check). Always return text.
  function errorText(data) {
    if (!data) return "";
    if (typeof data.message === "string" && data.message) return data.message;
    var e = data.error;
    if (e && typeof e === "object" && typeof e.message === "string") return e.message;
    return typeof e === "string" ? e : "";
  }

  function api(method, path, body, retried) {
    var headers = { "content-type": "application/json", "accept": "application/json" };
    if (setupToken) headers["x-ebp-setup-token"] = setupToken;
    return fetch(path, {
      method: method,
      headers: headers,
      body: body ? JSON.stringify(body) : undefined
    }).catch(function () { throw new Error("Can't reach your server right now. Check your internet connection and that the server is running, then try again."); }).then(function (res) {
      return res.json().catch(function () { return {}; }).then(function (data) {
        if (res.status === 401 && data.error === "setup_token_required") {
          // Wrong or missing code: go back to the code screen.
          setupToken = "";
          if (lastStatus) {
            show("code");
            // The code changes every time the server restarts; say so instead of just bouncing the page.
            fail(els.codeError, "Your setup code is no longer valid (it changes whenever the server restarts). That's OK, your progress is saved. Enter the new code shown in your payment server's Console.");
          }
        }
        if (!res.ok) {
          throw new Error(errorText(data) || (res.status === 429 ? "Too many tries. Wait a minute, then try again." : res.status >= 500 ? "The server had a problem. Try again in a minute; if it keeps happening, see Known issues in the docs." : "The server did not accept that (error " + res.status + ")."));
        }
        return data;
      });
    });
  }

  els.btnCode.addEventListener("click", function () {
    clearFail(els.codeError);
    setupToken = els.codeInput.value.trim();
    if (!setupToken) { fail(els.codeError, "Type the setup code first."); return; }
    api("POST", "/setup/verify-code", {})
      .then(function () { resume(lastStatus, true); })
      .catch(function () {
        setupToken = "";
        fail(els.codeError, "That code did not match. Check the SETUP CODE shown where you started the server.");
      });
  });

  // The banner and address wording follow the server's mode: the network
  // chosen on this page once the wallet exists, else how it was started.
  function applyMode(mode) {
    els.testBanner.textContent = mode === "live"
      ? "REAL MONEY. Experimental: keep amounts small."
      : "TEST MODE: play money (signet). Nothing here is real.";
    els.testBanner.classList.remove("hidden");
    payoutHints = mode === "live" ? payoutHintsFor.live : payoutHintsFor.test;
    arkPrefixText = mode === "live" ? "ark1" : "tark1";
    document.getElementById("payout-risk-test").classList.toggle("hidden", mode === "live");
    document.getElementById("payout-risk-live").classList.toggle("hidden", mode !== "live");
  }
  var networkTouched = false;
  var mainnetOk = document.getElementById("mainnet-ok");
  function networkChanged() {
    var main = els.network.value === "mainnet";
    els.asp.value = main ? "https://ark.second.tech" : "https://ark.signet.2nd.dev";
    document.getElementById("mainnet-warn").classList.toggle("hidden", !main);
    document.getElementById("mainnet-ok-box").classList.toggle("hidden", !main);
    if (!main) mainnetOk.checked = false;
    // Before the wallet exists, the banner previews the choice.
    if (!lastStatus || !lastStatus.wallet_created) applyMode(chosenMode());
  }
  function chosenMode() { return els.network.value === "mainnet" ? "live" : "test"; }
  function mainnetBlocked(errBox) {
    if (els.network.value === "mainnet" && !mainnetOk.checked) {
      fail(errBox, "You chose mainnet (real bitcoin). Tick the box to confirm you understand this server stays on mainnet for good, or choose signet.");
      return true;
    }
    return false;
  }
  els.network.addEventListener("change", function () { networkTouched = true; networkChanged(); });

  var autoTokenTried = false;
  function resume(status, codeOk) {
    lastStatus = status;
    if (status.https_required) { markStep(0); show("https"); return; }
    if (status.setup_code_required && !codeOk && !setupToken) { markStep(0); show("code"); return; }
    applyMode(status.mode);
    // Before the wallet exists the owner chooses the network; signet is the
    // default unless the server was started in live mode. After that the
    // choice is fixed.
    if (!status.wallet_created && !networkTouched) {
      els.network.value = status.mode === "live" ? "mainnet" : "signet";
      networkChanged();
    }
    if (status.setup_complete) { markStep(4); show("done"); return; }
    if (!status.wallet_created) { markStep(0); show("wallet"); return; }
    if (!status.backup_confirmed) {
      markStep(1);
      show("backup");
      els.words.classList.add("hidden");
      els.btnShowWords.classList.remove("hidden");
      if (!status.managed) els.ceremonyNote.classList.remove("hidden");
      els.wordsManual.textContent = status.managed
        ? "Tap \"Show my recovery words\". They are shown once and never stored. Write them down on paper before you confirm."
        : "Tap \"Show my recovery words\". They are shown once and never stored or logged. Write them down on paper before you confirm.";
      els.wordsManual.classList.remove("hidden");
      return;
    }
    // (A separate wallet container needs its token step first: the backup
    // marks setup as finished, so that step would never come round again.)
    if (status.wallet_restored && !skipRestoreFile && (status.managed || status.config_written)) { markStep(2); show("restorefile"); return; }
    if (!status.config_written && status.managed && !autoTokenTried) {
      // Managed install: the server already holds the wallet token; no
      // copying needed. Fall back to the manual step if this fails.
      autoTokenTried = true;
      api("POST", "/setup/token", {})
        .then(function () { return api("GET", "/setup/status"); })
        .then(function (s) { resume(s, true); })
        .catch(function () { resume(status, true); });
      return;
    }
    if (!status.config_written) { markStep(2); show("token"); return; }
    markStep(3);
    show("store");
  }

  api("GET", "/setup/status")
    .then(resume)
    .catch(function (err) {
      show("wallet");
      fail(els.walletError, "Could not read the setup status: " + err.message);
    });

  // The words are on screen only while "revealed" (60 s at most) and are
  // wiped from the page the moment they are hidden. This page keeps NO copy:
  // the quiz is checked by the server, which re-reads the words from barkd.
  var wordCount = 0;
  var revealTimer = null;
  function hideWords() {
    if (revealTimer) { clearInterval(revealTimer); revealTimer = null; }
    els.words.textContent = "";
    els.words.classList.add("hidden");
    document.getElementById("reveal-bar").classList.add("hidden");
    els.btnShowWords.classList.remove("hidden");
  }
  function renderWords(mnemonic) {
    var list = mnemonic.trim().split(/\s+/);
    wordCount = list.length;
    els.words.textContent = "";
    list.forEach(function (word, index) {
      var item = document.createElement("span");
      item.textContent = (index + 1) + ". " + word;
      els.words.appendChild(item);
    });
    els.words.classList.remove("hidden");
    els.btnShowWords.classList.add("hidden");
    var left = 60;
    var label = document.getElementById("reveal-left");
    label.textContent = "Hiding in " + left + " seconds. ";
    document.getElementById("reveal-bar").classList.remove("hidden");
    if (revealTimer) clearInterval(revealTimer);
    revealTimer = setInterval(function () {
      left -= 1;
      if (left <= 0) { hideWords(); return; }
      label.textContent = "Hiding in " + left + " seconds. ";
    }, 1000);
  }
  document.getElementById("btn-hide").addEventListener("click", hideWords);
  document.getElementById("safe-check").addEventListener("change", function () {
    els.btnShowWords.disabled = !this.checked;
  });

  els.btnWallet.addEventListener("click", function () {
    clearFail(els.walletError);
    if (mainnetBlocked(els.walletError)) return;
    els.btnWallet.disabled = true;
    els.btnWallet.textContent = "Creating your wallet\u2026 this can take a minute";
    api("POST", "/setup/wallet", {
      asp_url: els.asp.value.trim(),
      network: els.network.value,
      mainnet_understood: mainnetOk.checked
    })
      .then(function (result) {
        applyMode(chosenMode());
        if (result.mnemonic) {
          renderWords(result.mnemonic);
        } else {
          // The create response never carries the words (verified barkd
          // 0.8.0): the ceremony button is the way to see them.
          els.words.classList.add("hidden");
          els.btnShowWords.classList.remove("hidden");
          els.ceremonyNote.classList.remove("hidden");
          els.wordsManual.textContent = result.manual_instructions ||
            "Tap \"Show my recovery words\" below. They are shown once. Write them down on paper before you confirm.";
          els.wordsManual.classList.remove("hidden");
        }
        markStep(1);
        show("backup");
      })
      .catch(function (err) { fail(els.walletError, err.message); })
      .finally(function () { els.btnWallet.disabled = false; els.btnWallet.textContent = "Create the wallet"; });
  });

  // Restore from the 12 words. The words live only in this box and in one
  // request; the box is wiped as soon as the request is sent.
  var restoreWords = document.getElementById("restore-words");
  var restorePaper = document.getElementById("restore-paper");
  var btnRestore = document.getElementById("btn-restore");
  var restoreError = document.getElementById("restore-error");
  restorePaper.addEventListener("change", function () { btnRestore.disabled = !this.checked; });
  // Count the words as they are typed or pasted (spaces and line breaks both separate words).
  function restoreWordCount() { return restoreWords.value.split(/\s+/).filter(Boolean).length; }
  function updateRestoreCount() {
    var n = restoreWordCount(), el = document.getElementById("restore-count");
    el.textContent = n + " of 12 words";
    el.style.color = n === 12 ? "var(--ok)" : "";
  }
  restoreWords.addEventListener("input", updateRestoreCount);
  document.getElementById("btn-go-restore").addEventListener("click", function () {
    clearFail(els.walletError);
    syncRestoreKind();
    show("restore");
  });
  // The old server's kind of money is the one choice that cannot be undone, so it is asked in plain words.
  // It drives the same Network box as the previous screen.
  function syncRestoreKind() {
    var main = els.network.value === "mainnet";
    document.getElementById("restore-kind-signet").checked = !main;
    document.getElementById("restore-kind-mainnet").checked = main;
    document.getElementById("restore-mainnet-box").classList.toggle("hidden", !main);
    document.getElementById("restore-mainnet-ok").checked = mainnetOk.checked;
    document.getElementById("restore-net").textContent = main
      ? "Restoring a real-money wallet (mainnet). Not sure? Your old dashboard showed a TEST MODE banner if it was play money."
      : "Restoring a play-money wallet (signet). Not sure? Your old dashboard showed a TEST MODE banner if it was play money.";
  }
  document.querySelectorAll("input[name=restore-kind]").forEach(function (r) {
    r.addEventListener("change", function () {
      els.network.value = r.value; networkTouched = true; networkChanged(); syncRestoreKind();
    });
  });
  document.getElementById("restore-mainnet-ok").addEventListener("change", function () { mainnetOk.checked = this.checked; });
  document.getElementById("btn-restore-back").addEventListener("click", function () {
    restoreWords.value = ""; updateRestoreCount(); clearFail(restoreError); show("wallet");
  });
  btnRestore.addEventListener("click", function () {
    clearFail(restoreError);
    var words = restoreWords.value;
    if (!words.trim()) { fail(restoreError, "Type your recovery words first."); return; }
    var wc = restoreWordCount();
    if (wc !== 12) { fail(restoreError, "That's " + wc + " word" + (wc === 1 ? "" : "s") + ". Recovery words are always 12. Check your paper for a missed or extra word."); return; }
    if (mainnetBlocked(restoreError)) return;
    btnRestore.disabled = true;
    btnRestore.textContent = "Restoring… this can take a minute";
    api("POST", "/setup/wallet", {
      asp_url: els.asp.value.trim(),
      network: els.network.value,
      mainnet_understood: mainnetOk.checked,
      mnemonic: words,
      paper_copies_exist: restorePaper.checked
    })
      .then(function () {
        restoreWords.value = ""; updateRestoreCount();
        return api("GET", "/setup/status");
      })
      .then(function (s) { resume(s, true); })
      .catch(function (err) { fail(restoreError, err.message); })
      .finally(function () {
        btnRestore.textContent = "Restore my wallet";
        btnRestore.disabled = !restorePaper.checked;
      });
  });

  // Bring back a backup file: 4 bytes (password length), the password, then
  // the file, in one request. The server stages it and restarts; we then wait
  // for it to come back and show the last step.
  var rfError = document.getElementById("rf-error");
  var btnRf = document.getElementById("btn-rf");
  var rfWorking = document.getElementById("rf-working");
  document.getElementById("btn-rf-skip").addEventListener("click", function () {
    skipRestoreFile = true; clearFail(rfError); resume(lastStatus, true);
  });
  function waitForRestart() {
    rfWorking.textContent = "Restored. The server is restarting to use your backup; this takes about a minute…";
    rfWorking.classList.remove("hidden");
    var tries = 0;
    var timer = setInterval(function () {
      tries += 1;
      fetch("/setup/status", { headers: { "accept": "application/json" } })
        .then(function (r) { return r.ok ? r.json() : null; })
        .then(function (s) {
          if (s && s.restore_refused) {
            clearInterval(timer);
            rfWorking.classList.add("hidden");
            fail(rfError, "The server restarted but did not use your backup: " + s.restore_refused + ". Nothing was deleted. To try again, reload this page; you will need the new setup code shown in your payment server's Console.");
          } else if (s && s.setup_complete) {
            clearInterval(timer);
            document.getElementById("done-restored").classList.remove("hidden");
            markStep(4); show("done");
          } else if (tries > 100) {
            clearInterval(timer);
            fail(rfError, "The server is taking longer than expected to restart. Wait a minute, then reload this page. You'll need the new setup code shown in your payment server's Console.");
          }
        })
        .catch(function () { /* still restarting */ });
    }, 3000);
  }
  btnRf.addEventListener("click", function () {
    clearFail(rfError);
    var file = document.getElementById("rf-file").files[0];
    var pw = document.getElementById("rf-pw").value;
    if (!file) { fail(rfError, "Choose your backup file first."); return; }
    if (!pw) { fail(rfError, "Type the password for that backup."); return; }
    var pwBytes = new TextEncoder().encode(pw);
    var head = new Uint8Array(4);
    new DataView(head.buffer).setUint32(0, pwBytes.length, false);
    var headers = { "content-type": "application/octet-stream", "accept": "application/json" };
    if (setupToken) headers["x-ebp-setup-token"] = setupToken;
    btnRf.disabled = true;
    btnRf.textContent = "Restoring… this can take a minute";
    fetch("/setup/restore-backup", { method: "POST", headers: headers, body: new Blob([head, pwBytes, file]) })
      .catch(function () { throw new Error("Can't reach your server right now. Check that it is running, then try again."); })
      .then(function (res) {
        return res.json().catch(function () { return {}; }).then(function (data) {
          if (!res.ok) throw new Error(errorText(data) || (res.status === 429 ? "Too many tries. Wait a minute, then try again." : "The server did not accept that (error " + res.status + ")."));
          return data;
        });
      })
      .then(function () {
        document.getElementById("rf-pw").value = "";
        btnRf.classList.add("hidden");
        document.getElementById("btn-rf-skip").classList.add("hidden");
        waitForRestart();
      })
      .catch(function (err) { fail(rfError, err.message); btnRf.disabled = false; btnRf.textContent = "Restore from this backup"; });
  });

  // The expose-mnemonic ceremony: reads the words straight from barkd
  // (which must be running with --expose-mnemonic per the wizard's
  // instruction) and shows them exactly once. The endpoint itself enforces
  // once-only: it refuses after the backup is confirmed or setup completes.
  els.btnShowWords.addEventListener("click", function () {
    clearFail(els.backupError);
    els.btnShowWords.disabled = true;
    api("POST", "/setup/wallet/mnemonic", {})
      .then(function (result) {
        if (result.mnemonic) {
          renderWords(result.mnemonic);
          els.wordsManual.textContent = result.instructions || "";
          els.wordsManual.classList.remove("hidden");
          els.ceremonyNote.classList.add("hidden");
        } else {
          els.words.classList.add("hidden");
          els.wordsManual.textContent = result.instructions ||
            "The words are not exposed yet — follow the command in the note above, then tap the button again.";
          els.wordsManual.classList.remove("hidden");
        }
      })
      .catch(function (err) { fail(els.backupError, err.message); })
      .finally(function () { els.btnShowWords.disabled = !document.getElementById("safe-check").checked; });
  });

  els.backupCheck.addEventListener("change", function () {
    els.btnBackup.disabled = !els.backupCheck.checked;
  });

  function startQuiz() {
    // Three random, distinct positions, in order.
    var picks = [];
    var buf = new Uint32Array(32);
    window.crypto.getRandomValues(buf);
    for (var k = 0; k < buf.length && picks.length < 3; k++) {
      var n = (buf[k] % wordCount) + 1;
      if (picks.indexOf(n) < 0) picks.push(n);
    }
    picks.sort(function (a, b) { return a - b; });
    var box = document.getElementById("quiz-fields");
    box.textContent = "";
    picks.forEach(function (pos) {
      var label = document.createElement("label");
      label.textContent = "Word number " + pos;
      var input = document.createElement("input");
      // Password-style: phone keyboards do not learn or suggest the words,
      // and nobody can read them over a shoulder.
      input.type = "password";
      input.autocomplete = "off";
      input.autocapitalize = "none";
      input.setAttribute("autocorrect", "off");
      input.spellcheck = false;
      input.setAttribute("data-pos", String(pos));
      box.appendChild(label);
      box.appendChild(input);
    });
    els.btnBackup.classList.add("hidden");
    document.getElementById("quiz").classList.remove("hidden");
  }

  els.btnBackup.addEventListener("click", function () {
    clearFail(els.backupError);
    if (!wordCount) {
      fail(els.backupError, "Show your recovery words first, write them down, then check your backup.");
      return;
    }
    hideWords();
    startQuiz();
  });

  document.getElementById("btn-quiz-back").addEventListener("click", function () {
    clearFail(els.backupError);
    document.getElementById("quiz-fields").textContent = "";
    document.getElementById("quiz").classList.add("hidden");
    els.btnBackup.classList.remove("hidden");
  });

  document.getElementById("btn-quiz").addEventListener("click", function () {
    clearFail(els.backupError);
    var answers = [];
    document.querySelectorAll("#quiz-fields input").forEach(function (input) {
      answers.push({ position: Number(input.getAttribute("data-pos")), word: input.value });
    });
    var button = document.getElementById("btn-quiz");
    button.disabled = true;
    api("POST", "/setup/wallet/verify-backup", { answers: answers })
      .then(function (r) {
        if (!r.verified) {
          fail(els.backupError, "Those don't match. Check the numbers against your paper copy, or tap \"Show the words again\" and re-copy them.");
          return null;
        }
        return api("POST", "/setup/wallet/confirm-backup", { confirmed: true }).then(function () {
          document.getElementById("quiz-fields").textContent = "";
          document.getElementById("quiz").classList.add("hidden");
          hideWords();
          els.wordsManual.textContent = "";
          els.btnShowWords.classList.add("hidden");
          // Re-read the status and continue from there: on the single-container
          // install the server already holds barkd's token and applies it
          // itself (resume), so the merchant never sees a token step.
          return api("GET", "/setup/status")
            .then(function (s) { resume(s, true); })
            .catch(function () { markStep(2); show("token"); });
        });
      })
      .catch(function (err) { fail(els.backupError, err.message); })
      .finally(function () {
        button.disabled = false;
        document.querySelectorAll("#quiz-fields input").forEach(function (i) { i.value = ""; });
      });
  });

  els.btnToken.addEventListener("click", function () {
    clearFail(els.tokenError);
    var token = els.tokenInput.value.trim();
    if (!token) {
      fail(els.tokenError, "Paste the token from `barkd secret show` first.");
      return;
    }
    els.btnToken.disabled = true;
    api("POST", "/setup/token", { token: token })
      .then(function (result) {
        els.tokenNote.textContent = result.verified
          ? ("Verified. " + (result.restart_required
              ? "Now add EBP_BARKD_TOKEN to your deployment environment (docker-compose environment or systemd unit) and restart the payments server, so the token survives restarts. The wizard's copy works until then."
              : "The token is already in this server's environment — nothing more to do."))
          : "Not verified.";
        els.tokenNote.classList.remove("hidden");
        els.tokenInput.value = "";
        markStep(3);
        show("store");
      })
      .catch(function (err) { fail(els.tokenError, err.message); })
      .finally(function () { els.btnToken.disabled = false; });
  });

  els.btnStore.addEventListener("click", function () {
    clearFail(els.storeError);
    var name = els.storeName.value.trim();
    if (!name) {
      fail(els.storeError, "Give the store a name first.");
      return;
    }
    var pw = document.getElementById("admin-pw").value;
    if (pw.length < 16) {
      fail(els.storeError, "Choose an admin password of at least 16 characters (a few random words works well).");
      return;
    }
    var kind = payoutKind(), addr = document.getElementById("payout").value.trim();
    if (!kind) {
      fail(els.storeError, "Choose where your money should go: an Ark wallet or an on-chain Bitcoin wallet.");
      return;
    }
    if (!addr) {
      fail(els.storeError, "Paste your wallet's address to continue. It is where your earnings are sent.");
      return;
    }
    if (kind === "ark" && addr && !looksArk(addr)) {
      fail(els.storeError, "That does not look like an Ark address (on this server they start with " + arkPrefixText + "). If it is a regular Bitcoin address, choose the on-chain option instead.");
      return;
    }
    if (kind === "onchain" && addr && looksArk(addr)) {
      fail(els.storeError, "That looks like an Ark address. Choose the Ark option instead.");
      return;
    }
    els.btnStore.disabled = true;
    els.btnStore.textContent = "Creating your store\u2026 this can take a minute";
    var checked = payoutVerdictFor === addr ? Promise.resolve(payoutVerdict) : checkPayout();
    checked.then(function (v) {
      if (v && v.verdict === "failed" && !document.getElementById("payout-risk-ok").checked) {
        els.btnStore.disabled = false; els.btnStore.textContent = "Create the store";
        fail(els.storeError, "This address failed a check. Fix it, or tick the box under the address to use it anyway.");
        return null;
      }
      return api("POST", "/setup/store", { name: name, admin_password: pw, payout_address: payoutValue(), accept_risky_payout: !!(v && v.verdict === "failed") });
    })
      .then(function (result) {
        if (!result) return;
        document.getElementById("admin-pw").value = "";
        document.getElementById("admin-pw").type = "password";
        document.getElementById("admin-pw-show").textContent = "Show";
        if (result.api_key) {
          els.apiKey.textContent = result.api_key;
          els.webhookSecret.textContent = result.webhook_secret || "";
          document.getElementById(result.webhook_secret ? "webhook-secret-box" : "webhook-secret-later").classList.remove("hidden");
          els.storeSecrets.classList.remove("hidden");
          els.btnStore.classList.add("hidden");
          els.btnDone.classList.remove("hidden");
        } else {
          markStep(4);
          show("done");
        }
      })
      .catch(function (err) { fail(els.storeError, err.message); })
      .finally(function () { els.btnStore.disabled = false; els.btnStore.textContent = "Create the store"; });
  });

  // Copy buttons next to one-time secrets (the text is read from the page at click time).
  document.addEventListener("click", function (e) {
    var b = e.target && e.target.closest && e.target.closest("[data-copy]");
    if (!b) return;
    var src = document.getElementById(b.getAttribute("data-copy"));
    if (!src || !src.textContent || !navigator.clipboard) return;
    var label = b.getAttribute("data-label") || b.textContent;
    b.setAttribute("data-label", label);
    navigator.clipboard.writeText(src.textContent).then(function () {
      b.textContent = "Copied";
      setTimeout(function () { b.textContent = label; }, 1500);
    }, function () {});
  });

  els.btnDone.addEventListener("click", function () {
    // The API key and webhook secret were shown exactly once: blank them
    // here so they do not linger in the DOM (or bfcache) after this step.
    els.apiKey.textContent = "";
    els.webhookSecret.textContent = "";
    els.storeSecrets.classList.add("hidden");
    markStep(4);
    show("done");
  });

  // Back/forward cache restore would resurrect any secret still in the
  // DOM: wipe them on every persisted pageshow.
  window.addEventListener("pageshow", function (event) {
    if (event.persisted) {
      if (typeof hideWords === "function") hideWords();
      if (els.words) { els.words.textContent = ""; els.words.classList.add("hidden"); }
      if (els.wordsManual) { els.wordsManual.textContent = ""; }
      var rw = document.getElementById("restore-words"); if (rw) { rw.value = ""; updateRestoreCount(); }
      var rp = document.getElementById("rf-pw"); if (rp) { rp.value = ""; }
      if (els.apiKey) { els.apiKey.textContent = ""; }
      if (els.webhookSecret) { els.webhookSecret.textContent = ""; }
      if (els.storeSecrets) { els.storeSecrets.classList.add("hidden"); }
    }
  });
})();
