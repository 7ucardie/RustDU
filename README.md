<p align="center"><img src="assets/disk-explorer.png" width="160" alt="Disk Explorer logo"></p>

<h1 align="center">RustDU</h1>

<p align="center">
  <a href="https://github.com/7ucardie/RustDU/actions/workflows/ci.yml">
    <img alt="CI" src="https://img.shields.io/github/actions/workflow/status/7ucardie/RustDU/ci.yml?branch=main&label=CI&style=for-the-badge">
  </a>
  <a href="https://github.com/7ucardie/RustDU/releases">
    <img alt="Version" src="https://img.shields.io/github/v/release/7ucardie/RustDU?color=673AB7&label=version&style=for-the-badge">
  </a>
  <a href="LICENSE">
    <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-009688.svg?style=for-the-badge">
  </a>
</p>
<p align="center">
  <a href="https://github.com/7ucardie/RustDU/issues">
    <img alt="GitHub Issues" src="https://img.shields.io/github/issues/7ucardie/RustDU?color=F44336&style=for-the-badge">
  </a>
  <a href="https://github.com/7ucardie/RustDU/stargazers">
    <img alt="GitHub Stars" src="https://img.shields.io/github/stars/7ucardie/RustDU?color=FFEB3B&style=for-the-badge">
  </a>
  <a href="https://github.com/7ucardie/RustDU/network">
    <img alt="GitHub Forks" src="https://img.shields.io/github/forks/7ucardie/RustDU?color=2196F3&style=for-the-badge">
  </a>
</p>
<p align="center">
  <img alt="Rust edition 2024" src="https://img.shields.io/badge/Rust-edition%202024-DEA584.svg?logo=rust&logoColor=white&style=for-the-badge">
  <img alt="egui 0.37" src="https://img.shields.io/badge/egui-0.37-FF5722.svg?style=for-the-badge">
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-x64-0078D6.svg?style=for-the-badge">
  <img alt="macOS arm64 and x64" src="https://img.shields.io/badge/macOS-arm64%20%7C%20x64-000000.svg?logo=apple&logoColor=white&style=for-the-badge">
</p>

See where the disk space goes. Scan a folder and get an explorer view of its subfolders with sizes,
permissions, owners, git state and last activity. Rust and egui, for macOS and Windows (Linux builds).

![RustDU](assets/screenshot.png)

## What it shows

**Volumes**: every disk with a capacity gauge (green, orange from 80%, red from 92%), total, used and free,
and a Scan button that walks the whole volume. When a scan is open, its share of the volume is drawn in
purple and the Explorer header says how much of the used space lies outside the scanned folder. On macOS the
APFS volumes of one container share their free space, and snapshots and purgeable items count as used.

**Explorer**: a sortable tree of the scanned folder.

| Column | Meaning |
| --- | --- |
| Name | Icon, name, branch badge for git repos, 🔒 when unreadable, 💤 when nothing inside changed for a year |
| Size | On disk (allocated, what deleting frees) or logical, with a bar relative to the parent folder |
| % of parent | Share of the parent folder |
| Items | Files and folders inside |
| Last activity | Newest modification date anywhere inside |
| Modified | The item's own modification date |
| Permissions | `drwxr-xr-x` on macOS and Linux; `d r h s a` attributes on Windows |
| Owner | `user:group` on macOS and Linux; `DOMAIN\user` on Windows |
| Git | Branch, changed file count (orange dot) or clean (green check), last commit age |

**Largest files**: the 500 biggest files in the scan and the folder they are in.

**Git repos**: every folder containing `.git`, with branch, last commit, dirty state, repo size, `.git` history
size and remote.

**Trash / Recycle Bin**: what is in the bin with size, item count, deletion date and original location.
Restore (Windows and Linux; macOS keeps Put Back in Finder, Reveal opens the item there), Delete permanently
and Empty, each behind a confirmation.

**Inspector** (right): all of the above for the selection, plus octal mode, uid/gid, whether you can write,
compression savings, and the actions Reveal, Copy path, Open terminal, Scan this folder, Move to…, Move to
Trash…. With several items selected it shows their combined size and counts.

**Selection and file operations**: click selects, ⌘/Ctrl-click adds, Shift-click selects a range, ⌘A / Ctrl+A
selects all visible rows. Right-click for Reveal, Copy paths, Open terminal, Scan this folder, Move to… (a
folder picker; across volumes it copies and then deletes) and Move to Trash…, whose dialog also offers
Delete permanently. Operations run in the background and the tree updates when they finish. Delete or
Backspace opens the Trash dialog for the selection.

Double-click a folder to expand it and a file to reveal it. Drop a folder on the window to scan it.
⌘O / Ctrl+O opens a folder, ⌘R / Ctrl+R rescans, ⌘. stops.

When scanning a whole volume, mount points of other volumes and pseudo filesystems (`/System/Volumes`,
`/Volumes`, `/dev` on macOS; `/proc`, `/sys`, `/dev`, `/run` on Linux) are listed but not walked, so the
macOS Data volume is not counted twice through its firmlinks.

Hard links are counted once. Folders that cannot be read are counted and flagged; on macOS grant Full Disk
Access in System Settings › Privacy & Security to include protected locations.

Git details come from the `git` on your PATH (read-only, `--no-optional-locks`). Without git the branch is
read from `.git/HEAD` and the dirty state is unknown.

## Build and run

```sh
cargo run --release -- ~/some/folder     # scan a folder at once
cargo run --release                      # open and choose a folder
```

Checks, as CI runs them on Windows, macOS and Linux:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Successful CI runs also store downloadable Windows x64 binaries and installer, and an ad-hoc signed
macOS Apple Silicon app, under the workflow run's Artifacts. These builds are not notarized.

## Package and sign

**macOS**: `packaging/macos-app.sh` wraps the binary in `RustDU.app` with the icon and signs it ad hoc,
or with `SIGN_IDENTITY` set, with your Developer ID and the hardened runtime:

```sh
cargo build --release
sh packaging/macos-app.sh target/release/rustdu 0.1.0 out
SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" sh packaging/macos-app.sh target/release/rustdu 0.1.0 out
```

**Windows**: `cargo build --release` gives `target\release\rustdu.exe` with the icon embedded.
`packaging\windows-installer.ps1` builds a per-user Inno Setup installer. Sign either with `signtool`.

**Release**: push a `v<version>` tag. `.github/workflows/release.yml` builds Windows and macOS (arm64 and
amd64), signs and notarizes the macOS bundles with the `MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`,
`APPLE_TEAM_ID`, `NOTARY_KEY_P8`, `NOTARY_KEY_ID` and `NOTARY_ISSUER_ID` secrets, and publishes a GitHub
release with checksums and provenance attestations. A tag with a hyphen makes a pre-release.

## Layout

```
src/main.rs        entry point, optional folder argument
src/scan.rs        tree walk (parallel), sizes, permissions per OS, git probe
src/gui.rs         window: tree, largest files, git repos, inspector, status bar
assets/            icon (png for macOS and the window, ico for Windows)
packaging/         macOS .app script and entitlements, Windows Inno Setup installer
legacy/swift/      the first SwiftUI version, kept for reference
```
