# PocketVault

A portable, offline, cross-platform encrypted vault for personal files.

PocketVault encrypts files into a self-contained vault you can carry on a USB drive, external SSD, or just keep next to the executable. There's no install step, no cloud dependency, no database — everything the app needs lives in one `vault/` folder plus a `vault.meta` file, both created next to the `.exe` the first time you run it.

## Tech stack

- **Rust** (stable) — the entire application
- **[Iced](https://iced.rs/)** — the desktop GUI framework (`pocketvault-desktop`)
- **AES-256-GCM + Argon2id** — file encryption and password-based key derivation (`pocketvault-core`)

## Project layout

```
pocketvault/
├── pocketvault-core/       # Encryption, vault management, .pv file format — no UI code, fully unit-tested
│   ├── src/
│   ├── tests/              # Integration tests (full vault lifecycle, etc.)
│   └── benches/            # Criterion benchmarks (encryption/decryption speed)
└── pocketvault-desktop/     # The Iced GUI application
    ├── src/
    │   ├── main.rs          # Entry point, window/daemon setup
    │   ├── state.rs         # App state (Screen, Session, Modal, etc.)
    │   ├── message.rs       # The Message enum — every user action/event
    │   ├── update.rs        # Message -> state transitions, calls into pocketvault-core
    │   ├── theme.rs         # Colors and widget styling
    │   └── view/            # One file per screen (auth, vault_browser, modals, preview, rows)
    └── build.rs             # Embeds the Windows .exe icon
```

## Prerequisites

You need a Rust toolchain. If you don't have one:

1. Install [rustup](https://rustup.rs/) (works on Windows, Linux, and macOS).
2. Restart your terminal, then confirm it's installed:
   ```sh
   rustc --version
   cargo --version
   ```

### Windows

No extra system packages needed — this repo has only been built and tested on Windows so far.

### Linux

Iced (and the native file-dialog crate, `rfd`) need some system libraries to compile and run:

```sh
sudo apt-get update
sudo apt-get install -y \
  libgtk-3-dev \
  libxkbcommon-dev \
  libwayland-dev \
  libx11-dev \
  libxrandr-dev \
  libxi-dev \
  libxcursor-dev \
  libgl1-mesa-dev \
  pkg-config
```
(package names above are for Debian/Ubuntu; adjust for your distro's equivalents.)

### macOS

No extra system packages needed, just Xcode Command Line Tools (`xcode-select --install`) if you don't already have them.

> Linux and macOS builds are exercised in CI (see `.github/workflows/release.yml`) but haven't been manually verified on real hardware yet — if you hit a build issue on either platform, that's useful to report.

## Getting started

```sh
git clone <this-repo-url> pocketvault
cd pocketvault
```

### Build

```sh
# Debug build (faster to compile, slower binary, useful while developing)
cargo build -p pocketvault-desktop

# Release build (what you'd actually distribute)
cargo build --release -p pocketvault-desktop
```

The binary is built to:
- Windows: `target/release/pocketvault.exe`
- Linux/macOS: `target/release/pocketvault`

### Run

```sh
cargo run --release -p pocketvault-desktop
```

Or just launch the built executable directly. On first run (no `vault.meta` next to the executable yet), you'll land on the **Create Vault** screen instead of **Unlock**.

> The vault lives in whatever folder the executable is in — if you run via `cargo run`, that's `target/release/`. If you want to test against a clean/throwaway vault, copy the built executable to an empty folder and run it from there instead of using `cargo run` directly, so you don't clutter the build output with vault data.

### Run tests

```sh
# Just the core library (crypto, vault management, .pv format)
cargo test -p pocketvault-core

# Everything
cargo test
```

`pocketvault-core` has unit tests per module plus integration tests covering the full create → encrypt → export → lock → unlock lifecycle. `pocketvault-desktop` currently has no automated tests (it's UI wiring) — verify it by running the app.

### Benchmarks

```sh
cargo bench -p pocketvault-core
```

## Releasing

Pushing to a branch named `release` triggers `.github/workflows/release.yml`, which builds Windows/Linux/macOS binaries and publishes a GitHub Release tagged with whatever version is in `pocketvault-desktop/Cargo.toml`. Bump that version before merging to `release` to cut a new release.

See `RELEASE.md` for a complete step-by-step guide and recommended release practice.
