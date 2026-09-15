use anyhow::{Context, Result, ensure};
use atlas_model::{
    BuildContext, FileId, RepositoryId, SourceFile, SourceId, SourceSnapshot, digest,
};
use rustix::fs::{Dir, FileType, Mode, OFlags, fstat, open, openat};
use std::{collections::BTreeMap, fs::File, io::Read, os::fd::AsFd, path::Path};

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub profile: String,
    pub target: String,
    pub features: Vec<String>,
    pub default_features: bool,
    pub cfg: BTreeMap<String, Option<String>>,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub explicit_context: Option<BuildContext>,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            profile: "source".into(),
            target: "unspecified".into(),
            features: vec![],
            default_features: true,
            cfg: BTreeMap::new(),
            max_files: 20_000,
            max_file_bytes: 4 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            explicit_context: None,
        }
    }
}

#[derive(Default, PartialEq, Eq)]
struct Capture {
    files: BTreeMap<String, String>,
    manifests: BTreeMap<String, String>,
    warnings: Vec<String>,
    bytes: u64,
    entries: usize,
}

/// Captures untracked and modified Rust files, without invoking repository tools.
/// Equal passes verify the selected inputs, but do not provide a filesystem transaction.
pub fn capture(
    workspace: &Path,
    options: &CaptureOptions,
) -> Result<(SourceSnapshot, BuildContext)> {
    capture_with_hook(workspace, options, || {})
}

