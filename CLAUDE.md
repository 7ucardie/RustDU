# RustDU

Where the disk space goes: Rust (edition 2024), egui/eframe, same stack and packaging as
[hangar](https://github.com/evandervecht/hangar). Windows and macOS; Linux builds.

## Checks

Run all three before saying a change works, and quote their result:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI runs the same on Windows, macOS and Linux. Code behind `#[cfg(windows)]` or `#[cfg(unix)]` only compiles
there: keep logic in plain functions so it is tested everywhere (see `unix_mode_string` and
`windows_attr_string` in src/scan.rs). From macOS, `cargo check --target x86_64-pc-windows-msvc` catches
most Windows-only mistakes.

## Rules

- The scanner never follows symlinks and never writes; the only write is Move to Trash, behind a dialog.
- The last scan is cached in the user's cache folder (src/cache.rs), never inside the scanned tree. Bump
  `MAGIC` when the layout changes.
- Git is read-only: `--no-optional-locks`, `core.fsmonitor=false`, no prompts.
- Comment only the why. Update README.md when a column, action or script changes.
