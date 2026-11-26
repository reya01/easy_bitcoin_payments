# Reproducible builds (stage 1 built; stage 2: binary in the image and the download built)

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
- Until stage 2 (below), the Dockerfile built the image's binary without the path-rewriting
  flags, so the published images up to then do not match this hash.
- Until 26 November 2026 `rust-toolchain.toml` said only `stable`. Inside `/src` that file wins
  over the image's own compiler, so every build quietly downloaded the newest stable Rust: two
  builds on the same day matched, but a rebuild after the next Rust release could not. It now
  names an exact version (`1.100.0` at first); move it on purpose, in its own PR. The image still
  pins the C compiler, linker and system libraries. (Until 0.3.3-rc5 the release's `ebp-*.tar.gz` was
  built on the runner with the latest stable and the runner's musl tools, with file times in the
  tarball, so those downloads are not reproducible. See "the download" below.)

**Stage 2, first part: the binary inside the image. Built (24 November 2026, first in
0.3.3-rc1).** The Dockerfile's build stage now sets the same `CARGO_INCREMENTAL=0` and
`--remap-path-prefix` flags as the script, in the same `/src` folder and the same pinned Rust
image. The `reproducible-build` workflow gained a third job that builds the image
(`--target runtime`), copies `/usr/local/bin/ebp` out of it, and fails unless its sha256 equals
the two script builds. The managed (default) stage copies the same file from the same build
stage. And `scripts/verify-release.sh VERSION` lets anyone check a published release: it
rebuilds the tag with the script, takes the program out of the published amd64 image (by the
digest in the release's `IMAGE_DIGEST`), and prints MATCH or DIFFERENT. Only releases built
after this change can match. The arm64 image is built under emulation and is not compared yet.

**Stage 2, second part: the download. Built (26 November 2026, first in the release after
0.3.3-rc5).** `scripts/repro-build-musl.sh` builds `ebp-x86_64-unknown-linux-musl.tar.gz`, the
file people install, and the release workflow now publishes exactly what it builds. On top of
stage 1's settings it:
- installs the musl C tools (they compile the bundled SQLite and `ring`) from a **dated Debian
  snapshot** at an exact version (`SNAPSHOT` and `MUSL_VERSION` in the script; change them only
  together, in a PR);
- adds the musl standard library with `rustup target add`, which fetches it for the exact Rust
  version in `rust-toolchain.toml`;
- writes the tarball with sorted names, owner 0, fixed permissions and the commit's time, and
  gzips it without a name or time.

The `reproducible-build` workflow builds the download on the same two different runners and
fails unless both tarballs are identical. `scripts/verify-release.sh VERSION` rebuilds the
download too and compares it with the release's `SHA256SUMS` (for tags that have the script).

**Stage 2, still to do: the whole image.** Docker adds more moving parts:
- `apt-get install` in the image fetches whatever Debian serves that day. Either remove the
  packages (the healthcheck can stop needing `curl`), or install from a dated
  `snapshot.debian.org` mirror with exact versions.
- File timestamps and layer order: build with BuildKit `rewrite-timestamp=true` and
  `SOURCE_DATE_EPOCH`.
- The wallet program (barkd) is downloaded with a pinned sha256 already; keep that.
- Compare **layers** (or the binary inside), not the manifest, since provenance and SBOM
  attestations are attached next to the image and legitimately differ.

## Then publish a way to check

`scripts/verify-release.sh VERSION` exists (the binary inside the image, above). Still to do:
compare the image's layer hashes once the whole image is reproducible, and mention the script
in each release's notes.

## Related, smaller steps that help

- Vendor the crates (`cargo vendor`) or build with `--locked` in CI so the build never asks
  crates.io for anything it did not already have; CI already runs `cargo audit`.
- Keep the Dependabot proposals for base images and actions; a human reads each one.

Order: stage 1 (done, CI proves it), stage 2 first and second parts (built, CI proves them), then the whole image.