fn capture_with_hook(
    workspace: &Path,
    options: &CaptureOptions,
    between_passes: impl FnOnce(),
) -> Result<(SourceSnapshot, BuildContext)> {
    ensure!(
        options.max_files > 0 && options.max_file_bytes > 0,
        "capture limits must be positive"
    );
    let root = open(
        workspace,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .with_context(|| format!("open source root {}", workspace.display()))?;
    let first = scan(&root, options)?;
    between_passes();
    ensure!(
        first == scan(&root, options)?,
        "source changed during capture; retry with a stable working tree"
    );
    ensure!(
        !first.files.is_empty(),
        "no readable Rust source found in {}",
        workspace.display()
    );
    let repository_id = RepositoryId(digest(
        "repository",
        &workspace.canonicalize()?.as_os_str().as_encoded_bytes(),
    ));
    let source_id = SourceId(digest(
        "source",
        &(
            &repository_id,
            &first.files,
            &first.manifests,
            &first.warnings,
        ),
    ));
    let files = first
        .files
        .into_iter()
        .map(|(path, text)| SourceFile {
            id: FileId(digest("file", &(&source_id, &path))),
            content_hash: digest("content", &text),
            path,
            text,
        })
        .collect();
    let source = SourceSnapshot {
        revision: format!("working-tree {}", &source_id.0[7..19]),
        id: source_id,
        repository_id,
        files,
        manifests: first.manifests,
        warnings: first.warnings,
    };
    let context = super::context::build_context(&source, options)?;
    Ok((source, context))
}

fn scan(root: impl AsFd, options: &CaptureOptions) -> Result<Capture> {
    let mut capture = Capture::default();
    scan_dir(root, "", options, &mut capture, 0)?;
    capture.warnings.sort();
    Ok(capture)
}

fn scan_dir(
    directory: impl AsFd,
    parent: &str,
    options: &CaptureOptions,
    capture: &mut Capture,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 128, "source directory depth exceeds 128");
    let mut entries = Dir::read_from(&directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by(|a, b| a.file_name().to_bytes().cmp(b.file_name().to_bytes()));
    for entry in entries {
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        capture.entries += 1;
        ensure!(
            capture.entries <= options.max_files.saturating_mul(32),
            "directory entry budget exhausted"
        );
        let Ok(name) = std::str::from_utf8(bytes) else {
            let encoded = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            capture.warnings.push(format!(
                "unsupported non-UTF-8 path under {parent}: hex:{encoded}"
            ));
            continue;
        };
        if matches!(
            name,
            ".git" | "target" | "node_modules" | ".atlas" | ".ferrum-atlas"
        ) {
            continue;
        }
        let path = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        match entry.file_type() {
            FileType::Symlink => capture.warnings.push(format!("symlink omitted: {path}")),
            FileType::Directory => {
                let child = openat(
                    &directory,
                    entry.file_name(),
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .with_context(|| {
                    format!("source directory changed or became inaccessible: {path}")
                })?;
                scan_dir(child, &path, options, capture, depth + 1)?;
            }
            FileType::RegularFile => {
                let rust_source = name.ends_with(".rs");
                let manifest = matches!(
                    name,
                    "Cargo.toml" | "Cargo.lock" | "rust-toolchain.toml" | "rust-toolchain"
                ) || (name == "config.toml" && parent.ends_with(".cargo"));
                if !rust_source && !manifest {
                    continue;
                }
                ensure!(
                    capture.files.len() + capture.manifests.len() < options.max_files,
                    "source file budget exhausted"
                );
                // A held parent descriptor and NOFOLLOW prevent path swaps from escaping the root.
                let fd = openat(
                    &directory,
                    entry.file_name(),
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )?;
                let before = fstat(&fd)?;
                ensure!(
                    FileType::from_raw_mode(before.st_mode) == FileType::RegularFile,
                    "source changed file type: {path}"
                );
                ensure!(
                    before.st_size >= 0 && before.st_size as u64 <= options.max_file_bytes,
                    "file exceeds byte budget: {path}"
                );
                let mut file = File::from(fd);
                let mut content = Vec::new();
                (&mut file)
                    .take(options.max_file_bytes + 1)
                    .read_to_end(&mut content)?;
                ensure!(
                    content.len() as u64 <= options.max_file_bytes,
                    "file exceeds byte budget: {path}"
                );
                let after = fstat(&file)?;
                ensure!(
                    before.st_size == after.st_size
                        && before.st_mtime == after.st_mtime
                        && before.st_mtime_nsec == after.st_mtime_nsec,
                    "source changed while reading: {path}"
                );
                capture.bytes = capture
                    .bytes
                    .checked_add(content.len() as u64)
                    .context("capture byte overflow")?;
                ensure!(
                    capture.bytes <= options.max_total_bytes,
                    "total source byte budget exhausted"
                );
                match String::from_utf8(content) {
                    Ok(text) if rust_source => {
                        capture.files.insert(path, text);
                    }
                    Ok(text) => {
                        capture.manifests.insert(path, text);
                    }
                    Err(_) => capture
                        .warnings
                        .push(format!("non-UTF-8 source omitted: {path}")),
                }
            }
            _ => capture
                .warnings
                .push(format!("non-regular entry omitted: {path}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        dir
    }
    #[test]
    fn deterministic_source_and_configuration_identity() {
        let dir = project();
        let first = capture(dir.path(), &CaptureOptions::default()).unwrap();
        assert_eq!(
            first,
            capture(dir.path(), &CaptureOptions::default()).unwrap()
        );
        let options = CaptureOptions {
            target: "custom-firmware".into(),
            cfg: BTreeMap::from([("firmware".into(), None)]),
            ..Default::default()
        };
        let second = capture(dir.path(), &options).unwrap();
        assert_eq!(first.0.id, second.0.id);
        assert_ne!(first.1.id, second.1.id);
        assert_eq!(second.1.target, "custom-firmware");
    }
    #[test]
    fn detects_mutation_addition_and_removal_between_passes() {
        for change in [0, 1, 2] {
            let dir = project();
            let result =
                capture_with_hook(dir.path(), &CaptureOptions::default(), || match change {
                    0 => fs::write(dir.path().join("src/lib.rs"), "fn changed() {}").unwrap(),
                    1 => fs::write(dir.path().join("src/new.rs"), "fn added() {}").unwrap(),
                    _ => fs::remove_file(dir.path().join("src/lib.rs")).unwrap(),
                });
            assert!(result.unwrap_err().to_string().contains("source changed"));
        }
    }
    #[test]
    fn preserves_unicode_and_crlf_exactly() {
        let dir = project();
        let text = "pub fn caf\u{e9}() { let s = \"\u{1f680}\"; }\r\n";
        fs::write(dir.path().join("src/lib.rs"), text).unwrap();
        let (source, _) = capture(dir.path(), &CaptureOptions::default()).unwrap();
        assert_eq!(source.files[0].text, text);
        assert_eq!(source.files[0].content_hash, digest("content", text));
    }
    #[test]
    fn excludes_symlinks_non_utf8_and_build_execution() {
        use std::os::unix::{ffi::OsStringExt, fs::symlink};
        let dir = project();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.rs"), "fn private() {}").unwrap();
        symlink(outside.path(), dir.path().join("linked")).unwrap();
        symlink(
            outside.path().join("private.rs"),
            dir.path().join("src/linked.rs"),
        )
        .unwrap();
        fs::write(
            dir.path()
                .join(std::ffi::OsString::from_vec(vec![0xff, b'.', b'r', b's'])),
            "fn invalid_path() {}",
        )
        .unwrap();
        fs::write(dir.path().join("src/invalid.rs"), [0xff]).unwrap();
        fs::write(
            dir.path().join("build.rs"),
            "fn main() { panic!(\"must not run\"); }",
        )
        .unwrap();
        let (source, context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
        assert_eq!(source.files.len(), 2);
        assert_eq!(source.warnings.len(), 4);
        assert!(
            !source
                .files
                .iter()
                .any(|file| file.text.contains("private"))
        );
        assert_eq!(context.trust, "read_only");
    }
    #[test]
    fn enforces_file_and_total_budgets() {
        let dir = project();
        for options in [
            CaptureOptions {
                max_file_bytes: 3,
                ..Default::default()
            },
            CaptureOptions {
                max_total_bytes: 4,
                ..Default::default()
            },
            CaptureOptions {
                max_files: 1,
                ..Default::default()
            },
        ] {
            assert!(
                capture(dir.path(), &options)
                    .unwrap_err()
                    .to_string()
                    .contains("budget")
            );
        }
    }
}
