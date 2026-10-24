use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// A unique temporary directory that is removed when the test finishes.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tarpack-test-{}-{name}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tarpack<I, S>(cwd: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_tarpack"))
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .args(args)
        .output()
        .unwrap()
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Returns the archive entry names, normalized to `/` and without trailing
/// slashes on directories.
fn archive_paths(archive: &Path) -> Vec<String> {
    let file = File::open(archive).unwrap();
    let mut tar = tar::Archive::new(file);
    tar.entries()
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().replace('\\', "/");
            path.trim_end_matches('/').to_owned()
        })
        .collect()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Extracts the `path` part of lines like `[Add] [file] path -> target`.
fn listed_paths(output: &str, marker: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix(marker)?;
            let rest = rest.strip_prefix(" [")?;
            let end = rest.find("] ")?;
            let rest = &rest[end + 2..];
            // CLI output uses native separators, while archive paths always use
            // `/`; normalize so both sides can be compared on Windows too.
            Some(rest.split(" -> ").next().unwrap().replace('\\', "/"))
        })
        .collect()
}

#[test]
fn nested_packignore_is_honored_and_cat_matches_pack() {
    let temp = TempDir::new("nested");
    let input = temp.path().join("input");
    write(&input.join(".packignore"), "*.log\n");
    write(&input.join("nested/.packignore"), "*.txt\n");
    write(&input.join("nested/keep.rs"), "keep");
    write(&input.join("nested/drop.txt"), "drop");
    write(&input.join("nested/drop.log"), "drop");
    write(&input.join("src/main.rs"), "main");

    let archive = temp.path().join("out.tar");
    let output = tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            archive.as_os_str(),
            OsStr::new("-y"),
        ],
    );
    assert!(output.status.success(), "{output:?}");

    let packed = archive_paths(&archive);
    assert!(packed.contains(&"nested/keep.rs".to_owned()), "{packed:?}");
    assert!(
        !packed.contains(&"nested/drop.txt".to_owned()),
        "{packed:?}"
    );
    assert!(
        !packed.contains(&"nested/drop.log".to_owned()),
        "{packed:?}"
    );

    let cat = tarpack(
        temp.path(),
        [
            OsStr::new("cat-add-file"),
            OsStr::new("-i"),
            input.as_os_str(),
        ],
    );
    assert!(cat.status.success(), "{cat:?}");
    assert_eq!(listed_paths(&stdout(&cat), "[Add]"), packed);

    let ignore = tarpack(
        temp.path(),
        [
            OsStr::new("cat-ignore-file"),
            OsStr::new("-i"),
            input.as_os_str(),
        ],
    );
    assert!(ignore.status.success(), "{ignore:?}");
    let ignored = listed_paths(&stdout(&ignore), "[Ignore]");
    assert!(
        ignored.contains(&"nested/drop.txt".to_owned()),
        "{ignored:?}"
    );
    assert!(
        ignored.contains(&"nested/drop.log".to_owned()),
        "{ignored:?}"
    );
}

#[test]
fn bare_relative_output_name_is_accepted() {
    let temp = TempDir::new("relative-output");
    let input = temp.path().join("input");
    write(&input.join("file.txt"), "data");

    let output = tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            OsStr::new("archive.tar"),
            OsStr::new("-y"),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(temp.path().join("archive.tar").is_file());
}

#[test]
fn parallel_output_is_deterministic_and_sorted() {
    let temp = TempDir::new("deterministic");
    let input = temp.path().join("input");
    for dir in ["a", "b", "c", "d"] {
        for file in ["1", "2", "3", "4", "5"] {
            write(&input.join(dir).join(format!("{file}.txt")), file);
        }
    }

    let one = temp.path().join("one.tar");
    let many = temp.path().join("many.tar");
    assert!(tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            one.as_os_str(),
            OsStr::new("-j"),
            OsStr::new("1"),
            OsStr::new("-y"),
        ],
    )
    .status
    .success());
    assert!(tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            many.as_os_str(),
            OsStr::new("-j"),
            OsStr::new("8"),
            OsStr::new("-y"),
        ],
    )
    .status
    .success());

    let one = archive_paths(&one);
    let many = archive_paths(&many);
    assert_eq!(one, many);
    let mut sorted = one.clone();
    sorted.sort();
    assert_eq!(one, sorted);
}

