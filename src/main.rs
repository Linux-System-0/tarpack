use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use clap::Parser;
use ignore::{WalkBuilder, WalkState};
use tar::Builder;

/// A tar packaging tool that honors `.packignore` (gitignore-compatible) rules.
///
/// Packs the contents of a directory into a single `tar` archive. By default
/// the `.packignore` file located in the packing folder is used to exclude
/// files, exactly like `.gitignore`. Hidden files are included by default.
#[derive(Parser, Debug)]
#[command(name = "tarpack", version, about, long_about = None)]
enum Cli {
    Pack(PackArgs),
    CatAddFile(CatAddFileArgs),
    CatIgnoreFile(CatIgnoreFileArgs),
    CatFile(CatFileArgs),
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "Pack a directory into a tar archive")]
struct PackArgs {
    /// The folder to pack.
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input: PathBuf,

    /// The output tar archive to write.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: PathBuf,

    /// Recursively pack with symlink resolution.
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,

    /// Number of worker threads.
    #[arg(short = 'j', long = "jobs", value_name = "N")]
    jobs: Option<usize>,

    /// Add an extra ignore file.
    #[arg(short = 'a', long = "add-ignore", value_name = "FILE")]
    add_ignore: Vec<PathBuf>,

    /// Do not use the default `.packignore` ignore file.
    #[arg(short = 'd', long = "delete-default-ignore")]
    delete_default_ignore: bool,

    /// Overwrite without prompting.
    #[arg(short = 'y', long = "yes", conflicts_with = "no")]
    yes: bool,

    /// Abort if output exists.
    #[arg(short = 'n', long = "no", conflicts_with = "yes")]
    no: bool,
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "List files that will be included")]
struct CatAddFileArgs {
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input: PathBuf,

    /// Recursively resolve symlinks.
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,

    #[arg(short = 'j', long = "jobs", value_name = "N")]
    jobs: Option<usize>,
    #[arg(short = 'a', long = "add-ignore", value_name = "FILE")]
    add_ignore: Vec<PathBuf>,
    #[arg(short = 'd', long = "delete-default-ignore")]
    delete_default_ignore: bool,
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "List files that will be ignored")]
struct CatIgnoreFileArgs {
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input: PathBuf,

    /// Recursively resolve symlinks.
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,

    #[arg(short = 'j', long = "jobs", value_name = "N")]
    jobs: Option<usize>,
    #[arg(short = 'a', long = "add-ignore", value_name = "FILE")]
    add_ignore: Vec<PathBuf>,
    #[arg(short = 'd', long = "delete-default-ignore")]
    delete_default_ignore: bool,
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "List all files with include/exclude status (colored)")]
struct CatFileArgs {
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input: PathBuf,

    /// Recursively resolve symlinks.
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,

    #[arg(short = 'j', long = "jobs", value_name = "N")]
    jobs: Option<usize>,
    #[arg(short = 'a', long = "add-ignore", value_name = "FILE")]
    add_ignore: Vec<PathBuf>,
    #[arg(short = 'd', long = "delete-default-ignore")]
    delete_default_ignore: bool,
}

fn main() {
    let cli = Cli::parse();
    match cli {
        Cli::Pack(args) => if let Err(err) = run_pack(args) { eprintln!("tarpack: {err}"); std::process::exit(1); },
        Cli::CatAddFile(args) => if let Err(err) = run_cat_add_file(args) { eprintln!("tarpack: {err}"); std::process::exit(1); },
        Cli::CatIgnoreFile(args) => if let Err(err) = run_cat_ignore_file(args) { eprintln!("tarpack: {err}"); std::process::exit(1); },
        Cli::CatFile(args) => if let Err(err) = run_cat_file(args) { eprintln!("tarpack: {err}"); std::process::exit(1); },
    }
}

