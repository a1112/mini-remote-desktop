use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Secure relative-path validation failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathValidationError {
    /// Path was rejected for traversal, reserved names, streams, or unsafe syntax.
    #[error("path rejected: {0}")]
    Rejected(String),
    /// An approved root could not be resolved.
    #[error("root path could not be resolved: {0}")]
    RootUnavailable(String),
    /// A resolved target escaped its approved root.
    #[error("path escapes approved root")]
    EscapesRoot,
    /// A symlink/reparse point was encountered in a transfer path.
    #[error("symlink or reparse point is not allowed")]
    Symlink,
}

/// Validate and normalize a path supplied by a remote peer.
pub fn validate_relative_path(path: &str) -> Result<String, PathValidationError> {
    let path = path.trim();
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return Err(PathValidationError::Rejected("absolute or empty path".into()));
    }
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        let Component::Normal(value) = component else {
            return Err(PathValidationError::Rejected("parent or prefix component".into()));
        };
        let value = value
            .to_str()
            .ok_or_else(|| PathValidationError::Rejected("non-utf8 path component".into()))?;
        if value.is_empty()
            || value == "."
            || value == ".."
            || value.ends_with('.')
            || value.ends_with(' ')
            || value.contains(':')
            || value.chars().any(|ch| ch.is_control() || "<>\"|?*".contains(ch))
            || is_reserved_windows_name(value)
        {
            return Err(PathValidationError::Rejected(value.to_string()));
        }
        parts.push(value.to_string());
    }
    if parts.is_empty() {
        return Err(PathValidationError::Rejected("empty path".into()));
    }
    Ok(parts.join("/"))
}

fn is_reserved_windows_name(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem[3..].chars().all(|ch| ch.is_ascii_digit()))
}

/// Resolve a validated relative path beneath an approved root without
/// following symlinks/reparse points.
pub fn resolve_under_root(root: &Path, relative: &str) -> Result<PathBuf, PathValidationError> {
    let relative = validate_relative_path(relative)?;
    let root = root
        .canonicalize()
        .map_err(|error| PathValidationError::RootUnavailable(error.to_string()))?;
    let target = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    let mut current = root.clone();
    for component in target.strip_prefix(&root).expect("target starts at root").components() {
        let Component::Normal(component) = component else {
            return Err(PathValidationError::EscapesRoot);
        };
        current.push(component);
        if current.exists() {
            let metadata = std::fs::symlink_metadata(&current)
                .map_err(|error| PathValidationError::RootUnavailable(error.to_string()))?;
            if metadata.file_type().is_symlink() {
                return Err(PathValidationError::Symlink);
            }
        }
    }
    if let Ok(canonical) = target.canonicalize() {
        if !canonical.starts_with(&root) {
            return Err(PathValidationError::EscapesRoot);
        }
    } else if let Some(parent) = target.parent() {
        if let Ok(parent) = parent.canonicalize() {
            if !parent.starts_with(&root) {
                return Err(PathValidationError::EscapesRoot);
            }
        }
    }
    Ok(target)
}
