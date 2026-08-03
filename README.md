# PocketVault

A portable, offline, cross-platform encrypted vault for personal files.

PocketVault encrypts files into a self-contained vault you can carry on a USB drive, external SSD, or just keep next to the executable. There's no install step, no cloud dependency, no database — everything the app needs lives in one `vault/` folder plus a `vault.meta` file, both created next to the `.exe` the first time you run it.

## Tech stack

- **Rust** (stable) — the entire application
- **[Iced](https://iced.rs/)** — the desktop GUI framework (`pocketvault-desktop`)
- **[ratatui](https://ratatui.rs/) + [dialoguer](https://docs.rs/dialoguer)** — the terminal UI (`pocketvault-cli`)
- **AES-256-GCM + Argon2id** — file encryption and password-based key derivation (`pocketvault-core`)

## Project layout

```
pocketvault/
├── pocketvault-core/       # Encryption, vault management, .pv file format — no UI code, fully unit-tested
│   ├── src/
│   ├── tests/              # Integration tests (full vault lifecycle, etc.)
│   └── benches/            # Criterion benchmarks (encryption/decryption speed)
├── pocketvault-desktop/     # The Iced GUI application
│   ├── src/
│   │   ├── main.rs          # Entry point, window/daemon setup
│   │   ├── state.rs         # App state (Screen, Session, Modal, etc.)
│   │   ├── message.rs       # The Message enum — every user action/event
│   │   ├── update.rs        # Message -> state transitions, calls into pocketvault-core
│   │   ├── theme.rs         # Colors and widget styling
│   │   └── view/            # One file per screen (auth, vault_browser, modals, preview, rows)
│   └── build.rs             # Embeds the Windows .exe icon
└── pocketvault-cli/         # The terminal application — same vault, two front ends
    └── src/
        ├── main.rs           # Entry point, --vault-dir flag
        ├── session.rs        # Login flow (create/unlock), toggles between the two front ends below
        ├── menu.rs           # Arrow-key/checkbox menu UI (dialoguer) — the default
        ├── tui.rs            # Full-screen "nano-style" command UI (ratatui) — `Use command line`
        ├── actions.rs         # Encrypt/export/delete/preview/etc. — shared by both front ends
        ├── jobs.rs            # Background encrypt/export jobs, progress, Ctrl-C cancellation
        ├── listing.rs         # Folder/file listing, name/index resolution, formatting
        ├── input.rs           # The TUI's single-line editable input box, incl. Tab-completion edits
        ├── preview.rs         # Decrypt-to-temp-file + open with the OS's default app, for "Preview"
        ├── theme.rs           # `[x]`/`[ ]` checkbox styling for the menu UI
        ├── log.rs             # ✔/⚠/✖ result reporting, shared by both front ends
        └── prompt.rs          # Masked password / y-N prompts used only by the login flow
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

Only `pocketvault-desktop` needs system libraries — `pocketvault-cli` has no GUI dependencies and builds with nothing beyond a Rust toolchain. Iced (and the native file-dialog crate, `rfd`) need:

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

### Build & run the CLI

```sh
cargo build --release -p pocketvault-cli
cargo run --release -p pocketvault-cli
```

The binary is `target/release/pocketvault-cli` (`pocketvault-cli.exe` on Windows) — named separately from the desktop app's `pocketvault` so both can live in the same folder. It needs a real terminal (no pipes/CI — see [Terminal UI](#terminal-ui-pocketvault-cli) below) and, like the desktop app, creates its vault in the current directory by default; pass `-C <dir>` / `--vault-dir <dir>` to use another one.

### Run tests

```sh
# Just the core library (crypto, vault management, .pv format)
cargo test -p pocketvault-core

# Everything
cargo test
```

`pocketvault-core` has unit tests per module plus integration tests covering the full create → encrypt → export → lock → unlock lifecycle. `pocketvault-desktop` and `pocketvault-cli` currently have no automated tests (both are UI wiring over the same tested core) — verify them by running the app.

### Benchmarks

```sh
cargo bench -p pocketvault-core
```

## Terminal UI (pocketvault-cli)

`pocketvault-cli` is an interactive session, not a one-shot command with flags — run `pocketvault-cli` and it stays open (create/unlock once, then browse, encrypt, export, etc. until you exit), similar in spirit to how `claude` or `nano` work. It needs a real terminal; piped/non-interactive input isn't supported.

There are two front ends over the same vault, and you can switch between them at any time without leaving the session:

- **Menu UI** (default) — arrow keys + Enter over `[x]`/`[ ]` style lists: Encrypt File, Encrypt Folder, Browse Vault (navigate folders; per-item Preview/Export/Rename/Delete — Preview opens the file in your OS's default app for it; and Encrypt File/Encrypt Folder are offered again inside whatever subfolder you're browsing, not just from the main menu), Settings (Change Password, Lock Vault), Use command line, Exit.
- **Command-line UI** — reached via the menu's **Use command line**, or by typing `menu` from inside it to switch back. Instead of a scrolling transcript, the terminal becomes a full-screen frame (like `nano`): a title bar showing the current folder, the folder's contents (or a running job's live progress, or the help screen), a status line, and a command input at the bottom.

  The listing itself is arrow-key navigable, same idea as the menu UI: **Up/Down highlights a row** (only when the input line is empty — once you start typing, Up/Down instead recalls command history), and the highlighted row's actions appear inline on that row:

  ```
  📁 Documents  (3 items)
  📄 astro.svg  (2.8 KB, 5 weeks ago)  [Preview] [Export] [Delete]
  [Menu]  Leave the command line
  ```

  Press **Enter** on a highlighted row to open a small `[x]`/`[ ]` action list for it — `[Open] [Export] [Rename] [Delete]` for a folder, `[Preview] [Export] [Delete]` for a file — navigated the same way (Up/Down, Enter, Esc to cancel). Two rows are always present regardless of what's in the folder: **`[..]`** at the top (except at the vault root) to go up a level, and **`[Menu]`** at the bottom to leave the command line and return to the menu UI — no keybinding to memorize either one.

  **Preview** decrypts the file to a temporary location and opens it with your OS's default app for that file type (like double-clicking it) — a text editor for text, an image viewer for images, and so on, rather than trying to render it inside the terminal. That temp copy is plaintext; the previous one is deleted before writing a new one, but it isn't wiped the instant the viewer opens it.

  You can still type a full command instead of navigating — **Tab** completes the command name, a vault item's name, or (for `encrypt`/`export`'s destination) a filesystem path:

  | Command | Effect |
  |---|---|
  | `ls` | Refresh the listing |
  | `cd <name>` / `cd ..` / `cd /` | Change folder |
  | `mkdir <name>` | Create a folder here |
  | `rename <folder> <name>` | Rename a folder |
  | `rm <name> [-f]` | Delete a file or folder (asks to confirm unless `-f`) |
  | `encrypt <path> [<path> ...]` | Encrypt file(s), or one folder recursively, into the current folder |
  | `export <name> <dest>` | Decrypt a file or folder out to disk |
  | `cat <name>` | Same as the `[Preview]` button — open in your OS's default app |
  | `passwd` | Change the master password |
  | `menu` | Switch to the menu UI |
  | `lock` / `exit` | Lock the vault / leave PocketVault |

  A file or folder can be addressed by its exact name, or by the index number shown in the last listing. Encrypt/export jobs at or above 100MB show a live byte-progress bar with ETA and are cancellable with Ctrl-C (anything smaller runs with just a brief spinner); Ctrl-L locks the vault and Ctrl-X exits, from either front end.

## Releasing

Pushing a version tag like `v0.1.6` triggers `.github/workflows/release.yml`, which builds Windows/Linux/macOS binaries for **both** `pocketvault-desktop` and `pocketvault-cli` and publishes them together on one GitHub Release. The two apps are versioned in lockstep — `pocketvault-desktop/Cargo.toml` and `pocketvault-cli/Cargo.toml` must have matching versions, and that version must match the pushed tag, or the release job fails.

See `RELEASE.md` for a complete step-by-step guide and recommended release practice.
