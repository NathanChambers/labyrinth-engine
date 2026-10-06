use std::path::{Path, PathBuf};

use ab_glyph::FontArc;

#[derive(Clone, Debug)]
pub struct FontAsset {
    font: FontArc,
    path: PathBuf,
}

impl FontAsset {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|error| format!("failed to read font asset {}: {error}", path.display()))?;
        let font = FontArc::try_from_vec(bytes).map_err(|_| format!("failed to parse font asset {}", path.display()))?;
        Ok(Self { font, path })
    }

    pub fn from_working_directory(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = std::env::current_dir().map_err(|error| format!("failed to resolve working directory: {error}"))?.join(path);
        Self::from_file(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn font(&self) -> &FontArc {
        &self.font
    }
}
