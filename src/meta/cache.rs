use super::model::*;
use crate::runtime::IntegrationError;
use std::{
    io::{Read, Write},
    path::Path,
};

pub fn load_cache(
    path: &Path,
    key: &MetaKey,
    now: i64,
) -> Result<Option<CachedMeta>, IntegrationError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err(IntegrationError::Invalid("Cache excede limite de tamanho"));
    }
    let snapshot: MetaSnapshot = serde_json::from_slice(&bytes)?;
    snapshot.validate()?;
    let age = now
        .checked_sub(snapshot.fetched_at)
        .ok_or(IntegrationError::Invalid("Timestamp inválido"))?;
    if age < -600 {
        return Err(IntegrationError::Invalid("Cache com timestamp futuro"));
    }
    if &snapshot.key != key {
        return Ok(None);
    }
    Ok(Some(CachedMeta {
        snapshot,
        stale: age >= CACHE_TTL_SECONDS,
    }))
}

pub fn save_cache(path: &Path, snapshot: &MetaSnapshot) -> Result<(), IntegrationError> {
    snapshot.validate()?;
    let bytes = serde_json::to_vec(snapshot)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err(IntegrationError::Invalid("Cache excede limite de tamanho"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|e| IntegrationError::Io(e.error))?;
    Ok(())
}
