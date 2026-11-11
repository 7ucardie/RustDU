# Build and release

Build and check the application:

```sh
cargo build --release --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

GitHub Actions builds Windows and macOS release artifacts when a `v<version>` tag is pushed. macOS signing
and notarization require the secrets listed in [the release workflow](../../.github/workflows/release.yml).
The workflow creates a draft, uploads the packages and checksums, then publishes it after all jobs pass.
Each platform builds independently. If a build fails, the release stays a draft; add any missing secrets
and use **Re-run failed jobs** in the GitHub Actions run. A full rerun also resumes the existing draft.
Published releases are protected from replacement by reruns. To change a published release, use a new tag.
For local packaging instructions, see [Package and sign](../../README.md#package-and-sign).

Dependabot checks Cargo dependencies and GitHub Actions weekly and opens pull requests for updates.