#[cfg(unix)]
#[test]
fn symlinks_are_preserved_by_default_and_resolved_with_r() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new("symlinks");
    let input = temp.path().join("input");
    write(&input.join("real/file.txt"), "data");
    symlink("real", input.join("dirlink")).unwrap();
    symlink("real/file.txt", input.join("filelink")).unwrap();

    let plain = temp.path().join("plain.tar");
    let output = tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            plain.as_os_str(),
            OsStr::new("-y"),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let plain_paths = archive_paths(&plain);
    assert!(
        plain_paths.contains(&"dirlink".to_owned()),
        "{plain_paths:?}"
    );
    assert!(!plain_paths.contains(&"dirlink/file.txt".to_owned()));

    let followed = temp.path().join("followed.tar");
    let output = tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            followed.as_os_str(),
            OsStr::new("-r"),
            OsStr::new("-y"),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let followed_paths = archive_paths(&followed);
    assert!(
        followed_paths.contains(&"dirlink/file.txt".to_owned()),
        "{followed_paths:?}"
    );

    let cat = tarpack(
        temp.path(),
        [
            OsStr::new("cat-file"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-r"),
        ],
    );
    let cat = stdout(&cat);
    assert!(cat.contains("[dir] dirlink"), "{cat}");
    assert!(cat.contains("[file] filelink"), "{cat}");
}

#[cfg(unix)]
#[test]
fn output_cannot_escape_into_input_through_a_dangling_symlink() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new("containment");
    let input = temp.path().join("input");
    write(&input.join("file.txt"), "data");
    let inside = input.join("inside.tar");
    symlink(&inside, temp.path().join("out.tar")).unwrap();

    let output = tarpack(
        temp.path(),
        [
            OsStr::new("pack"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-o"),
            OsStr::new("out.tar"),
            OsStr::new("-y"),
        ],
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(!inside.exists());
}

#[test]
fn invalid_jobs_report_the_actual_value() {
    let temp = TempDir::new("jobs");
    let input = temp.path().join("input");
    write(&input.join("file.txt"), "data");

    let output = tarpack(
        temp.path(),
        [
            OsStr::new("cat-file"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-j"),
            OsStr::new("0"),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`0`"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn output_symlink_outside_pointing_into_input_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new("output-link");
    let input = temp.path().join("input");
    write(&input.join("keep.txt"), "keep");
    write(&input.join("target.tar"), "old contents");

    let run = |output: &Path| {
        tarpack(
            temp.path(),
            [
                OsStr::new("pack"),
                OsStr::new("-i"),
                input.as_os_str(),
                OsStr::new("-o"),
                output.as_os_str(),
                OsStr::new("-y"),
            ],
        )
    };

    // The link itself lives outside the input tree, but its target does not.
    let link = temp.path().join("existing.tar");
    symlink(input.join("target.tar"), &link).unwrap();
    let output = run(&link);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(input.join("target.tar")).unwrap(),
        "old contents"
    );

    // Same, but the target does not exist yet.
    let link = temp.path().join("missing.tar");
    symlink(input.join("new.tar"), &link).unwrap();
    let output = run(&link);
    assert!(!output.status.success(), "{output:?}");
    assert!(!input.join("new.tar").exists());
}

#[cfg(unix)]
#[test]
fn closed_stdout_pipe_does_not_panic() {
    let temp = TempDir::new("broken-pipe");
    let input = temp.path().join("input");
    // More output than a typical pipe buffer so the writer reliably sees
    // EPIPE after the reader goes away, regardless of scheduling.
    for i in 0..2000 {
        let name = format!("{i:04}-{}", "x".repeat(180));
        write(&input.join(name), "data");
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_tarpack"))
        .current_dir(temp.path())
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .args([OsStr::new("cat-file"), OsStr::new("-i"), input.as_os_str()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Close the read end while tarpack is still producing output.
    drop(child.stdout.take());

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn output_parent_symlink_to_input_or_child_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new("output-parent-link");
    let input = temp.path().join("input");
    write(&input.join("keep.txt"), "keep");
    write(&input.join("sub/keep.txt"), "keep");

    let run = |output: &Path| {
        tarpack(
            temp.path(),
            [
                OsStr::new("pack"),
                OsStr::new("-i"),
                input.as_os_str(),
                OsStr::new("-o"),
                output.as_os_str(),
                OsStr::new("-y"),
            ],
        )
    };

    // Parent symlink -> input directory.
    let parent = temp.path().join("parent");
    symlink(&input, &parent).unwrap();
    let output = run(&parent.join("root.tar"));
    assert!(!output.status.success(), "{output:?}");
    assert!(!input.join("root.tar").exists());

    // Parent symlink -> a subdirectory of the input directory.
    let parent = temp.path().join("sub-parent");
    symlink(input.join("sub"), &parent).unwrap();
    let output = run(&parent.join("sub.tar"));
    assert!(!output.status.success(), "{output:?}");
    assert!(!input.join("sub/sub.tar").exists());

    // Dangling parent symlink that points into a not-yet-existing input child.
    let parent = temp.path().join("dangling-parent");
    symlink(input.join("newdir"), &parent).unwrap();
    let output = run(&parent.join("new.tar"));
    assert!(!output.status.success(), "{output:?}");
    assert!(!input.join("newdir").exists());
}
