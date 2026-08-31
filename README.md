# tarpack

A small, pure-Rust command-line tool that packs a directory into a single
`tar` archive while honoring gitignore-compatible rules.

## Build

```bash
cargo build --release
# binary: target/release/tarpack
```

## Usage

```bash
tarpack -i <DIR> -o <FILE> [options]
```

| Option                       | Short | Description                                                        |
| ---------------------------- | ----- | ------------------------------------------------------------------ |
| `--input <DIR>`              | `-i`  | The folder to pack.                                                |
| `--output <FILE>`            | `-o`  | The output `tar` archive to write.                                 |
| `--add-ignore <FILE>`        | `-a`  | Apply an extra ignore file in addition to the default rules. Repeatable. |
| `--delete-default-ignore`    | `-d`  | Do not use the default `.packignore` ignore file.                  |
| `--yes`                      | `-y`  | Overwrite an existing output archive without prompting.            |
| `--no`                       | `-n`  | Abort if the output archive already exists without prompting.      |

### Example

```bash
tarpack -i ./project -o ./project.tar
tarpack -i ./project -o ./project.tar -d            # ignore nothing by default
tarpack -i ./project -o ./project.tar -a extra.gi   # also apply extra.gi rules
tarpack -i ./project -o ./project.tar -y            # overwrite without prompting
tarpack -i ./project -o ./project.tar -n            # abort if the archive exists
```

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
   matching the default behavior of GNU `tar`.
