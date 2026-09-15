use crate::{compiler::CompilerSourceFile, sha256};
use rustc_span::source_map::FileLoader;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 4096;

pub struct Capture {
    root: PathBuf,
    files: Mutex<BTreeMap<PathBuf, Arc<[u8]>>>,
}

impl Capture {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: Mutex::new(BTreeMap::new()),
        }
    }

    fn path(&self, path: &Path) -> io::Result<PathBuf> {
        let path = fs::canonicalize(self.root.join(path))?;
        if !path.starts_with(&self.root) || path.to_str().is_none() {
            return Err(io::Error::other("source path outside root or not UTF-8"));
        }
        Ok(path)
    }

    pub fn relative_path(&self, path: &Path) -> Option<String> {
        let path = self.path(path).ok()?;
        self.files.lock().unwrap().contains_key(&path).then(|| {
            path.strip_prefix(&self.root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
    }

    fn read(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        let path = self.path(path)?;
        let mut files = self.files.lock().unwrap();
        if let Some(bytes) = files.get(&path) {
            return Ok(bytes.clone());
        }
        if files.len() >= MAX_FILES {
            return Err(io::Error::other("source file budget exceeded"));
        }
        let used: usize = files.values().map(|bytes| bytes.len()).sum();
        let remaining = MAX_TOTAL_BYTES - used;
        let file = fs::File::from(rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )?);
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("source is not a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES.min(remaining as u64) + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES || bytes.len() > remaining {
            return Err(io::Error::other("source byte budget exceeded"));
        }
        let bytes: Arc<[u8]> = bytes.into();
        files.insert(path, bytes.clone());
        Ok(bytes)
    }

    pub fn files(&self) -> Vec<CompilerSourceFile> {
        self.files
            .lock()
            .unwrap()
            .iter()
            .map(|(path, bytes)| CompilerSourceFile {
                path: path
                    .strip_prefix(&self.root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .into(),
                sha256: sha256(bytes),
                byte_length: bytes.len() as u32,
            })
            .collect()
    }
}

pub struct Loader(pub Arc<Capture>);

impl FileLoader for Loader {
    fn file_exists(&self, path: &Path) -> bool {
        self.0.path(path).is_ok_and(|path| path.is_file())
    }
    fn read_file(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.0.read(path)?.to_vec()).map_err(io::Error::other)
    }
    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        self.0.read(path)
    }
    fn current_directory(&self) -> io::Result<PathBuf> {
        Ok(self.0.root.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_reads_use_identical_captured_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lib.rs");
        fs::write(&path, b"first\r\n").unwrap();
        let capture = Arc::new(Capture::new(directory.path().to_owned()));
        let loader = Loader(capture.clone());
        assert_eq!(loader.read_file(&path).unwrap(), "first\r\n");
        fs::write(&path, b"different\n").unwrap();
        assert_eq!(loader.read_file(&path).unwrap(), "first\r\n");
        assert_eq!(capture.files()[0].sha256, sha256(b"first\r\n"));
    }

    #[test]
    fn oversized_sources_are_not_added_to_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.rs");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let capture = Arc::new(Capture::new(directory.path().to_owned()));
        assert!(Loader(capture.clone()).read_file(&path).is_err());
        assert!(capture.files().is_empty());
    }

    #[test]
    fn special_files_are_rejected_without_blocking_for_a_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fifo.rs");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &path,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        let capture = Arc::new(Capture::new(directory.path().to_owned()));
        assert!(Loader(capture.clone()).read_file(&path).is_err());
        assert!(capture.files().is_empty());
    }

    #[test]
    fn captured_paths_are_sorted_and_symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("b.rs"), b"b").unwrap();
        fs::write(root.join("a.rs"), b"a").unwrap();
        fs::write(directory.path().join("outside.rs"), b"outside").unwrap();
        symlink(directory.path().join("outside.rs"), root.join("escape.rs")).unwrap();
        let capture = Arc::new(Capture::new(root));
        let loader = Loader(capture.clone());
        loader.read_file(Path::new("b.rs")).unwrap();
        loader.read_file(Path::new("a.rs")).unwrap();
        assert!(loader.read_file(Path::new("escape.rs")).is_err());
        assert_eq!(
            capture
                .files()
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["a.rs", "b.rs"]
        );
    }
}
