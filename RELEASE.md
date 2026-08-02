# Release Guide

This document describes the release workflow for PocketVault.

## Release trigger

A GitHub Actions workflow runs when a push is made to the `release` branch.
The workflow builds the desktop app for Windows, Linux, and macOS, packages the binaries, and publishes a GitHub Release.

## Version source

The published release version is taken from `pocketvault-desktop/Cargo.toml`.

Example version field:

```toml
[package]
name = "pocketvault-desktop"
version = "0.1.0"
edition = "2021"
```

The workflow uses that version to create a release tag like `v0.1.0`.

## What happens in CI

The workflow uses three runners:

1. `windows-latest`
2. `ubuntu-latest`
3. `macos-latest`

Each runner does a `cargo build --release -p pocketvault-desktop` and uploads its built binary as an artifact.

Then a Linux runner downloads the three artifacts, combines the three platform binaries into cross-platform ZIP archives, and publishes them to GitHub Release.

## How to cut a new release

1. Update `pocketvault-desktop/Cargo.toml` to the desired version.
   - Follow semantic versioning: `MAJOR.MINOR.PATCH`.
   - Example: `0.1.0` → `0.1.1` for a bug fix.

2. Commit the version bump.

3. Merge the commit into the `release` branch.

4. GitHub Actions will run the release workflow automatically.

5. After the workflow finishes, the new GitHub Release will be created with the new tag.

## Recommended versioning

Use small, frequent releases when possible.

- `0.1.1` for bug fixes
- `0.2.0` for small new features
- `1.0.0` for a stable first major release

## Alternative workflow (tag-based)

A more common release pattern is to release from git tags instead of a dedicated `release` branch.

1. Develop on `main`.
2. When ready to publish, bump the version in `pocketvault-desktop/Cargo.toml`.
3. Create a git tag:
   ```sh
git tag v0.1.1
git push origin v0.1.1
```
4. CI publishes the tagged version.

This is a good future improvement if you want a more standard versioned release flow.

## What to include in releases

Each published release should include:

- `pocketvault-windows-x86_64.zip`
- `pocketvault-linux-x86_64.zip`
- `pocketvault-macos-x86_64.zip`

Each ZIP contains all three platform binaries:
- `pocketvault.exe`
- `pocketvault`
- `pocketvault-macos`

That lets anyone open the archive and use the executable for their platform.

## Notes

- The release workflow is currently triggered by pushes to the `release` branch.
- The version must be bumped in `pocketvault-desktop/Cargo.toml` before merging to `release`.
- If you want, this workflow can be updated later to use git tags instead of a dedicated release branch.

So the usual flow is:

Commit the fix
Bump the version in Cargo.toml
Push to main
Create a new tag, for example v0.1.5
Example:


## COmon
git add pocketvault-desktop/Cargo.toml
git commit -m "Fix release workflow build issue"
git push origin main

git tag v0.1.5
git push origin v0.1.5