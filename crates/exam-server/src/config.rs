use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub bind: String,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = std::env::var("EXAM_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data"));
        Self {
            data_dir,
            bind: std::env::var("EXAM_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into()),
        }
    }

    pub fn from_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            bind: "127.0.0.1:0".into(),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("exam.db")
    }

    pub fn ensure_data_dir(&self) -> std::io::Result<&Path> {
        std::fs::create_dir_all(&self.data_dir)?;
        Ok(self.data_dir.as_path())
    }
}
