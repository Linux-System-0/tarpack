use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use ignore::WalkBuilder;
use tar::Builder;

/// A tar packaging tool that honors `.packignore` (gitignore-compatible) rules.
///
/// Packs the contents of a directory into a single `tar` archive. By default
/// the `.packignore` file located in the packing folder is used to exclude
/// files, exactly like `.gitignore`. Hidden files are included by default.
#[derive(Parser, Debug)]
#[command(name = "tarpack", version, about, long_about = None)]
struct Args {
    /// The folder to pack.
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input: PathBuf,

    /// The output tar archive to write.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: PathBuf,

    /// Add an extra ignore file whose rules are applied in addition to the
    /// default ignore rules. May be repeated.
    #[arg(short = 'a', long = "add-ignore", value_name = "FILE")]
    add_ignore: Vec<PathBuf>,

    /// Do not use the default `.packignore` ignore file.
    #[arg(short = 'd', long = "delete-default-ignore")]
    delete_default_ignore: bool,

    /// Overwrite an existing output archive without prompting.
    #[arg(short = 'y', long = "yes", conflicts_with = "no")]
    yes: bool,

    /// Abort if the output archive already exists without prompting.
    #[arg(short = 'n', long = "no", conflicts_with = "yes")]
    no: bool,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("tarpack: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let input = fs::canonicalize(&args.input).map_err(|e| {
        format!("cannot access input directory `{}`: {e}", args.input.display())
    })?;
    if !input.is_dir() {
        return Err(format!("input `{}` is not a directory", input.display()).into());
    }

    validate_output(&input, &args.output, args.yes, args.no)?;

    // Build the walker. We only want rules that live in `.packignore` (and any
    // user supplied ignore files). Everything else (`.gitignore`, `.ignore`,
    // parent directories, global git excludes) is disabled so the rules stay
    // predictable and confined to the packing folder.
    let mut walker = WalkBuilder::new(&input);
    walker
        .hidden(false)      // include hidden files (also `.packignore`)
        .parents(false)     // do not read ignore files from parent dirs
        .ignore(false)      // do not read `.ignore`
        .git_ignore(false)  // do not read `.gitignore`
        .git_global(false)  // do not read global gitignore
        .git_exclude(false) // do not read `.git/info/exclude`
        .require_git(false) // ignore rules apply without a git repo
        .sort_by_file_path(|a, b| a.cmp(b));

    if !args.delete_default_ignore {
        walker.add_custom_ignore_filename(".packignore");
    }
    for add in &args.add_ignore {
        if let Some(err) = walker.add_ignore(add) {
            return Err(format!(
                "failed to add ignore file `{}`: {err}",
                add.display()
            )
            .into());
        }
    }

    let out_file = File::create(&args.output)
        .map_err(|e| format!("cannot create output `{}`: {e}", args.output.display()))?;
    let mut tar = Builder::new(out_file);
    // Preserve symlinks as symlinks inside the archive instead of following
    // them (matching the default behavior of GNU tar).
    tar.follow_symlinks(false);

    let mut added = 0usize;
    for entry in walker.build() {
        let entry = entry.map_err(|e| format!("error while walking input: {e}"))?;
        let path = entry.path();
        // Skip the root directory itself; only archive its contents.
        let Ok(rel) = path.strip_prefix(&input) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            tar.append_dir(rel, path)?;
        } else {
            tar.append_path_with_name(path, rel)?;
        }
        added += 1;
    }

    tar.finish()?;

    println!(
        "Packed {} item{} from `{}` into `{}`",
        added,
        if added == 1 { "" } else { "s" },
        input.display(),
        args.output.display()
    );

    Ok(())
}

/// Validates that the output archive can be safely written.
///
/// Errors if the output resides inside the folder being packed and prompts the
/// user for confirmation if the output already exists. The `--yes`/`--no`
/// flags override the prompt: `--yes` overwrites silently, `--no` aborts.
fn validate_output(
    input: &Path,
    output: &Path,
    yes: bool,
    no: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // The archive must not live inside the packed folder, otherwise the
    // archive would (or could) be included in its own contents.
    let out_abs = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    let out_canon = match fs::canonicalize(&out_abs) {
        Ok(p) => p,
        Err(_) => {
            // Output does not exist yet; canonicalize the nearest existing
            // ancestor (its parent directories) for the containment check.
            canonicalize_nearest_ancestor(&out_abs)
        }
    };
    if out_canon.starts_with(input) {
        return Err(format!(
            "output `{}` must not be inside the packed folder `{}`",
            output.display(),
            input.display()
        )
        .into());
    }

    if out_abs.exists() {
        // Decide whether to overwrite: `--yes` forces it, `--no` refuses it,
        // otherwise fall back to an interactive prompt.
        let overwrite = if yes {
            true
        } else if no {
            false
        } else {
            confirm_overwrite(&out_abs)?
        };
        if !overwrite {
            return Err(format!(
                "output `{}` already exists; not overwriting (aborting)",
                output.display()
            )
            .into());
        }
    } else if let Some(parent) = output.parent() {
        if !parent.exists() {
            return Err(format!(
                "output directory `{}` does not exist",
                parent.display()
            )
            .into());
        }
    }

    Ok(())
}

/// Resolves `path` to the closest existing ancestor, canonicalized, with the
/// remaining path components re-appended. Used when the output does not exist
/// yet so permissions/symlinks along the way are honored for the containment
/// check.
fn canonicalize_nearest_ancestor(path: &Path) -> PathBuf {
    let mut ancestor = path.to_path_buf();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while !ancestor.exists() {
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                suffix.push(name.to_os_string());
                ancestor = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut resolved = fs::canonicalize(&ancestor).unwrap_or(ancestor);
    for name in suffix.iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// Prompts the user on stdin for overwrite confirmation. Returns `true` only
/// when the user explicitly answers `y`/`yes`.
fn confirm_overwrite(output: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    print!(
        "output `{}` already exists. Overwrite? [y/N] ",
        output.display()
    );
    io::stdout().flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}
