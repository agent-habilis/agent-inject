//! Write one upload into the target directory without ever overwriting a
//! file that is already there.
//!
//! The body streams into a hidden `.part` file inside the target directory,
//! so the final link or rename never crosses a filesystem. The `.part` file is
//! removed on drop unless the upload finished, which covers an error, a panic,
//! a cancelled task, and ctrl-c.

use std::io;
use std::path::{Path, PathBuf};

use tokio::fs::{File, OpenOptions};

use super::name::candidates;

/// Give up on suffixes after this many. Reaching it means something other
/// than real collisions is going on.
const MAX_CANDIDATES: usize = 10_000;

const PART_PREFIX: &str = ".agent-inject-";
const PART_SUFFIX: &str = ".part";

#[derive(Debug)]
pub(crate) struct TempFile {
    path: PathBuf,
    file: File,
    armed: bool,
}

impl TempFile {
    pub(crate) async fn create(dir: &Path) -> io::Result<Self> {
        loop {
            let path = dir.join(format!(
                "{PART_PREFIX}{:016x}{PART_SUFFIX}",
                rand::random::<u64>()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        armed: true,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) fn file(&mut self) -> &mut File {
        &mut self.file
    }

    /// Sync the data and move it to the first free name among `name`,
    /// `name-2`, … Returns the final path.
    pub(crate) async fn finalize(mut self, dir: &Path, name: &str) -> io::Result<PathBuf> {
        self.file.sync_all().await?;
        let temp = self.path.clone();
        let dir = dir.to_owned();
        let name = name.to_owned();
        let target = tokio::task::spawn_blocking(move || {
            link_no_clobber(&temp, &dir, &name, |from, to| std::fs::hard_link(from, to))
        })
        .await
        .map_err(io::Error::other)??;
        self.armed = false;
        Ok(target)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Move `temp` to the first candidate name that does not exist yet.
///
/// A hard link is atomic and fails on an existing target, so two uploads of
/// the same name can never clobber each other. Where the filesystem has no
/// hard links (exFAT, some network shares), a zero-byte placeholder is
/// created with `create_new` first and then replaced by `rename`, which only
/// ever overwrites that placeholder.
fn link_no_clobber(
    temp: &Path,
    dir: &Path,
    name: &str,
    link: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    for candidate in candidates(name).take(MAX_CANDIDATES) {
        let target = dir.join(candidate);
        match link(temp, &target) {
            Ok(()) => {
                std::fs::remove_file(temp)?;
                return Ok(target);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
                ) =>
            {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                {
                    Ok(_) => {
                        std::fs::rename(temp, &target)?;
                        return Ok(target);
                    }
                    Err(placeholder) if placeholder.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(placeholder) => return Err(placeholder),
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(format!(
        "no free name for {name} after {MAX_CANDIDATES} tries"
    )))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use tokio::io::AsyncWriteExt;

    use super::{TempFile, link_no_clobber};

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    async fn write_temp(dir: &Path, body: &[u8]) -> TempFile {
        let mut temp = TempFile::create(dir).await.unwrap();
        temp.file().write_all(body).await.unwrap();
        temp
    }

    #[tokio::test]
    async fn existing_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"old").unwrap();

        let path = write_temp(dir.path(), b"new")
            .await
            .finalize(dir.path(), "a.jpg")
            .await
            .unwrap();

        assert_eq!(path, dir.path().join("a-2.jpg"));
        assert_eq!(std::fs::read(dir.path().join("a.jpg")).unwrap(), b"old");
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(names(dir.path()), ["a-2.jpg", "a.jpg"]);
    }

    #[tokio::test]
    async fn concurrent_uploads_of_one_name_get_distinct_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut tasks = Vec::new();
        for index in 0..20u8 {
            let dir = dir.path().to_owned();
            tasks.push(tokio::spawn(async move {
                write_temp(&dir, &[index])
                    .await
                    .finalize(&dir, "same.txt")
                    .await
                    .unwrap()
            }));
        }
        let mut bodies = Vec::new();
        for task in tasks {
            bodies.push(std::fs::read(task.await.unwrap()).unwrap()[0]);
        }
        bodies.sort_unstable();
        assert_eq!(bodies, (0..20).collect::<Vec<_>>());
        assert_eq!(names(dir.path()).len(), 20);
    }

    #[tokio::test]
    async fn dropped_temp_file_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        drop(write_temp(dir.path(), b"partial").await);
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn placeholder_fallback_when_hard_links_are_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
        let temp = dir.path().join("temp.part");
        std::fs::write(&temp, b"new").unwrap();

        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        let path = link_no_clobber(&temp, dir.path(), "a.txt", unsupported).unwrap();

        assert_eq!(path, dir.path().join("a-2.txt"));
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"old");
        assert!(!temp.exists());
    }
}
