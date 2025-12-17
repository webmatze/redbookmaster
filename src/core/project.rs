use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::Album;

/// Project file extension
pub const PROJECT_EXTENSION: &str = "rbm";

/// Current project file version
pub const PROJECT_VERSION: u32 = 1;

/// A Red Book Master project file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Project file version (for future compatibility)
    pub version: u32,

    /// The album data
    pub album: Album,

    /// Path to the project file (not serialized)
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

impl Project {
    /// Create a new project with the given album
    pub fn new(album: Album) -> Self {
        Self {
            version: PROJECT_VERSION,
            album,
            file_path: None,
        }
    }

    /// Create a new empty project
    pub fn empty() -> Self {
        Self::new(Album::default())
    }

    /// Load a project from a file
    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        let content = fs::read_to_string(path)
            .map_err(|e| ProjectError::ReadError(path.to_path_buf(), e))?;

        let mut project: Project = serde_json::from_str(&content)
            .map_err(|e| ProjectError::ParseError(path.to_path_buf(), e))?;

        project.file_path = Some(path.to_path_buf());

        // Handle version migrations if needed
        if project.version > PROJECT_VERSION {
            return Err(ProjectError::VersionTooNew {
                file_version: project.version,
                supported_version: PROJECT_VERSION,
            });
        }

        Ok(project)
    }

    /// Save the project to its current file path
    pub fn save(&self) -> Result<(), ProjectError> {
        match &self.file_path {
            Some(path) => self.save_to(path),
            None => Err(ProjectError::NoFilePath),
        }
    }

    /// Save the project to a specific path
    pub fn save_to(&self, path: &Path) -> Result<(), ProjectError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| ProjectError::SerializeError(e))?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| ProjectError::WriteError(path.to_path_buf(), e))?;
            }
        }

        fs::write(path, content)
            .map_err(|e| ProjectError::WriteError(path.to_path_buf(), e))?;

        Ok(())
    }

    /// Save and update the file path
    pub fn save_as(&mut self, path: &Path) -> Result<(), ProjectError> {
        self.save_to(path)?;
        self.file_path = Some(path.to_path_buf());
        Ok(())
    }

    /// Generate a default filename from the album title
    pub fn default_filename(&self) -> String {
        let safe_name: String = self
            .album
            .title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        let safe_name = safe_name.trim().replace(' ', "-").to_lowercase();
        format!("{}.{}", safe_name, PROJECT_EXTENSION)
    }

    /// Check if the project has unsaved changes
    /// (This is a placeholder - in a real implementation you'd track changes)
    pub fn is_modified(&self) -> bool {
        // TODO: Implement change tracking
        false
    }

    /// Get the project name (filename without extension, or album title)
    pub fn name(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.album.title.clone())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("Failed to read project file '{0}': {1}")]
    ReadError(PathBuf, std::io::Error),

    #[error("Failed to parse project file '{0}': {1}")]
    ParseError(PathBuf, serde_json::Error),

    #[error("Failed to write project file '{0}': {1}")]
    WriteError(PathBuf, std::io::Error),

    #[error("Failed to serialize project: {0}")]
    SerializeError(serde_json::Error),

    #[error("Project file version {file_version} is newer than supported version {supported_version}")]
    VersionTooNew {
        file_version: u32,
        supported_version: u32,
    },

    #[error("No file path set. Use save_as() to specify a path.")]
    NoFilePath,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_filename() {
        let album = Album::new("My Awesome Album!".to_string(), "Artist".to_string());
        let project = Project::new(album);
        assert_eq!(project.default_filename(), "my-awesome-album_.rbm");
    }

    #[test]
    fn test_project_serialization() {
        let album = Album::new("Test Album".to_string(), "Test Artist".to_string());
        let project = Project::new(album);

        let json = serde_json::to_string(&project).unwrap();
        let loaded: Project = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.album.title, "Test Album");
        assert_eq!(loaded.version, PROJECT_VERSION);
    }
}
