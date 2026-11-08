# Branta (planned for a much later release)

**Status: an idea only. Nothing is built, and nothing here is promised.** It sits
after WooCommerce is thoroughly tested, alongside [Shopify](SHOPIFY.md).

## What it is, and why we like it

Branta (<https://github.com/BrantaOps>) makes tools that let a person check, before
paying, that a Bitcoin payment address really belongs to the shop they think they
are paying. Its merchant product is called **Guardrail**, a "pre-payment
verification plug-in". In the one merchant case study we read, the checkout looks
the same except for a small "Click to verify with Branta" link; clicking it opens a
page showing whether the payment will go to the correct address. Branta also has
wallet-side pieces (a Windows app with wallet verification, and a partnership with
the Manna wallet showing verified merchant details), and a Python package on PyPI.

Why it fits EBP: our biggest remaining risk is a tampered or hijacked server or
checkout page showing customers the wrong address. Today only the shop owner's
weekly check against their own wallet catches that. A second, independent place
where the customer can confirm "this address is really this shop's" would help
customers feel safer and would make that attack harder, which matches EBP's goal of
better security and easier use together.

## What we would want, in plain words

- A small "Verify this payment" option on the EBP checkout page, in the same quiet
  style as Branta's own.
- The shop's payment address registered with Branta when a payment request is
  created, so a customer's wallet or the Branta page can confirm it.
- Off by default and optional, with a plain explanation of what is shared.
- Technical names (and any standards involved) kept in the background, per our
  [design principle](../../CONTRIBUTING.md).

## Open questions (nothing below has been checked)

- **Privacy:** what Branta learns about each payment (address, amount, shop) and
  whether that fits "no company in the middle".
- **Trust and sovereignty:** does it depend on Branta's service being up and honest,
  and what happens to checkout if it is down? It must never block a payment.
- **Cost and licence:** the pages we read say nothing about pricing or licences, and
  the code's licences have not been checked.
- **Integration route:** the case study names BTCPay Server only; we found no
  WooCommerce or Shopify plugin. We would need to call their service or library
  ourselves, or ask them.
- **Maturity and ownership:** how stable and widely used it is, and who runs it.
- **Our own part:** whether the address EBP shows can be signed or published in a way
  that does not need Branta at all (a self-hosted version of the same idea).

## Next steps when we get there

1. Read Branta's docs and code licences properly and talk to the team.
2. Prototype on signet only, behind a switch, with a failure that never blocks checkout.
3. Decide privacy wording, then write it into the checkout and the guide.

## Sources (read briefly, not in depth)

- Branta on GitHub: https://github.com/BrantaOps (the page itself would not load for
  us, so the repository list was not reviewed)
- Case study, merchants using Guardrail: https://brantaops.substack.com/p/case-study-merchants-implement-brantas
- Manna wallet partnership: https://bitcoinmagazine.com/business/manna-wallet-branta-guardrails-self-custodial-bitcoin-payments-now-show-verified-merchant-details
- Windows wallet verification: https://www.github.com/BrantaOps/branta-windows
