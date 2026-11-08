# Silent payments (BIP-352): why payouts cannot use them yet

**Short answer:** you cannot set a silent payment address (`sp1…`) as your payout
address today. EBP recognises one if you paste it and tells you so. This page
explains why, and what could change it.

## What a silent payment address is

A silent payment address lets many people (or many payments) pay one static
address while every payment lands on a fresh, unlinkable on-chain address. For a
shop that sweeps to a single fixed address, it would remove the main privacy
cost of the payout design ("every withdrawal goes to the same address",
see [CHOOSING_A_HOST.md](OTHER_HOSTS.md)).

## Why EBP cannot send to one

A silent payment is not built from the address alone. The **sender** has to
combine the private keys of the coins it is spending with the receiver's
address to work out the one-time destination (BIP-352). So the program that
builds the transaction must hold the keys of every input.

EBP does not build the transaction. It asks the wallet program (barkd) to do
it, and the on-chain withdrawal ("offboard") is made with the Ark server: barkd's
offboard takes a plain Bitcoin address, turns it into an output script, and the
server builds the transaction from its own coins. As far as we could check in
the published Bark source, nothing there handles silent payments, and the
sending wallet does not hold the keys of those inputs. So neither EBP nor barkd
can compute the destination. (This is our reading of the public code, not a
statement from the Bark authors.)

## What would change this

- Bark adding silent payment sending to its on-chain withdrawal. Then EBP only
  needs to accept the address and pass it on; the checking code is already in
  place (`src/payout.rs`).
- Or a payout path where EBP itself holds the keys of the coins it sends, which
  is a different custody design and not planned.

## What you can do today

Use a regular Bitcoin address, or an Ark address (cheap, and the till stays
small). To limit address reuse with a regular address, keep the payout limit
sensible so withdrawals are infrequent, and use a wallet that gives you a fresh
receiving address you can paste when you change it (changing it takes a day and
needs your admin password).

A possible future alternative with a similar privacy benefit, not built: pay out
to a new address derived from a public key you export from your wallet, one
address per withdrawal. That reveals your future addresses (not your funds) to
the server, so it is a trade-off we would want to discuss before building.

## Address checks that do exist

Every payout address, whether typed in the wizard, in Settings, or set in the
server configuration, is checked when it is saved, when the server starts, and
again right before money is sent:

- The checksum (a typo is caught).
- For Bitcoin addresses: the form of the address (witness version and length),
  following BIP-173 and BIP-350.
- For Ark addresses: the `ark` / `tark` form with its checksum.
- That it belongs to this server's network: a test (signet) server refuses a
  real-Bitcoin address, and a live server refuses a test address.
