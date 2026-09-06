# tarpack

A small, pure-Rust command-line tool that packs a directory into a single
`tar` archive while honoring gitignore-compatible rules.

## Build

```bash
cargo build --release
# binary: target/release/tarpack
```

## Usage

### Pack a directory

```bash
tarpack pack -i <DIR> -o <FILE> [options]
```

| Option                        | Short | Description                                                                    |
| ----------------------------- | ----- | ------------------------------------------------------------------------------ |
| `--input <DIR>`               | `-i`  | The folder to pack.                                                            |
| `--output <FILE>`             | `-o`  | The output `tar` archive to write.                                             |
| `--recursive`                 | `-r`  | Recursively pack with symlink resolution.                                      |
| `--jobs <N>`                  | `-j`  | Worker threads for directory traversal. Defaults to the number of available CPU cores. |
| `--add-ignore <FILE>`         | `-a`  | Apply an extra ignore file in addition to the default rules. Repeatable.       |
| `--delete-default-ignore`     | `-d`  | Do not use the default `.packignore` ignore file.                              |
| `--yes`                       | `-y`  | Overwrite an existing output archive without prompting.                        |
| `--no`                        | `-n`  | Abort if the output archive already exists without prompting.                  |

### Example

```bash
# Pack with symlink resolution
tarpack pack -r ./project -o ./project.tar
tarpack pack -i ./project -o ./project.tar -d            # ignore nothing by default
tarpack pack -i ./project -o ./project.tar -a extra.gi   # also apply extra.gi rules
tarpack pack -i ./project -o ./project.tar -y            # overwrite without prompting
tarpack pack -i ./project -o ./project.tar -n            # abort if the archive exists
tarpack pack -i ./project -o ./project.tar -j 8          # use 8 worker threads

# List files that will be included
`[Add]`-marked entries; symlinks shown with their resolved target when `-r`.
tarpack cat-add-file --input ./project

tarpack cat-add-file --input ./project -d                   # no default ignore rules

# List files that will be ignored
`[Ignore]`-marked entries.
tarpack cat-ignore-file --input ./project

tarpack cat-ignore-file --input ./project -a extra.gi

# List ALL files with `[Add]` / `[Ignore]` tag, `dir`/`file`/`link` kind and
# detailed path (recursive). `-r` follows symlinks.
tarpack cat-file --input ./project

# Example output:
#   [Add]    [dir]  src
#   [Add]    [file] src/main.rs
#   [Ignore] [file] debug.log
#   [Add]    [link] link.rs -> src/main.rs
tarpack cat-file --input ./project -d                       # no default ignore rules
tarpack cat-file -r --input ./project                       # resolve symlinks
```

## Multithreading

`tarpack pack` traverses the input directory in parallel using `--jobs/-j`, which
defaults to the number of available CPU cores (so `-j` is optional). `-j10`
(attached) and `-j 10` (separate) are both accepted; `0` or negative values are
rejected.

Directory traversal runs on the requested number of worker threads, while a
single dedicated thread writes the archive entries in order (the `tar` writer
is not thread-safe).

## Subcommands

`tarpack` now supports multiple subcommands:

- `tarpack pack` - Pack a directory into a tar archive
- `tarpack cat-add-file` - List files that will be included (`[Add]`)
- `tarpack cat-ignore-file` - List files that will be ignored/excluded (`[Ignore]`)
- `tarpack cat-file` - List every file with `[Add]`/`[Ignore]` tag, kind and path

All `cat-*` commands accept the same ignore-related flags (`-a`, `-d`) and
`-r/--recursive` to follow symlinks.

## Ignore rules

By default, if a `.packignore` file exists in the packing folder, its rules are
applied exactly like a `.gitignore` file. The `ignore` crate (from ripgrep) is
used under the hood, so the **full .gitignore syntax** is supported:

- `# comments` and blank lines
- `*`, `?`, `[...]`, and `**` globs
- negation with `!`
- directory patterns (`dir/`)
- anchored patterns (`/foo`)
- nested `.packignore` files in subdirectories

Hidden files (including `.packignore` itself) are included in the archive by
default.

### Rule sources

By default **only** `<DIR>/.packignore` is consulted. The tool does **not**
read `.gitignore`, `.ignore`, global git excludes, or any parent-directory
ignore files, so the rules stay confined to the packing folder.

- `--add-ignore/-a` adds one or more extra ignore files whose rules are applied
  in addition to the default rules.
- `--delete-default-ignore/-d` disables the default `.packignore` file.

## Behavior guarantees

1. If the output archive already exists, `tarpack` asks for confirmation before
   overwriting it. `--yes`/`-y` overwrites silently, `--no`/`-n` aborts instead
   of prompting. The two flags are mutually exclusive.
2. The output archive must **not** reside inside the folder being packed;
   otherwise `tarpack` errors out (so the archive cannot be included in its own
   contents).
3. Symlinks are preserved as symlinks inside the archive rather than followed,
   matching the default behavior of GNU `tar`. Use `-r/--recursive` to resolve
   symlinks and pack their targets instead.
