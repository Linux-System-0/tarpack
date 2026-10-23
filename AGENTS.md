# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project

`tarpack` is a small, pure-Rust CLI that packs a directory into a tar archive
while honoring `.packignore` files (full `.gitignore` syntax via the `ignore`
crate). The entire binary lives in `src/main.rs`; integration tests are in
`tests/cli.rs`.

- Rust edition 2021, MSRV 1.88 (`rust-version` in `Cargo.toml`).
- Runtime dependencies: `clap`, `ignore`, `tar`, `colored`. Keep the dependency
  set small; do not add crates for things the standard library can do.
- License is GPL-3.0-only. New files must be compatible with it.

## Commands

Run the same checks CI runs before declaring work done:

```bash
cargo fmt --all
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
```

`cargo build --release` produces `target/release/tarpack`. For manual checks,
prefer running the built binary against a throwaway temp directory and inspecting
the archive with `tar -tf`.

Do not hand-edit `Cargo.lock`; update it through Cargo, and keep `--locked`
working in CI. Do not bump `version` in `Cargo.toml` unless the task asks for
it. Releases are cut from `v*` tags on `main`; inside a release, asset names use
the 7-character commit SHA (see "Releases" below).

## Code layout (`src/main.rs`)

The file is organized top-down and sections are easy to locate:

1. `Cli` enum + `PackArgs` / `Cat*Args` + shared `CommonArgs` (clap derive).
2. `main` + one `run_*` function per subcommand.
3. Job/input validation (`resolve_jobs`, `canonical_input`).
4. Walking and classification: `IgnoreMode`, `build_walker`, `EntryInfo`,
   `collect_entries`, `entry_info`, `is_symlink`, `symlink_target`.
5. `print_entry` (the `[Add]`/`[Ignore]` output format).
6. Output safety: `validate_output`, `absolute_output_path`,
   `ensure_output_outside_input`, `resolve_output_path`, `confirm_overwrite`.

### Invariants — preserve these

- **One walker for everything.** `pack` and all `cat-*` commands go through
  `build_walker` + `collect_entries`, so listing output always matches archive
  contents. Never duplicate traversal or ignore logic.
- **Only `.packignore` is read.** The walker explicitly disables git ignore,
  global excludes, parent directories, and hidden-file filtering. Hidden files
  are packed by default. Don't "helpfully" enable other ignore sources.
- **Deterministic order.** Entries are sorted by relative path after collection,
  regardless of `-j`. A directory must sort before its children. Any new output
  (archive or listing) must stay deterministic across thread counts.
- **`-r` means dereference, not recurse.** Traversal is always recursive; the
  flag only controls whether symlinks are followed. `pack` passes it to
  `tar::Builder::follow_symlinks`; `cat-*` uses it to report the stored kind.
  Broken symlinks with `-r` are an error.
- **Output containment is checked twice** (before traversal and immediately
  before `File::create`) because traversal can be slow and paths can change.
  Resolution must handle symlinked parents, dangling final components, and
  targets inside the input tree. Keep both checks and the symlink-aware
  `resolve_output_path` logic intact.
- **Never create/truncate the archive before traversal succeeds.** Entries are
  collected first so a traversal error cannot destroy an existing archive.
- **`--jobs` is bounded.** `0` is rejected and values above `MAX_JOBS` (1024)
  are rejected with the actual value quoted in the error.

## Conventions

- Errors use `Result<_, Box<dyn std::error::Error>>` and lowercase,
  action-oriented messages (`cannot create output ...`, `output ... already
  exists`). `main` prefixes them with `tarpack: ` and exits 1.
- Prefer clear `match`/`if let` over clever iterator chains when it aids
  readability. Comments should explain *why* (platform quirks, safety checks,
  determinism), not restate the code.
- Keep `--help` text and the README in sync. `README.md` is user-facing
  documentation: update it whenever CLI behavior, flags, output format, or
  guarantees change (the PR template requires this).
- Platform-specific behavior (Windows junctions, symlinks, path separators) is
  guarded with `#[cfg(windows)]` / `#[cfg(unix)]`. Keep both branches compiling;
  CI tests Linux, macOS, and Windows.
