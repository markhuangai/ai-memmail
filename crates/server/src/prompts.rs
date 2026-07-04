use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("prompt path must be relative to prompt root: {0}")]
    Absolute(PathBuf),
    #[error("prompt path must not escape prompt root: {0}")]
    Escape(PathBuf),
    #[error("failed to read prompt {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("prompt is empty: {0}")]
    Empty(PathBuf),
}

pub fn resolve_prompt_path(root: &Path, prompt_path: &Path) -> Result<PathBuf, PromptError> {
    if prompt_path.is_absolute() {
        return Err(PromptError::Absolute(prompt_path.to_path_buf()));
    }
    if prompt_path.components().any(is_escape_component) {
        return Err(PromptError::Escape(prompt_path.to_path_buf()));
    }
    Ok(root.join(prompt_path))
}

fn is_escape_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::Prefix(_) | Component::RootDir
    )
}

pub fn read_prompt(root: &Path, prompt_path: &Path) -> Result<String, PromptError> {
    let resolved = resolve_prompt_path(root, prompt_path)?;
    let content = fs::read_to_string(&resolved).map_err(|source| PromptError::Read {
        path: resolved.clone(),
        source,
    })?;
    if content.trim().is_empty() {
        return Err(PromptError::Empty(resolved));
    }
    Ok(content)
}

pub fn prompt_is_missing(root: &Path, prompt_path: &Path) -> Result<bool, PromptError> {
    let resolved = resolve_prompt_path(root, prompt_path)?;
    match fs::metadata(&resolved) {
        Ok(_) => Ok(false),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(true),
        Err(source) => Err(PromptError::Read {
            path: resolved,
            source,
        }),
    }
}

impl PromptError {
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::Read { source, .. } if source.kind() == ErrorKind::NotFound
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_prompt_path_under_root() {
        let resolved = resolve_prompt_path(Path::new("prompts"), Path::new("support.md")).unwrap();
        assert_eq!(resolved, PathBuf::from("prompts/support.md"));

        let nested =
            resolve_prompt_path(Path::new("prompts"), Path::new("support/agent.md")).unwrap();
        assert_eq!(nested, PathBuf::from("prompts/support/agent.md"));
    }

    #[test]
    fn rejects_absolute_prompt_path() {
        let error = resolve_prompt_path(Path::new("prompts"), Path::new("/tmp/prompt.md"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("prompt path must be relative"));
    }

    #[test]
    fn rejects_parent_directory_prompt_path() {
        let error = resolve_prompt_path(Path::new("prompts"), Path::new("../config/config.yaml"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("must not escape prompt root"));
    }

    #[test]
    fn reads_non_empty_prompt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("prompt.md"), "Answer carefully.").unwrap();
        let content = read_prompt(dir.path(), Path::new("prompt.md")).unwrap();
        assert_eq!(content, "Answer carefully.");
    }

    #[test]
    fn read_prompt_reports_missing_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = read_prompt(dir.path(), Path::new("missing.md"))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("failed to read prompt"));

        std::fs::write(dir.path().join("empty.md"), "  \n").unwrap();
        let empty = read_prompt(dir.path(), Path::new("empty.md"))
            .unwrap_err()
            .to_string();
        assert!(empty.contains("prompt is empty"));
    }

    #[test]
    fn prompt_is_missing_reports_only_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("prompt.md"), "Answer carefully.").unwrap();

        assert!(!prompt_is_missing(dir.path(), Path::new("prompt.md")).unwrap());
        assert!(prompt_is_missing(dir.path(), Path::new("missing.md")).unwrap());

        let error = prompt_is_missing(dir.path(), Path::new("../missing.md")).unwrap_err();
        assert!(!error.is_not_found());
        assert!(error.to_string().contains("must not escape prompt root"));

        let missing = read_prompt(dir.path(), Path::new("missing.md")).unwrap_err();
        assert!(missing.is_not_found());

        let file_root = dir.path().join("not-a-directory");
        std::fs::write(&file_root, "not a directory").unwrap();
        let error = prompt_is_missing(&file_root, Path::new("prompt.md")).unwrap_err();
        assert!(!error.is_not_found());
        assert!(error.to_string().contains("failed to read prompt"));
    }
}
