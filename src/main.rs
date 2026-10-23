use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use clap::Parser;
use ignore::{WalkBuilder, WalkState};
use tar::Builder;

/// A tar packaging tool that honors `.packignore` (gitignore-compatible) rules.
///
/// Packs the contents of a directory into a single `tar` archive. By default
/// the `.packignore` files found in the packing folder (including nested ones)
/// are used to exclude files, exactly like `.gitignore`. Hidden files are
/// included by default.
#[derive(Parser, Debug)]
#[command(name = "tarpack", version, about, long_about = None)]
enum Cli {
    Pack(PackArgs),
    CatAddFile(CatAddFileArgs),
    CatIgnoreFile(CatIgnoreFileArgs),
    CatFile(CatFileArgs),
}

/// Options shared by every subcommand.
#[derive(clap::Args, Debug, Clone)]
struct CommonArgs {
    /// The input directory.
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input: PathBuf,

    /// Follow symlinks and operate on their targets instead of the links.
    ///
    /// Directory traversal is always recursive; this only controls whether a
    /// symlink/junction is dereferenced.
    #[arg(short = 'r', long = "recursive", visible_alias = "follow-symlinks")]
    recursive: bool,

    /// Number of traversal worker threads. Defaults to available CPU cores.
    #[arg(short = 'j', long = "jobs", value_name = "N")]
    jobs: Option<usize>,

    /// Additional ignore file, applied in addition to `.packignore` files.
    ///
    /// Relative FILE paths are resolved from the current directory. Rules are
    /// matched relative to the input directory. May be repeated.
    #[arg(short = 'a', long = "add-ignore", value_name = "FILE")]
    add_ignore: Vec<PathBuf>,

    /// Do not read `.packignore` files.
    #[arg(short = 'd', long = "delete-default-ignore")]
    delete_default_ignore: bool,
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "Pack a directory into a tar archive")]
struct PackArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// The output tar archive to write.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: PathBuf,

    /// Overwrite an existing output archive without prompting.
    #[arg(short = 'y', long = "yes", conflicts_with = "no")]
    yes: bool,

    /// Abort if the output archive already exists without prompting.
    #[arg(short = 'n', long = "no", conflicts_with = "yes")]
    no: bool,
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "List files that will be included")]
struct CatAddFileArgs {
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Parser, Debug)]
#[command(name = "tarpack", about = "List files that will be ignored")]
struct CatIgnoreFileArgs {
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Parser, Debug)]
#[command(
    name = "tarpack",
    about = "List all files with include/exclude status (colored)"
)]
struct CatFileArgs {
    #[command(flatten)]
    common: CommonArgs,
}

/// Upper bound for `--jobs`; protects against accidentally spawning thousands
/// of OS threads (which would make the process abort instead of failing
/// cleanly).
const MAX_JOBS: usize = 1024;

fn main() {
    let result = match Cli::parse() {
        Cli::Pack(args) => run_pack(args),
        Cli::CatAddFile(args) => run_cat_add_file(args),
        Cli::CatIgnoreFile(args) => run_cat_ignore_file(args),
        Cli::CatFile(args) => run_cat_file(args),
    };

    if let Err(err) = result {
        eprintln!("tarpack: {err}");
        std::process::exit(1);
    }
}

fn run_pack(args: PackArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = canonical_input(&args.common.input)?;
    let jobs = resolve_jobs(args.common.jobs)?;

    validate_output(&input, &args.output, args.yes, args.no)?;

    // Collect every entry first. This keeps the archive deterministic (the
    // entries are sorted) and means the output file is only created/truncated
    // once the traversal has succeeded.
    let entries = collect_entries(&input, &args.common, jobs, IgnoreMode::Apply)?;

    // Traversal can take a long time, so re-check the output right before
    // opening it: a path component (e.g. a symlink outside the input tree)
    // could have been changed to point back into the input after the initial
    // validation.
    ensure_output_outside_input(&input, &args.output)?;

    let mut tar = Builder::new(
        File::create(&args.output)
            .map_err(|e| format!("cannot create output `{}`: {e}", args.output.display()))?,
    );
    tar.follow_symlinks(args.common.recursive);

    for entry in &entries {
        let src = input.join(&entry.rel);
        let result = if entry.is_dir {
            tar.append_dir(&entry.rel, &src)
        } else {
            tar.append_path_with_name(&src, &entry.rel)
        };
        result.map_err(|e| format!("cannot add `{}`: {e}", entry.rel.display()))?;
    }

    tar.finish()
        .map_err(|e| format!("cannot finish archive `{}`: {e}", args.output.display()))?;

    println!(
        "Packed {} item{} from `{}` into `{}` ({} thread{})",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" },
        input.display(),
        args.output.display(),
        jobs,
        if jobs == 1 { "" } else { "s" },
    );

    Ok(())
}

