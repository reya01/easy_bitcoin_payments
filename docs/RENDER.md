# Running on Render (planned, not tested live yet)

Render (render.com) looks after the server, the operating system and the https
padlock for you, so it may turn out to be the easiest way to run EBP. **We
have not run this on Render yet.** Today the recommended path is
[a DigitalOcean server](VPS_DIGITALOCEAN.md). Which of the two is easier will
be decided by testing both the same way; see [CHOOSING_A_HOST.md](CHOOSING_A_HOST.md).
EBP is plain Docker, so using Render never locks you in: your encrypted backup
restores on any other host.

**Test version:** play-money (signet) only.

## What you need to know first

- **A paid plan is required.** Your wallet and order history live on a
  persistent disk. Render only offers disks on paid services. On a free plan
  **everything, including your wallet files, is erased on every restart.**
  The blueprints in this repository refuse to be set to a free plan (a CI
  check enforces it), but do not change the plan yourself.
- **Updates are manual.** The blueprint sets `autoDeploy: false`. You press
  **Manual Deploy** when you choose to. Never turn auto-deploy on.
- **Trust.** Render, like any host, can see the server's disk, and you are
  trusting the code you deploy. The trade-offs are the same as on a VPS; read
  [CHOOSING_A_HOST.md](CHOOSING_A_HOST.md).

## Two ways to deploy

| | Build from the repository (`render.yaml`) | Pinned release image (`deploy/render-image.yaml.template`) |
| --- | --- | --- |
| Available | Now | After the first release |
| What runs | Whatever is in the branch Render builds | A signed image, fixed by its digest |
| Build time | Long (compiles the Rust code on Render) | None (just pulls the image) |
| Recommended | Only on **your own copy (fork)** of the repository | **Yes**, once available |

For either, deploy from **your own copy (fork)** of the repository, not
directly from the original. Then nothing changes on your server unless you
change your copy and press Manual Deploy.

## Steps (draft)

1. Create a Render account with two-factor sign-in.
2. Fork the repository on GitHub.
3. After the first release: copy `deploy/render-image.yaml.template` to
   `render.yaml` in your fork and replace the placeholder with the image line
   from the release notes (an `@sha256:` digest).
4. In Render: **New → Blueprint**, choose your fork, accept the paid plan with
   the disk.
5. Open the service's **Logs** tab and find `SETUP CODE`. Your address is
   shown at the top of the service page (`https://….onrender.com`).
6. Open `https://….onrender.com/setup` and follow [START_HERE.md](START_HERE.md).

## Safer payout settings on any host

The payout logic is unchanged. For any hosted setup it is safer to choose an
**Ark payout address with a low limit**, so each payment is moved to a wallet
you control within minutes and the server never holds much. On-chain payouts
are the simplest and most widely understood, and you can watch an on-chain
address with any public address watcher; if you use on-chain, keep the limit
sensible. See "Make a stolen or tampered server hurt less" in
[CHOOSING_A_HOST.md](CHOOSING_A_HOST.md) and do the weekly wallet check.

## What we still have to find out

- Whether the cheapest paid plan has enough memory for EBP plus the wallet
  process (CI prints the container's idle memory so we have a first number, but
  the real test is a live run).
- Whether a non-technical person finishes setup unassisted, and how
  confusing the Logs tab is for finding the setup code.
- How Render's disk backups behave. Keep downloading your own encrypted backup
  from **Settings** regardless.
