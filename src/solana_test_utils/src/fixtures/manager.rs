use super::fixture::SolanaTestFixture;
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Manager for loading and saving test fixtures
pub struct FixtureManager {
    base_path: PathBuf,
}

impl FixtureManager {
    /// Create a new fixture manager with the given base path
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Load a fixture from a JSON file
    pub fn load<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        let path = self.fixture_path(name);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read fixture file: {}", path.display()))?;

        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse fixture file: {}", path.display()))
    }

    /// Load a Solana test fixture
    pub fn load_solana_fixture(&self, name: &str) -> Result<SolanaTestFixture> {
        self.load(name)
    }

    /// Save a fixture to a JSON file
    pub fn save<T: Serialize>(&self, name: &str, data: &T) -> Result<()> {
        let path = self.fixture_path(name);

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(data)
            .context("Failed to serialize fixture data")?;

        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write fixture file: {}", path.display()))?;

        Ok(())
    }

    /// Get the full path for a fixture file
    fn fixture_path(&self, name: &str) -> PathBuf {
        let name = if name.ends_with(".json") {
            name.to_string()
        } else {
            format!("{}.json", name)
        };

        self.base_path.join(name)
    }

    /// List all fixture files in the base path
    pub fn list_fixtures(&self) -> Result<Vec<String>> {
        let mut fixtures = Vec::new();

        if !self.base_path.exists() {
            return Ok(fixtures);
        }

        for entry in std::fs::read_dir(&self.base_path)
            .context("Failed to read fixture directory")?
        {
            let entry = entry.context("Failed to read directory entry")?;
            let path = entry.path();

            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    fixtures.push(name.to_string());
                }
            }
        }

        fixtures.sort();
        Ok(fixtures)
    }

    /// Check if a fixture exists
    pub fn exists(&self, name: &str) -> bool {
        self.fixture_path(name).exists()
    }

    /// Get the base path
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_fixture_manager() {
        let temp_dir = TempDir::new().unwrap();
        let manager = FixtureManager::new(temp_dir.path());

        let fixture = SolanaTestFixture::new(
            "Test fixture",
            "11111111111111111111111111111111",
            "SGVsbG8=",
        );

        manager.save("test", &fixture).unwrap();
        assert!(manager.exists("test"));

        let loaded: SolanaTestFixture = manager.load("test").unwrap();
        assert_eq!(loaded.description, "Test fixture");

        let fixtures = manager.list_fixtures().unwrap();
        assert_eq!(fixtures, vec!["test"]);
    }
}
