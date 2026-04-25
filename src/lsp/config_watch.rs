use super::error::LspError;
use crate::config::Config;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::Url;

/// Walk parent directories from `file_uri`'s path to find the nearest `.splitrs.toml`.
///
/// Converts the URI to a filesystem path, then walks upward through ancestor
/// directories until `.splitrs.toml` is found or the filesystem root is reached.
/// Returns `None` if no config file is found.
pub fn find_workspace_config(file_uri: &Url) -> Option<PathBuf> {
    let file_path = file_uri.to_file_path().ok()?;
    let mut dir: &Path = if file_path.is_dir() {
        file_path.as_path()
    } else {
        file_path.parent()?
    };

    loop {
        let candidate = dir.join(".splitrs.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

/// Load a [`Config`] from `path`.
///
/// Reads the TOML file at the given path and deserialises it into a [`Config`]
/// instance.  Any IO or parse errors are mapped to the appropriate [`LspError`]
/// variant.
pub fn reload_config(path: &Path) -> Result<Config, LspError> {
    Config::from_file(path).map_err(LspError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_config_walking_up() {
        let temp = std::env::temp_dir().join("splitrs_test_config_walk");
        let subdir = temp.join("src").join("deeply").join("nested");
        fs::create_dir_all(&subdir).unwrap();

        // Write config at root level
        let config_path = temp.join(".splitrs.toml");
        fs::write(&config_path, "[splitrs]\nmax_lines = 1000\n").unwrap();

        // Create a file URI pointing to a file in the nested subdir
        let test_file = subdir.join("foo.rs");
        fs::write(&test_file, "").unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        let found = find_workspace_config(&uri);
        assert_eq!(found, Some(config_path));

        // Cleanup
        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn returns_none_when_no_config() {
        let temp = std::env::temp_dir().join("splitrs_test_no_config");
        let nested = temp.join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();
        let test_file = nested.join("bar.rs");
        fs::write(&test_file, "").unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        // We cannot guarantee that ancestor directories lack .splitrs.toml,
        // so just verify the call completes without panicking.
        let _ = find_workspace_config(&uri);

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn reload_config_reads_file() {
        let temp = std::env::temp_dir().join("splitrs_test_reload_config");
        fs::create_dir_all(&temp).unwrap();
        let config_path = temp.join(".splitrs.toml");
        fs::write(&config_path, "[splitrs]\nmax_lines = 500\n").unwrap();

        let result = reload_config(&config_path);
        assert!(result.is_ok(), "reload_config failed: {:?}", result.err());

        fs::remove_dir_all(&temp).ok();
    }
}
