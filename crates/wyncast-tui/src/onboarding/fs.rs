// Filesystem abstraction for dependency injection.
//
// Production code uses `RealFileSystem` (actual `std::fs` calls).
// Tests inject `FakeFileSystem` (in-memory HashMap) to avoid disk I/O.

use std::path::Path;

/// Abstraction over filesystem operations needed by the onboarding module.
///
/// This trait exists so tests can inject a fake implementation that avoids
/// writing to disk. Production code uses [`RealFileSystem`].
pub trait FileSystem: Send + Sync {
    /// Read the entire contents of a file into a string.
    fn read_to_string(&self, path: &Path) -> std::io::Result<String>;

    /// Write `contents` to `path`, creating or overwriting the file.
    fn write(&self, path: &Path, contents: &str) -> std::io::Result<()>;

    /// Recursively create all directories in `path` if they don't exist.
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()>;

    /// Atomically rename `from` to `to`.
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
}

/// Real filesystem implementation using `std::fs`.
///
/// Writes use an atomic write-to-temp-then-rename pattern via the
/// [`OnboardingManager`](super::OnboardingManager) save logic, so
/// `write` here is a plain write and `rename` handles the atomic swap.
#[derive(Debug, Clone, Copy)]
pub struct RealFileSystem;

impl FileSystem for RealFileSystem {
    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write(&self, path: &Path, contents: &str) -> std::io::Result<()> {
        std::fs::write(path, contents)
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }
}