- GitHub Actions in `.github/workflows/*.yml` must be pinned to a full
  40-character commit SHA, never a mutable tag or branch (for example
  `actions/checkout@<sha> # v7.0.1`). Keep the trailing version comment so
  Dependabot can update the pin.

## Tests

- Add integration tests in `tests/cli.rs`; they invoke the real binary via
  `env!("CARGO_BIN_EXE_tarpack")`.
- Use the existing `TempDir` helper, which cleans up on drop. Pass `NO_COLOR=1`
  and `CLICOLOR=0` (the `tarpack` helper already does this) so assertions don't
  depend on ANSI codes.
- Parse output with `listed_paths` / `archive_paths` rather than raw string
  matching where possible.
- Unix-only filesystem behavior (symlinks) must be behind `#[cfg(unix)]`.
- Tests should be hermetic and parallel-safe: unique temp dirs, no fixed paths,
  no reliance on the repository working tree.
- When fixing a bug, add a regression test that fails without the fix. For
  safety guarantees (output containment, overwrite behavior), assert that no
  input file was modified as well as the exit status.

## Git / branches

The repository uses two long-lived branches:

| Branch    | Role                                                                 |
| --------- | -------------------------------------------------------------------- |
| `main`    | Stable and release-ready. Receives release merges from `develop` (and hotfixes). |
| `develop` | Integration branch for ongoing work; the default target for feature PRs. |

- Never commit or push directly to `main` or `develop`; open a pull request.
- Branch off `develop` with a short-lived `type/topic` name, e.g.
  `feature/symlink-ignore`, `fix/output-containment`, `docs/release-process`,
  `ci/msrv-matrix`. Keep one logical change per branch/PR.
- Rebase topic branches onto `develop` (preferred over merge commits) before
  merging. Don't rewrite `main`/`develop`, and don't force-push a branch others
  are reviewing without warning.
- Feature/fix/docs/CI PRs target `develop`; release and hotfix PRs target
  `main`. After a release or hotfix, merge `main` back into `develop` so the
  branches don't diverge.
- `main` must always build and pass CI; never leave it in a broken state.
- Do not commit build output or scratch files (`target/`, `*.tar`, temp test
  dirs); `.gitignore` already covers the common cases.
- All CI jobs (fmt, clippy, docs, tests on Linux/macOS/Windows + MSRV) must pass
  before a PR is merged.

## Releases

- Releases are triggered by pushing a `v*` tag (for example `v0.1.0`), never by
  pushing to `main` or `develop`. The tag must match the `version` in
  `Cargo.toml`; the release workflow rejects mismatches.
- The workflow publishes the crate to crates.io before creating the GitHub
  release. It needs a `CARGO_REGISTRY_TOKEN` repository secret containing a
  crates.io API token that may publish `tarpack`. Publishing is idempotent: if
  the version is already on crates.io (for example when retrying a failed
  release), the upload is skipped because crates.io versions are immutable.
- Cut a release from `main`: open a PR merging `develop` into `main`, let CI
  pass, merge it, then create an annotated `v*` tag on the merge commit and push
  the tag. Do not tag `develop` or a feature branch.
- Hotfixes branch from `main` as `hotfix/<topic>`, PR back into `main`, are
  tagged like any other release, and are then merged back into `develop`.
- The tag is the release version. Inside the release, assets are named after the
  7-character short commit SHA (for example
  `tarpack-464e72a-x86_64-unknown-linux-gnu`), so downloads map to an exact
  commit.
- `Cargo.toml` is not bumped automatically; only change `version` as part of a
  task that explicitly asks for it.
- Never release from a feature branch or by manually creating a release that
  bypasses the workflow. Verify `.github/workflows/release.yml` when changing
  tag or asset naming.

## Commits and pull requests

Commit messages follow `type: summary` (e.g. `feat: ...`, `fix: ...`,
`docs: ...`). Use the imperative mood and explain the *why* in the body when it
is not obvious. Fill in `.github/PULL_REQUEST_TEMPLATE.md`, mark the checklist
honestly, and call out behavior changes, migration notes, and follow-up work for
reviewers.
