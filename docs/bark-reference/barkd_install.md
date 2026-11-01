> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Install Barkd

> Install Barkd on Linux, macOS, or Windows. Download a pre-built binary from GitLab releases or compile from source.

Barkd has two installation methods:

* Download and install the binary release
* [Compile from source](#compile-from-source)

## Binary release

Pre-built `barkd` binaries are published on [GitLab releases](https://gitlab.com/ark-bitcoin/bark/-/releases). Pick the archive for your platform and drop the binary into your `PATH`.

> [tab] Ubuntu/Debian
>
> **1. Download barkd**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> curl -L https://gitlab.com/ark-bitcoin/bark/-/releases/bark-0.8.0/downloads/barkd-0.8.0-linux-x86_64 --output barkd
> ```
>
> **2. Mark the binary as executable**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> chmod +x barkd
> ```
>
> **3. Add barkd to your PATH**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> sudo cp barkd /usr/local/bin/
> ```

> [tab] Fedora/CentOS
>
> **1. Download barkd**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> curl -L https://gitlab.com/ark-bitcoin/bark/-/releases/bark-0.8.0/downloads/barkd-0.8.0-linux-x86_64 --output barkd
> ```
>
> **2. Mark the binary as executable**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> chmod +x barkd
> ```
>
> **3. Add barkd to your PATH**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> sudo cp barkd /usr/local/bin/
> ```

> [tab] macOS
>
> **1. Download barkd**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> curl -L https://gitlab.com/ark-bitcoin/bark/-/releases/bark-0.8.0/downloads/barkd-0.8.0-apple-aarch64 --output barkd
> ```
>
> **2. Mark the binary as executable**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> chmod +x barkd
> ```
>
> **3. Add barkd to your PATH**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> sudo cp barkd /usr/local/bin/
> ```

> [tab] macOS (Intel)
>
> **1. Download barkd**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> curl -L https://gitlab.com/ark-bitcoin/bark/-/releases/bark-0.8.0/downloads/barkd-0.8.0-apple-x86_64 --output barkd
> ```
>
> **2. Mark the binary as executable**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> chmod +x barkd
> ```
>
> **3. Add barkd to your PATH**
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> sudo cp barkd /usr/local/bin/
> ```

> [tab] Windows
>
> Download barkd:
>
> ```PowerShell theme={"theme":{"light":"github-light","dark":"github-dark"}}
> wget https://gitlab.com/ark-bitcoin/bark/-/releases/bark-0.8.0/downloads/barkd-0.8.0-windows-x86_64.exe -OutFile barkd.exe
> ```
>
> > [!NOTE]
> >
> > When using Windows, replace `barkd` with `barkd.exe` in the rest of the commands.

## Verify installation

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
barkd --version
```

You should see: `barkd 0.8.0 (<commit hash>)`

## Compile from source

**1. Clone the repository**

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
git clone https://gitlab.com/ark-bitcoin/bark
cd bark
```

**2. Install Rust**

You need a Rust compiler. The [rustup website](https://rustup.rs) offers installation instructions for almost any platform.

**3. Install build dependencies**

Download additional dependencies using your package manager.

> [tab] Ubuntu/Debian
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> apt update
> apt install build-essential clang protobuf-compiler
> ```

> [tab] Fedora/CentOS
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> yum install gcc make clang protobuf-compiler
> ```

> [tab] macOS
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> brew install gcc make llvm@14 protobuf
> ```

> [tab] Windows
>
> ```PowerShell theme={"theme":{"light":"github-light","dark":"github-dark"}}
> winget install protobuf ezwinports.make LLVM.LLVM
> ```

> [tab] Nix
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> nix develop
> ```
>
> Requires the `nix-command` and `flakes` experimental features.

**4. Build barkd**

Use `cargo build` to produce the binary in `target/`, or `cargo install` to build and add it to your `PATH` in one step.

> [tab] build
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> cargo build --bin barkd
> ./target/debug/barkd --version
> ```

> [tab] build and install
>
> ```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
> cargo install --locked --path ./bark-cli --bin barkd
> barkd --version
> ```

**5. Verify installation**

Run `barkd --version` to confirm the install completed successfully.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.