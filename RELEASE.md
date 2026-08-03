# Release Guide

This document describes the release workflow for PocketVault.

## Release trigger

A GitHub Actions workflow (`.github/workflows/release.yml`) runs when a version tag like `v0.1.5` is pushed (or via manual dispatch from the Actions tab).
The workflow builds **both** `pocketvault-desktop` and `pocketvault-cli` for Windows, Linux, and macOS, packages the binaries, and publishes a single GitHub Release containing all of it — one tag, one release page, both apps.

## Version source

The published release version is taken from `pocketvault-desktop/Cargo.toml`. `pocketvault-cli/Cargo.toml` is kept at the **same** version — the two are released in lockstep, not independently versioned. CI enforces this: the release job fails if the two `Cargo.toml` versions don't match, or if either doesn't match the pushed tag.

Example version field:

```toml
[package]
name = "pocketvault-desktop"
version = "0.1.5"
edition = "2021"
```

```toml
[package]
name = "pocketvault-cli"
version = "0.1.5"   # <- must match pocketvault-desktop's version exactly
edition = "2021"
```

The workflow uses the desktop version to create a release tag like `v0.1.5`.

## What happens in CI

The workflow uses three runners:

1. `windows-latest`
2. `ubuntu-latest`
3. `macos-latest`

Each runner builds **both** binaries (`cargo build --release -p pocketvault-desktop` and `cargo build --release -p pocketvault-cli`) and uploads each as its own artifact — six artifacts total (3 platforms × 2 apps).

Then a Linux runner downloads all six, packages them into platform archives, and publishes them together to one GitHub Release:

- `pocketvault-windows-x86_64.zip` / `pocketvault-linux-x86_64.tar.gz` / `pocketvault-macos-x86_64.app.zip` — the desktop app (unchanged from before; macOS gets a minimal `.app` bundle since it's a GUI app).
- `pocketvault-cli-windows-x86_64.zip` / `pocketvault-cli-linux-x86_64.tar.gz` / `pocketvault-cli-macos-x86_64.zip` — the CLI. No `.app` bundle — it's a terminal tool, so each archive is just the plain `pocketvault-cli` / `pocketvault-cli.exe` binary.

## How to cut a new release

1. Update **both** `pocketvault-desktop/Cargo.toml` and `pocketvault-cli/Cargo.toml` to the same new version.
   - Follow semantic versioning: `MAJOR.MINOR.PATCH`.
   - Example: `0.1.5` → `0.1.6` for a bug fix.

2. Commit the version bump.

3. Push to `main` (or merge a PR that includes it).

4. Tag the commit and push the tag:
   ```sh
   git tag v0.1.6
   git push origin v0.1.6
   ```

5. GitHub Actions runs the release workflow automatically on the tag push.

6. After the workflow finishes, the new GitHub Release will be created with both apps' archives attached under that one tag.

## Recommended versioning

Use small, frequent releases when possible.

- `0.1.1` for bug fixes
- `0.2.0` for small new features
- `1.0.0` for a stable first major release

## What to include in releases

Each published release should include six archives, one per (app, platform) pair:

- `pocketvault-windows-x86_64.zip` → `pocketvault.exe` (desktop)
- `pocketvault-linux-x86_64.tar.gz` → `pocketvault` (desktop)
- `pocketvault-macos-x86_64.app.zip` → `PocketVault.app` (desktop, minimal macOS app bundle)
- `pocketvault-cli-windows-x86_64.zip` → `pocketvault-cli.exe` (CLI)
- `pocketvault-cli-linux-x86_64.tar.gz` → `pocketvault-cli` (CLI)
- `pocketvault-cli-macos-x86_64.zip` → `pocketvault-cli` (CLI)

Each archive contains just the one binary for that platform — pick the app and platform you want, download that one archive.

## Notes

- Example end-to-end flow for a patch release:
  ```sh
  # bump both versions to 0.1.6 in pocketvault-desktop/Cargo.toml and pocketvault-cli/Cargo.toml first
  git add pocketvault-desktop/Cargo.toml pocketvault-cli/Cargo.toml
  git commit -m "Bump version to 0.1.6"
  git push origin main

  git tag v0.1.6
  git push origin v0.1.6
  ```
- The release job double-checks that `pocketvault-desktop/Cargo.toml`'s version, `pocketvault-cli/Cargo.toml`'s version, and the pushed tag all agree — it fails loudly rather than publishing a mismatched release.