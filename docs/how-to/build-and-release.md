# Build and release

Build and check the application:

```sh
cargo build --release --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

To release, merge the release pull request. Write commit subjects as
[Conventional Commits](https://www.conventionalcommits.org/): on every push to `main` the Version workflow
reads the commits since the last tag and picks the bump.

| Commits since the last tag | Next version |
| --- | --- |
| `feat: …` | minor (0.2.3 → 0.3.0) |
| `fix: …`, `perf: …` | patch (0.2.3 → 0.2.4) |
| `feat!: …` or a `BREAKING CHANGE:` footer | major, or minor while the version is 0.x |
| only `docs`, `ci`, `chore`, `build`, `test`, `refactor` | no release |

It opens a pull request `chore: release <version>` from the branch `release/next`, holding only the new
version in `Cargo.toml` and `Cargo.lock` and listing the changes. The pull request is rebuilt each time
`main` changes, so the version and the list stay current. CI runs on it and reports as the status
**CI on the release pull request**; GitHub's own run for a bot's pull request waits for **Approve and run**,
which you can ignore. Merge it when you want the
release: the Version workflow sees a version without a tag, tags the merge `v<version>` and starts the
Release workflow, which names every artifact after the tag.

To release when the commits ask for none (dependency updates, for example), run **Version** from the
Actions tab and choose a bump. `sh packaging/next-version.sh` prints what the next version would be.

The rule behind it: a version in `Cargo.toml` on `main` that has no tag gets released. Leave that line to
the release pull request. Pushing a `v<version>` tag yourself still starts a release, which is how to make
a pre-release (`v1.0.0-rc.1`).

The repository setting **Actions › General › Allow GitHub Actions to create and approve pull requests**
must be on, or the workflow cannot open the pull request.

GitHub Actions builds Windows, macOS and Linux release artifacts for the tag (Linux: a `.deb` and a
tarball for amd64 and arm64). macOS signing
and notarization use the secrets listed in [the release workflow](../../.github/workflows/release.yml).
Without them the release still goes out: the macOS bundles are signed ad hoc, the release notes say so, and
users allow the app once under System Settings › Privacy & Security › Open Anyway.
The workflow creates a draft, uploads the packages and checksums, then publishes it after all jobs pass.
Each platform builds independently. If a build fails, the release stays a draft; fix the cause
and use **Re-run failed jobs** in the GitHub Actions run. A full rerun also resumes the existing draft.
Published releases are protected from replacement by reruns. To change a published release, use a new tag.
For local packaging instructions, see [Package and sign](../../README.md#package-and-sign).

Dependabot checks Cargo dependencies and GitHub Actions weekly and opens pull requests for updates.
