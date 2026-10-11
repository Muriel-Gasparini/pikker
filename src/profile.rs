use crate::domain::*;
use serde::{Deserialize, Serialize};
use std::io::Write;
pub fn proficiency_level(p: &Preference) -> usize {
    p.proficiency
        .map(|value| (value * 5.0).round() as usize + 1)
        .unwrap_or(0)
}
pub fn edit_preference(
    p: &Preference,
    favorite: bool,
    level: usize,
    changed: bool,
) -> Result<Preference, String> {
    if level > 6 {
        return Err("Nível de domínio inválido".into());
    }
    let proficiency = if changed {
        if level == 0 {
            None
        } else {
            Some((level - 1) as f32 / 5.0)
        }
    } else {
        p.proficiency
    };
    Ok(Preference {
        favorite,
        proficiency,
    })
}
use std::path::{Path, PathBuf};
#[derive(Serialize, Deserialize)]
struct Record {
    id: ChampionId,
    role: Role,
    favorite: bool,
    proficiency: Option<f32>,
}
pub fn load_profile(path: &Path) -> Result<PlayerProfile, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PlayerProfile::default()),
        Err(e) => return Err(e.to_string()),
    };
    let rows: Vec<Record> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut p = PlayerProfile::default();
    for row in rows {
        if row.id == 0 || row.proficiency.is_some_and(|p| !unit(p)) {
            return Err("Preferência inválida".into());
        }
        p.preferences.insert(
            (row.id, row.role),
            Preference {
                favorite: row.favorite,
                proficiency: row.proficiency,
            },
        );
    }
    Ok(p)
}
pub fn save_profile(path: &Path, profile: &PlayerProfile) -> Result<(), String> {
    let mut rows = Vec::new();
    for (&(id, role), p) in &profile.preferences {
        if id == 0 || p.proficiency.is_some_and(|p| !unit(p)) {
            return Err("Preferência inválida".into());
        }
        rows.push(Record {
            id,
            role,
            favorite: p.favorite,
            proficiency: p.proficiency,
        });
    }
    let bytes = serde_json::to_vec_pretty(&rows).map_err(|e| e.to_string())?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn profile_path() -> PathBuf {
    if let Some(path) = std::env::var_os("PIKKER_DATA_DIR") {
        return PathBuf::from(path).join("profile.json");
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(path).join("Pikker/profile.json");
    }
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(path).join("pikker/profile.json");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path).join(".local/share/pikker/profile.json");
    }
    PathBuf::from("pikker-data/profile.json")
}
