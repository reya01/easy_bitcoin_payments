# Reproducible builds (stage 1 built; stage 2 not started)

**Goal:** anyone can build a release from its source and get the **same bytes** the project
published, so the signature does not have to be taken on trust ("built by our workflow") but
can be checked against the source ("this is what that source produces").

**Why it matters:** the signed-update path ([UPGRADING.md](../UPGRADING.md)) makes the release
workflow and the GitHub account the single point of trust. A reproducible build lets an
independent person notice if a published image ever differs from the code in the repository.

## Do it in two stages

**Stage 1: the `ebp` binary. Built (draft PR, 18 November 2026).** `scripts/repro-build.sh`
builds the binary from the committed source inside the exact Rust image the Dockerfile uses
(pinned by digest), and prints its sha256. The `reproducible-build` workflow runs it on two
different runners (Ubuntu 22.04 and 24.04) and fails if the hashes differ. Anyone with Docker
can run the script at a tag and compare.

How the script removes the usual causes of difference:
- **Different tools.** The build runs in the digest-pinned `rust:1-bookworm` image, so the Rust
  compiler, the linker and the C compiler (which builds the bundled SQLite and `ring`) are the
  same on every machine. Only the host differs.
- **Paths baked into the binary.** The source is always built in `/src`, and
  `--remap-path-prefix` rewrites the source, cargo and rustup folders to fixed names.
- **Moving dependencies.** `--locked`: `Cargo.lock` is law, and the build fails if it would change.
- **Noise and time.** `CARGO_INCREMENTAL=0`; `SOURCE_DATE_EPOCH` is the commit's time. Only
  committed files are built (`git archive HEAD`), never the working folder.

What it does **not** yet prove:
- The Dockerfile builds the image's binary a little differently (a two-step build for caching,
  and without the path-rewriting flags), so this hash is **not yet** the hash of the file inside
  the published image. Stage 2 aligns the Dockerfile with the script and compares the real
  image.
- `rust-toolchain.toml` says only `stable`, so it does not pin a compiler. The digest-pinned
  image does, which is why the script uses it. (The release's `ebp-*.tar.gz` is still built on
  the runner with the latest stable and the runner's musl tools, and its tarball also stores
  file times, so its hash is not reproducible; that is for stage 2.)

**Stage 2: the image.** Docker adds more moving parts:
- `apt-get install` in the image fetches whatever Debian serves that day. Either remove the
  packages (the healthcheck can stop needing `curl`), or install from a dated
  `snapshot.debian.org` mirror with exact versions.
- File timestamps and layer order: build with BuildKit `rewrite-timestamp=true` and
  `SOURCE_DATE_EPOCH`.
- The wallet program (barkd) is downloaded with a pinned sha256 already; keep that.
- Compare **layers** (or the binary inside), not the manifest, since provenance and SBOM
  attestations are attached next to the image and legitimately differ.

## Then publish a way to check

`scripts/verify-release.sh VERSION`: rebuild at the tag, compare to the published binary hash
and to the image's layer hashes, print match or mismatch. Mention it in each release's notes.

## Related, smaller steps that help

- Vendor the crates (`cargo vendor`) or build with `--locked` in CI so the build never asks
  crates.io for anything it did not already have; CI already runs `cargo audit`.
- Keep the Dependabot proposals for base images and actions; a human reads each one.

Order: stage 1 (done, CI proves it), then stage 2.