fn run_cat_add_file(args: CatAddFileArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = canonical_input(&args.common.input)?;
    let jobs = resolve_jobs(args.common.jobs)?;

    let entries = collect_entries(&input, &args.common, jobs, IgnoreMode::Apply)?;
    for entry in &entries {
        print_entry(entry, false, args.common.recursive);
    }
    println!("\nTotal: {} files/folders to add", entries.len());
    Ok(())
}

fn run_cat_ignore_file(args: CatIgnoreFileArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = canonical_input(&args.common.input)?;
    let jobs = resolve_jobs(args.common.jobs)?;

    let included = collect_entries(&input, &args.common, jobs, IgnoreMode::Apply)?;
    let all = collect_entries(&input, &args.common, jobs, IgnoreMode::Nothing)?;
    let included: HashSet<&Path> = included.iter().map(|e| e.rel.as_path()).collect();

    let mut count = 0;
    for entry in &all {
        if !included.contains(entry.rel.as_path()) {
            print_entry(entry, true, args.common.recursive);
            count += 1;
        }
    }
    println!("\nTotal: {} files/folders ignored", count);
    Ok(())
}

fn run_cat_file(args: CatFileArgs) -> Result<(), Box<dyn std::error::Error>> {
    let input = canonical_input(&args.common.input)?;
    let jobs = resolve_jobs(args.common.jobs)?;

    let included = collect_entries(&input, &args.common, jobs, IgnoreMode::Apply)?;
    let all = collect_entries(&input, &args.common, jobs, IgnoreMode::Nothing)?;
    let included: HashSet<&Path> = included.iter().map(|e| e.rel.as_path()).collect();

    let mut added = 0;
    let mut ignored = 0;
    for entry in &all {
        let is_ignored = !included.contains(entry.rel.as_path());
        if is_ignored {
            ignored += 1;
        } else {
            added += 1;
        }
        print_entry(entry, is_ignored, args.common.recursive);
    }

    println!("\nTotal: {} items", added + ignored);
    println!("Add: {}    Ignore: {}", added, ignored);
    Ok(())
}

/// Default number of worker threads: the number of available CPU cores.
fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn resolve_jobs(jobs: Option<usize>) -> Result<usize, Box<dyn std::error::Error>> {
    match jobs {
        None => Ok(default_jobs()),
        Some(0) => Err("invalid thread count `0`: must be a positive number".into()),
        Some(n) if n > MAX_JOBS => {
            Err(format!("invalid thread count `{n}`: must be at most {MAX_JOBS}").into())
        }
        Some(n) => Ok(n),
    }
}

fn canonical_input(input: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = fs::canonicalize(input)
        .map_err(|e| format!("cannot access input directory `{}`: {e}", input.display()))?;
    if !path.is_dir() {
        return Err(format!("input `{}` is not a directory", path.display()).into());
    }
    Ok(path)
}

#[derive(Clone, Copy)]
enum IgnoreMode {
    /// Apply `.packignore` files and `--add-ignore` files (pack semantics).
    Apply,
    /// Do not apply any ignore rules; used to enumerate ignored entries.
    Nothing,
}