fn run_pack(args: PackArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = fs::canonicalize(&args.input).map_err(|e| {
        format!("cannot access input directory `{}`: {e}", args.input.display())
    })?;
    if !input.is_dir() {
        return Err(format!("input `{}` is not a directory", input.display()).into());
    }

    let jobs = match args.jobs {
        Some(n) if n > 0 => n,
        Some(_) => { return Err(format!("invalid thread count `{n}`: must be a positive number", n = 0).into()); }
        None => default_jobs(),
    };

    validate_output(&input, &args.output, args.yes, args.no)?;

    // Build the walker.
    let mut walker = WalkBuilder::new(&input);
    configure_walker(
        &mut walker,
        &args.add_ignore,
        args.delete_default_ignore,
        args.recursive,
    )?;

    // Bounded channel
    let (tx, rx) = mpsc::sync_channel::<Result<(PathBuf, PathBuf, bool), String>>(jobs.saturating_mul(2));

    let output_path = args.output.clone();
    let writer_handle = std::thread::spawn(move || -> Result<usize, String> {
        let mut tar = Builder::new(File::create(&output_path).map_err(|e| format!("cannot create output `{}`: {e}", output_path.display()))?);
        tar.follow_symlinks(args.recursive);

        let mut added = 0usize;
        for item in rx {
            let (path, rel, is_dir) = item?;
            // Skip the root directory itself; only archive its contents.
            if rel.as_os_str().is_empty() {
                continue;
            }
            if is_dir {
                tar.append_dir(&rel, &path).map_err(|e| e.to_string())?;
            } else {
                tar.append_path_with_name(&path, &rel).map_err(|e| e.to_string())?;
            }
            added += 1;
        }
        tar.finish().map_err(|e| e.to_string())?;
        Ok(added)
    });

    // Traverse the input directory in parallel using `jobs` worker threads.
    // Each worker thread gets its own visitor (built lazily by the factory
    // below), which forwards the discovered entries to the writer thread.
    let input_for_visitor = input.clone();
    let tx_for_visitor = tx.clone();
    walker.threads(jobs).build_parallel().run(move || {
        let tx = tx_for_visitor.clone();
        let input_ref = input_for_visitor.clone();
        Box::new(move |result: Result<ignore::DirEntry, ignore::Error>| -> WalkState {
            let entry = match result {
                Ok(entry) => entry,
                Err(err) => {
                    let _ = tx.send(Err(format!("error while walking input: {err}")));
                    return WalkState::Continue;
                }
            };

            let path = entry.path();
            let Ok(rel) = path.strip_prefix(&input_ref) else {
                return WalkState::Continue;
            };
            if rel.as_os_str().is_empty() {
                return WalkState::Continue;
            }

            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if tx
                .send(Ok((path.to_path_buf(), rel.to_path_buf(), is_dir)))
                .is_err()
            {
                // Writer thread is gone; stop traversing.
                return WalkState::Quit;
            }
            WalkState::Continue
        })
    });

    // All walker threads have finished and their channel senders are dropped;
    // closing the last sender lets the writer thread see the end of the stream.
    drop(tx);

    let added = writer_handle
        .join()
        .map_err(|_| "archive writer thread panicked".to_string())??;

    println!(
        "Packed {} item{} from `{}` into `{}` ({} thread{})",
        added,
        if added == 1 { "" } else { "s" },
        input.display(),
        args.output.display(),
        jobs,
        if jobs == 1 { "" } else { "s" },
    );

    Ok(())
}

fn run_cat_add_file(args: CatAddFileArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = fs::canonicalize(&args.input).map_err(|e| {
        format!("cannot access input directory `{}`: {e}", args.input.display())
    })?;
    if !input.is_dir() {
        return Err(format!("input `{}` is not a directory", input.display()).into());
    }

    let _jobs = match args.jobs {
        Some(n) if n > 0 => n,
        Some(_) => {
            return Err(format!("invalid thread count `{n}`: must be a positive number", n = 0).into())
        }
        None => default_jobs(),
    };

    let entries = collect_entries(&input, &args.add_ignore, args.delete_default_ignore, args.recursive)?;
    let mut count = 0;
    for e in &entries {
        if !e.ignored {
            print_entry(e, false);
            count += 1;
        }
    }
    println!("\nTotal: {} files/folders to add", count);
    Ok(())
}

