use crate::runtime::IntegrationError;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};
pub const RANKS: [&str; 10] = [
    "iron",
    "bronze",
    "silver",
    "gold",
    "platinum",
    "emerald",
    "diamond",
    "master",
    "grandmaster",
    "challenger",
];
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppSettings {
    pub rank: Option<String>,
}
impl AppSettings {
    fn validate(&self) -> Result<(), IntegrationError> {
        if self
            .rank
            .as_deref()
            .is_some_and(|rank| !RANKS.contains(&rank))
        {
            return Err(IntegrationError::Invalid("Elo configurado inválido"));
        }
        Ok(())
    }
}
pub fn load_settings(path: &Path) -> Result<AppSettings, IntegrationError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AppSettings::default()),
        Err(e) => return Err(e.into()),
    };
    if bytes.len() > 4096 {
        return Err(IntegrationError::Invalid(
            "Arquivo de configuração excede limite",
        ));
    }
    let settings: AppSettings = serde_json::from_slice(&bytes)?;
    settings.validate()?;
    Ok(settings)
}
pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), IntegrationError> {
    settings.validate()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec(settings)?)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|e| IntegrationError::Io(e.error))?;
    Ok(())
}