/// Builds a walker with the settings shared by `pack` and the `cat-*`
/// commands.
///
/// Hidden files are included, only `.packignore` (plus any `--add-ignore`
/// files) is honored, and no global/parent git rules are consulted.
fn build_walker(
    input: &Path,
    common: &CommonArgs,
    mode: IgnoreMode,
) -> Result<WalkBuilder, Box<dyn std::error::Error>> {
    let mut walker = WalkBuilder::new(input);
    walker
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(common.recursive)
        // Explicitly added ignore files are interpreted relative to the input
        // root (not the process CWD), matching the `cat-*` commands.
        .current_dir(input);

    if matches!(mode, IgnoreMode::Apply) {
        if !common.delete_default_ignore {
            walker.add_custom_ignore_filename(".packignore");
        }

        // Resolve relative `--add-ignore` paths from the process CWD (the
        // directory the user typed them in), then hand absolute paths to the
        // walker so its `current_dir` setting only affects pattern anchoring.
        let cwd = std::env::current_dir()
            .map_err(|e| format!("cannot determine current directory: {e}"))?;
        for add in &common.add_ignore {
            let path = if add.is_absolute() {
                add.clone()
            } else {
                cwd.join(add)
            };
            if let Some(err) = walker.add_ignore(&path) {
                return Err(format!("failed to add ignore file `{}`: {err}", add.display()).into());
            }
        }
    }

    Ok(walker)
}

/// A single discovered entry, fully classified so the `cat-*` commands can
/// render it without re-reading the filesystem.
struct EntryInfo {
    /// Path relative to the packing root, using the native separator.
    rel: PathBuf,
    /// Whether the entry is a directory *after* symlink handling (i.e. whether
    /// it is archived as a directory).
    is_dir: bool,
    is_symlink: bool,
    /// Resolved link target, when this entry is a symlink/junction.
    target: Option<String>,
    /// Whether the link target is a directory. `None` for non-links.
    target_is_dir: Option<bool>,
}

/// Walks `input` and returns every entry that the walker yields.
///
/// With [`IgnoreMode::Apply`] this is exactly the set of entries `pack` would
/// archive (the same walker configuration is used). With
/// [`IgnoreMode::Nothing`] the full tree is returned, which lets the `cat-*`
/// commands compute the ignored subset by difference. In both cases the
/// result is sorted by archive path, so output is deterministic even when the
/// traversal itself runs on multiple threads.
fn collect_entries(
    input: &Path,
    common: &CommonArgs,
    jobs: usize,
    mode: IgnoreMode,
) -> Result<Vec<EntryInfo>, Box<dyn std::error::Error>> {
    let mut walker = build_walker(input, common, mode)?;
    let mut entries = Vec::new();

    if jobs <= 1 {
        for result in walker.build() {
            let entry =
                result.map_err(|e| format!("error while walking `{}`: {e}", input.display()))?;
            if let Some(info) = entry_info(entry, input, common.recursive) {
                entries.push(info);
            }
        }
    } else {
        let collected: Mutex<Vec<EntryInfo>> = Mutex::new(Vec::new());
        let first_error: Mutex<Option<String>> = Mutex::new(None);

        walker.threads(jobs).build_parallel().run(|| {
            Box::new(|result| match result {
                Ok(entry) => {
                    if let Some(info) = entry_info(entry, input, common.recursive) {
                        collected.lock().unwrap().push(info);
                    }
                    WalkState::Continue
                }
                Err(err) => {
                    let mut slot = first_error.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(format!("error while walking `{}`: {err}", input.display()));
                    }
                    WalkState::Quit
                }
            })
        });

        if let Some(message) = first_error.into_inner().unwrap() {
            return Err(message.into());
        }
        entries = collected.into_inner().unwrap();
    }

    // `Path` ordering is component-wise, so a directory always sorts directly
    // before its children.
    entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(entries)
}

fn entry_info(entry: ignore::DirEntry, input: &Path, follow: bool) -> Option<EntryInfo> {
    let path = entry.path();
    let rel = match path.strip_prefix(input) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel,
        _ => return None,
    };

    // `path_is_symlink` comes from the walker, so regular files need no extra
    // `stat` call. On Windows, additionally consult the filesystem so junction
    // points are recognized as well.
    let is_symlink = entry.path_is_symlink() || (cfg!(windows) && is_symlink(path));
    let (is_dir, target, target_is_dir) = if is_symlink {
        let target = symlink_target(path);
        // `metadata` follows the link, which tells us what the target is even
        // when `-r` is not used (needed for `link(dir)` on Windows).
        let target_is_dir = fs::metadata(path).ok().map(|m| m.is_dir());
        let is_dir = if follow {
            target_is_dir.unwrap_or(false)
        } else {
            false
        };
        (is_dir, target, target_is_dir)
    } else {
        (entry.file_type().is_some_and(|ft| ft.is_dir()), None, None)
    };

    Some(EntryInfo {
        rel: rel.to_path_buf(),
        is_dir,
        is_symlink,
        target,
        target_is_dir,
    })
}