fn run_cat_ignore_file(args: CatIgnoreFileArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = fs::canonicalize(&args.input).map_err(|e| {
        format!("cannot access input directory `{}`: {e}", args.input.display())
    })?;
    if !input.is_dir() {
        return Err(format!("input `{}` is not a directory", input.display()).into());
    }

    let _jobs = match args.jobs {
        Some(n) if n > 0 => n,
        Some(_) => {
            return Err(format!("invalid thread count `{n}`: must be a positive number", n = 0).into())
        }
        None => default_jobs(),
    };

    let entries = collect_entries(&input, &args.add_ignore, args.delete_default_ignore, args.recursive)?;
    let mut count = 0;
    for e in &entries {
        if e.ignored {
            print_entry(e, true);
            count += 1;
        }
    }
    println!("\nTotal: {} files/folders ignored", count);
    Ok(())
}

fn run_cat_file(args: CatFileArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = fs::canonicalize(&args.input).map_err(|e| {
        format!("cannot access input directory `{}`: {e}", args.input.display())
    })?;
    if !input.is_dir() {
        return Err(format!("input `{}` is not a directory", input.display()).into());
    }

    let _jobs = match args.jobs {
        Some(n) if n > 0 => n,
        Some(_) => {
            return Err(format!("invalid thread count `{n}`: must be a positive number", n = 0).into())
        }
        None => default_jobs(),
    };

    let entries = collect_entries(&input, &args.add_ignore, args.delete_default_ignore, args.recursive)?;
    let mut included = 0;
    let mut ignored = 0;
    for e in &entries {
        if e.ignored {
            ignored += 1;
        } else {
            included += 1;
        }
        print_entry(e, e.ignored);
    }

    println!("\nTotal: {} items", included + ignored);
    println!("Add: {}    Ignore: {}", included, ignored);
    Ok(())
}

/// Default number of worker threads: the number of available CPU cores.
fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Applies the common traversal settings to a [`WalkBuilder`].
///
/// These settings mirror the packing semantics: hidden files are included,
/// only `.packignore` (plus any `--add-ignore` files) is honored, and no
/// global/parent git rules are consulted.
fn configure_walker(
    walker: &mut ignore::WalkBuilder,
    add_ignore: &[PathBuf],
    delete_default_ignore: bool,
    follow_links: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    walker
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(follow_links)
        .sort_by_file_path(|a, b| a.cmp(b));

    if !delete_default_ignore {
        walker.add_custom_ignore_filename(".packignore");
    }
    for add in add_ignore {
        if let Some(err) = walker.add_ignore(add) {
            return Err(format!("failed to add ignore file `{}`: {err}", add.display()).into());
        }
    }
    Ok(())
}

/// Builds a [`Gitignore`] from the default `.packignore` file (unless disabled)
/// plus any `--add-ignore` files, rooted at `input`.
///
/// *Only* the top-level rule files are consulted here, matching the documented
/// behavior that the tool does not read parent-directory ignore files.
fn build_ignore_rules(
    input: &Path,
    add_ignore: &[PathBuf],
    delete_default_ignore: bool,
) -> Result<ignore::gitignore::Gitignore, Box<dyn std::error::Error>> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(input);

    if !delete_default_ignore {
        let packignore = input.join(".packignore");
        if packignore.exists() {
            if builder.add(&packignore).is_some() {
                eprintln!("warning: could not load .packignore");
            }
        }
    }
    for add in add_ignore {
        if builder.add(add).is_some() {
            return Err(format!("failed to add ignore file `{}`", add.display()).into());
        }
    }
    Ok(builder.build().map_err(|e| e.to_string())?)
}

/// Compile-time platform flag used where the same code must behave differently
/// on Windows vs Unix. `cfg!` is evaluated at compile time, not runtime.
const IS_WINDOWS: bool = cfg!(windows);

/// Returns `true` if `path` is a symbolic link (or a Windows junction).
///
/// Uses `symlink_metadata` so that the link itself is inspected, never its
/// target. The exact metadata call differs per platform, so it is selected by
/// `#[cfg]` at compile time.
fn is_symlink(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    let ft = meta.file_type();

    // `FileType::is_symlink()` is available on every platform. On Windows,
    // junction points are reparse points that are *also* reported as symlinks
    // by `is_symlink()`, but we explicitly consult the platform extension
    // methods selected here at compile time to be robust.
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        ft.is_symlink() || ft.is_symlink_dir() || ft.is_symlink_file()
    }
    #[cfg(not(windows))]
    {
        ft.is_symlink()
    }
}

