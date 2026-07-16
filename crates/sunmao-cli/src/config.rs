use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub base_url: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("config dir")?
        .join("sunmao");
    Ok(dir.join("config.toml"))
}

pub fn load_global() -> Result<GlobalConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }
    let s = fs::read_to_string(&path)?;
    Ok(toml::from_str(&s)?)
}

pub fn save_global(cfg: &GlobalConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(cfg)?)?;
    Ok(())
}
