// OS-appropriate application data directory resolution.
//
// On Linux:   ~/.local/share/wyncast
// On macOS:   ~/Library/Application Support/wyncast
// On Windows: %APPDATA%\wyncast
//
// All config, database, and log files are stored here so the application
// does not litter files in whatever directory it happens to be launched from.

use std::path::PathBuf;

const APP_NAME: &str = "wyncast";

/// Returns the OS-standard application data directory for wyncast.
///
/// - Linux:   `~/.local/share/wyncast`
/// - macOS:   `~/Library/Application Support/wyncast`
/// - Windows: `%APPDATA%\wyncast`
///
/// Creates the directory if it does not already exist.
///
/// # Panics
///
/// Panics if the OS cannot provide a data directory (extremely rare; would
/// indicate a misconfigured home directory) or if the directory cannot be
/// created.
pub fn app_data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| {
        // Fallback: use a hidden directory in the user's home, or CWD as last resort.
        dirs::home_dir()
            .map(|h| h.join(".local").join("share"))
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .expect("cannot determine current directory for app data fallback")
            })
    });

    let dir = base.join(APP_NAME);

    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("failed to create app data directory {}: {e}", dir.display()));

    dir
}

/// Returns the path to the database file inside the app data directory.
///
/// Example: `~/.local/share/wyncast/draft-assistant.db`
pub fn db_path() -> PathBuf {
    app_data_dir().join("draft-assistant.db")
}

/// Returns the path to the log directory inside the app data directory,
/// creating it if necessary.
///
/// Example: `~/.local/share/wyncast/logs`
pub fn log_dir() -> PathBuf {
    let dir = app_data_dir().join("logs");
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("failed to create log directory {}: {e}", dir.display()));
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_contains_app_name() {
        let dir = app_data_dir();
        assert!(
            dir.to_str()
                .map(|s| s.contains(APP_NAME))
                .unwrap_or(false),
            "app data dir should contain the app name, got: {dir:?}"
        );
    }

    #[test]
    fn app_data_dir_is_created() {
        let dir = app_data_dir();
        assert!(dir.exists(), "app data directory should be created");
    }

    #[test]
    fn db_path_has_expected_filename() {
        let path = db_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("draft-assistant.db")
        );
    }

    #[test]
    fn log_dir_exists_after_call() {
        let dir = log_dir();
        assert!(dir.exists(), "log directory should be created");
    }
}