/// Resolves the immediate target of a symlink/junction as a string, if any.
///
/// Returns `None` for non-links or when the target cannot be read. `read_link`
/// is cross-platform; the `#[cfg]` selection documents that the semantics can
/// differ (e.g. on Windows a link may point to a UNC path).
fn symlink_target(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        std::fs::read_link(path)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(windows)]
    {
        std::fs::read_link(path)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
}

/// A single discovered entry, fully classified so the `cat-*` commands can
/// render it without re-reading the filesystem.
struct EntryInfo {
    /// Path relative to the packing root, using the native separator.
    rel: PathBuf,
    is_dir: bool,
    is_symlink: bool,
    /// Resolved link target, when this entry is a symlink/junction.
    target: Option<String>,
    /// `true` = excluded by ignore rules (shown as `Ignore`).
    ignored: bool,
}

/// Recursively walks `input` and classifies every entry against the ignore
/// rules.
///
/// Unlike the pack walker, this walker does **not** apply ignore filtering,
/// so that ignored entries are also collected (they would otherwise be hidden
/// and could never be labelled). Classification is done afterwards by
/// [`build_ignore_rules`]. When `recursive` is set, symlinks are followed (the
/// same semantics as `pack -r`).
fn collect_entries(
    input: &Path,
    add_ignore: &[PathBuf],
    delete_default_ignore: bool,
    recursive: bool,
) -> Result<Vec<EntryInfo>, Box<dyn std::error::Error>> {
    let rules = build_ignore_rules(input, add_ignore, delete_default_ignore)?;

    let mut walker = WalkBuilder::new(input);
    // Deliberately do NOT register a custom ignore filename here so that
    // ignored entries are still visited; rules are applied manually below.
    walker
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(recursive)
        .sort_by_file_path(|a, b| a.cmp(b));

    let mut out = Vec::new();
    for entry in walker.build() {
        let entry = entry?;
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(input) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let link = is_symlink(path);
        let target = if link { symlink_target(path) } else { None };
        let ignored = entry_ignored(&rules, rel, is_dir);

        out.push(EntryInfo {
            rel: rel.to_path_buf(),
            is_dir,
            is_symlink: link,
            target,
            ignored,
        });
    }
    Ok(out)
}

/// Determines whether `rel` is excluded by the ignore rules, honoring the
/// gitignore rule that an ignored directory *and everything beneath it* are
/// skipped.
///
/// A single top-level `Gitignore::matched` call only answers for the exact
/// path, so a file like `build/x.txt` would not match the pattern `build/`
/// directly. This walks the path's ancestors so that children of an ignored
/// directory are also reported as ignored, exactly as the pack walker behaves.
fn entry_ignored(rules: &ignore::gitignore::Gitignore, rel: &Path, is_dir: bool) -> bool {
    let mut current = rel;
    loop {
        let dir = current != rel || is_dir;
        if rules.matched(current, dir).is_ignore() {
            return true;
        }
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => current = parent,
            _ => return false,
        }
    }
}

/// Prints one entry for a `cat-*` command.
///
/// A symlink is shown as `rel -> target`; ignored entries are marked `Ignore`
/// and non-ignored ones `Add`. The directory/filesystem kind is shown, and on
/// Windows a symlinked directory is reported as `link(dir)`: a native `\`
/// would otherwise be ambiguous. Colored output keeps the two states distinct.
fn print_entry(e: &EntryInfo, as_ignore: bool) {
    use colored::Colorize;

    let marker = if as_ignore {
        "[Ignore]".red()
    } else {
        "[Add]".green()
    };

    // `IS_WINDOWS` is a compile-time constant (`cfg!`), so this branch is
    // resolved when the binary is built, never at runtime.
    let kind = if e.is_symlink {
        if IS_WINDOWS && e.is_dir { "link(dir)" } else { "link" }
    } else if e.is_dir {
        "dir"
    } else if IS_WINDOWS {
        "file"
    } else {
        "file"
    };

    let mut line = format!("{marker} [{kind}] {}", e.rel.display());
    if let Some(ref t) = e.target {
        line.push_str(" -> ");
        line.push_str(t);
    }
    println!("{line}");
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