/// Returns `true` if `path` is a symbolic link (or a Windows junction).
///
/// `symlink_metadata` inspects the link itself, never its target.
fn is_symlink(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    let file_type = meta.file_type();

    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        // `is_symlink` covers symlinks; the extension methods additionally
        // cover junction points on older Rust toolchains.
        file_type.is_symlink() || file_type.is_symlink_dir() || file_type.is_symlink_file()
    }
    #[cfg(not(windows))]
    {
        file_type.is_symlink()
    }
}

/// Resolves the immediate target of a symlink/junction as a string, if any.
fn symlink_target(path: &Path) -> Option<String> {
    fs::read_link(path)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Prints one entry for a `cat-*` command.
///
/// Without `-r`, a symlink is shown as `rel -> target` with kind `link` (or
/// `link(dir)` on Windows, where the target type would otherwise be
/// ambiguous). With `-r`, the kind reflects what `pack -r` stores: the target
/// type (`dir` or `file`). Ignored entries are marked `[Ignore]` and included
/// ones `[Add]`.
fn print_entry(entry: &EntryInfo, as_ignore: bool, follow: bool) {
    use colored::Colorize;

    let marker = if as_ignore {
        "[Ignore]".red()
    } else {
        "[Add]".green()
    };

    let target_is_dir = entry.target_is_dir.unwrap_or(entry.is_dir);
    let kind = if entry.is_symlink && !follow {
        if cfg!(windows) && target_is_dir {
            "link(dir)"
        } else {
            "link"
        }
    } else if target_is_dir {
        "dir"
    } else {
        "file"
    };

    let mut line = format!("{marker} [{kind}] {}", entry.rel.display());
    if let Some(ref target) = entry.target {
        line.push_str(" -> ");
        line.push_str(target);
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
    ensure_output_outside_input(input, output)?;

    let out_abs = absolute_output_path(output)?;

    if out_abs.is_dir() {
        return Err(format!("output `{}` is a directory", output.display()).into());
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
    } else {
        // Use the absolute parent: `Path::parent` on a bare relative name
        // returns `Some("")`, which does not exist and used to reject
        // perfectly valid outputs such as `-o archive.tar`.
        let parent = out_abs.parent().unwrap_or_else(|| Path::new("."));
        if !parent.exists() {
            return Err(format!("output directory `{}` does not exist", parent.display()).into());
        }
    }

    Ok(())
}

/// Returns the output path as an absolute path, resolving it against the
/// current directory if necessary.
fn absolute_output_path(output: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if output.is_absolute() {
        Ok(output.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(output))
    }
}

/// Rejects an output path whose *resolved* location is inside the input tree.
///
/// The output path may legitimately be a symlink or contain symlinked parent
/// directories, so every component is resolved (including a dangling symlink in
/// the final component) before the containment check.
fn ensure_output_outside_input(
    input: &Path,
    output: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let out_abs = absolute_output_path(output)?;
    let out_resolved = resolve_output_path(&out_abs);
    if out_resolved.starts_with(input) {
        return Err(format!(
            "output `{}` must not be inside the packed folder `{}`",
            output.display(),
            input.display()
        )
        .into());
    }
    Ok(())
}

/// Resolves a path to an absolute, symlink-free form even when (some of) its
/// trailing components do not exist yet.
///
/// `fs::canonicalize` fails for a missing output file, so we recursively
/// resolve the parent and then follow a symlink in the final component (which
/// may be dangling). `depth` bounds symlink loops.
fn resolve_output_path(path: &Path) -> PathBuf {
    resolve_output_path_inner(path, 0)
}

fn resolve_output_path_inner(path: &Path, depth: usize) -> PathBuf {
    if let Ok(resolved) = fs::canonicalize(path) {
        return resolved;
    }
    if depth >= 40 {
        return path.to_path_buf();
    }

    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    let parent = resolve_output_path_inner(parent, depth + 1);
    let candidate = parent.join(name);
    match fs::read_link(&candidate) {
        Ok(target) => {
            let target = if target.is_absolute() {
                target
            } else {
                parent.join(target)
            };
            resolve_output_path_inner(&target, depth + 1)
        }
        Err(_) => candidate,
    }
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
