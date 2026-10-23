# tarpack

[![CI](https://github.com/Linux-System-0/tarpack/actions/workflows/ci.yml/badge.svg)](https://github.com/Linux-System-0/tarpack/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/tarpack.svg)](https://crates.io/crates/tarpack)
[![Security audit](https://github.com/Linux-System-0/tarpack/actions/workflows/audit.yml/badge.svg)](https://github.com/Linux-System-0/tarpack/actions/workflows/audit.yml)
[![Release](https://img.shields.io/github/v/release/Linux-System-0/tarpack?display_name=tag)](https://github.com/Linux-System-0/tarpack/releases)
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue.svg)](LICENSE)

A small, pure-Rust command-line tool that packs a directory into a single
`tar` archive while honoring gitignore-compatible rules.

## Build

Requires Rust 1.88 or newer (see `rust-version` in `Cargo.toml`).

Install from crates.io:

```bash
cargo install tarpack
```

Or installl from github:

```bash
cargo install --git https://github.com/Linux-System-0/tarpack.git
```

Or pull from github and install:

```bash
git clone https://github.com/Linux-System-0/tarpack.git tarpack && cd tarpack
cargo install --path .
# If you want to delete temp directories:
# cd ..
# rm -rf tarpack
```

## Usage

### Pack a directory

```bash
tarpack pack -i <DIR> -o <FILE> [options]
```

| Option                    | Short | Description                                                                                  |
| ------------------------- | ----- | -------------------------------------------------------------------------------------------- |
| `--input <DIR>`           | `-i`  | The folder to pack.                                                                          |
| `--output <FILE>`         | `-o`  | The output `tar` archive to write.                                                           |
| `--recursive`             | `-r`  | Follow symlinks and pack their targets instead of the links themselves (alias: `--follow-symlinks`). Traversal is always recursive; this only controls dereferencing. |
| `--jobs <N>`              | `-j`  | Traversal worker threads. Defaults to the number of available CPU cores.                     |
| `--add-ignore <FILE>`     | `-a`  | Apply an extra ignore file in addition to `.packignore` files. Repeatable.                    |
| `--delete-default-ignore` | `-d`  | Do not read `.packignore` files.                                                              |
| `--yes`                   | `-y`  | Overwrite an existing output archive without prompting.                                       |
| `--no`                    | `-n`  | Abort if the output archive already exists without prompting.                                 |

### Examples

```bash
# Pack a project
tarpack pack -i ./project -o ./project.tar

# Follow symlinks and pack their targets
tarpack pack -i ./project -o ./project.tar -r

# Ignore nothing by default
tarpack pack -i ./project -o ./project.tar -d

# Also apply rules from extra.ignore
tarpack pack -i ./project -o ./project.tar -a extra.ignore

# Overwrite without prompting (or abort instead with -n)
tarpack pack -i ./project -o ./project.tar -y
tarpack pack -i ./project -o ./project.tar -n

# Use 8 traversal worker threads
tarpack pack -i ./project -o ./project.tar -j 8
```

### Inspect what will be packed

```bash
# Files that will be included ([Add])
tarpack cat-add-file -i ./project

# Files that will be ignored ([Ignore])
tarpack cat-ignore-file -i ./project

# Every entry with its [Add]/[Ignore] status, kind and symlink target
tarpack cat-file -i ./project

# Example output (without -r):
#   [Add] [dir] src
#   [Add] [file] src/main.rs
#   [Ignore] [file] debug.log
#   [Add] [link] link.rs -> src/main.rs
#   [Add] [link] dirlink -> src
```

The `cat-*` commands accept the same `-i`, `-r`, `-j`, `-a` and `-d` options
and use exactly the same walker/ignore configuration as `pack`, so their output
matches the archive contents.

## Multithreading

All subcommands support `--jobs/-j`; it defaults to the number of available CPU
cores. Both `-j10` (attached) and `-j 10` (separate) are accepted. `0` is
rejected, and values above 1024 are rejected to avoid accidentally spawning an
unbounded number of OS threads.

Traversal runs on the requested number of worker threads. Regardless of the
thread count, entries are sorted by their archive path before being printed or
written, so the entry order is deterministic: a directory entry always precedes
its children, and `-j1` and `-j8` produce the same order. (Tar headers such as
mtimes and ownership still reflect the source files.)

## Subcommands

- `tarpack pack` - Pack a directory into a tar archive
- `tarpack cat-add-file` - List files that will be included (`[Add]`)
- `tarpack cat-ignore-file` - List files that will be ignored (`[Ignore]`)
- `tarpack cat-file` - List every file with `[Add]`/`[Ignore]` status, kind and path

## Symlinks

By default, symlinks are preserved as symlinks inside the archive rather than
followed, matching the default behavior of GNU `tar`. The `cat-*` commands show
them as `[link] path -> target` (on Windows, a link to a directory is shown as
`[link(dir)]`).

With `-r/--recursive`, symlinks are dereferenced: a symlink to a file is
archived as a regular file, and a symlink to a directory is archived as a
directory and traversed. The `cat-*` commands then report the kind that would be
stored (`[file]` or `[dir]`) and still show the link target after `->`.

Broken symlinks are an error when `-r` is used, since there is no target to
archive.

## Ignore rules

By default, `.packignore` files are used to exclude files. The `ignore` crate
(from ripgrep) is used under the hood, so the **full .gitignore syntax** is
supported:

- `# comments` and blank lines
- `*`, `?`, `[...]`, and `**` globs
- negation with `!`
- directory patterns (`dir/`)
- anchored patterns (`/foo`)
- nested `.packignore` files in subdirectories

Hidden files (including `.packignore` itself) are included in the archive by
default.

### Rule sources

Only `.packignore` files are consulted. The tool does **not** read `.gitignore`,
`.ignore`, global git excludes, or ignore files from parent directories above
the packing folder. Nested `.packignore` files inside the packing folder **are**
honored, just like nested `.gitignore` files in git.

- `--add-ignore/-a` adds one or more extra ignore files. Relative `FILE` paths
  are resolved from the current directory, while the rules inside them are
  matched relative to the packing root. As with ripgrep's `--ignore-file`,
  these files have lower precedence than `.packignore` files: they can add
  exclusions, but cannot re-include something excluded by `.packignore`.
- `--delete-default-ignore/-d` disables all `.packignore` files, including
  nested ones.

## Behavior guarantees

1. If the output archive already exists, `tarpack` asks for confirmation before
   overwriting it. `--yes`/`-y` overwrites silently, `--no`/`-n` aborts instead
   of prompting. The two flags are mutually exclusive.
2. The output archive must **not** reside inside the folder being packed;
   otherwise `tarpack` errors out (so the archive cannot be included in its own
   contents). All symlinks in the output path are resolved for this check,
   including a dangling symlink in the final component and symlinked parent
   directories. The check is performed again immediately before the output file
   is created, since directory traversal can take a long time.
   - A symlink outside the input tree that points to a file inside the input
     tree is rejected, and no file inside the input tree is touched.
   - If the output's parent directory is a symlink to the input directory (or to
     a subdirectory of it), the output is rejected as well.
3. The output file is only created after the directory traversal has completed
   successfully, so a traversal error does not truncate an existing archive.
4. Entries are written in sorted order, so the archive entry order is
   deterministic for a given input tree.

## Development

The CI configuration in `.github/workflows/ci.yml` runs the same checks locally:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
```

### Branching

`main` is the stable, release-ready branch. Day-to-day work lands on `develop`
through short-lived `feature/*`, `fix/*`, `docs/*` and `ci/*` branches. Feature
PRs target `develop`; release and `hotfix/*` PRs target `main`. After a release
or hotfix, `main` is merged back into `develop`.

### Releases

Cut a release by merging `develop` into `main` and pushing a `v*` tag, not by
pushing to `main` or `develop`. Create the annotated tag on `main` (for example
`git tag -a v0.1.0 -m "tarpack v0.1.0"`) and push it
(`git push origin v0.1.0`); the tag is the release version. The tag must match
the `version` in `Cargo.toml`, otherwise the workflow rejects the release.

The workflow publishes the crate to crates.io (using the
`CARGO_REGISTRY_TOKEN` repository secret) before creating the GitHub release,
so the tag is only released once the crate is available. Re-running a release
skips the upload if that version is already on crates.io. Inside a release,
uploads are identified by the 7-character short commit SHA, so asset names look
like `tarpack-464e72a-x86_64-unknown-linux-gnu`. The workflow uploads prebuilt
Linux, macOS and Windows binaries with SHA-256 checksums and generates release
notes grouped by `.github/release.yml`. The version in `Cargo.toml` is not bumped
automatically.

## Security

Please do not open public issues for security problems. See
[.github/SECURITY.md](.github/SECURITY.md) for the private reporting process and
scope.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
