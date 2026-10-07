//! Runtime directories. State is SQLite; workspaces and inboxes are user-owned.
use std::path::PathBuf;
#[derive(Debug, Clone)]
pub struct DataLayout {
    root: PathBuf,
}
impl DataLayout {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: data_dir.into(),
        }
    }
    pub fn state_db(&self) -> PathBuf {
        self.root.join("state.db")
    }
    pub fn user_root(&self, user: &str) -> PathBuf {
        self.root
            .parent()
            .unwrap_or(&self.root)
            .join("users")
            .join(format!("{:x}", md5::compute(user)))
    }
    pub fn workspace(&self, user: &str) -> PathBuf {
        self.user_root(user).join("workspace")
    }
    pub fn inbox(&self, user: &str) -> PathBuf {
        self.user_root(user).join("inbox")
    }
}
