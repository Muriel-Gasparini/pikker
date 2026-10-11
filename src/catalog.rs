use crate::domain::*;
use std::collections::BTreeMap;

pub type Catalog = BTreeMap<ChampionId, Champion>;
pub fn load_catalog(text: &str) -> Result<Catalog, String> {
    let rows: Vec<Champion> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Err("Catálogo vazio".into());
    }
    let mut catalog = Catalog::new();
    for row in rows {
        if row.id == 0
            || row.name.trim().is_empty()
            || row.roles.is_empty()
            || !unit(row.difficulty)
            || !row.traits.values().into_iter().all(unit)
            || row.roles.values().any(
                |e| matches!(e, RoleEvidence::Demonstrative { viability } if !unit(*viability))
                    || matches!(e,RoleEvidence::Statistical {win_rate,games,..} if !unit(*win_rate)||*games==Some(0)),
            )
        {
            return Err("Campeão ou avaliação inválida".into());
        }
        if catalog.insert(row.id, row).is_some() {
            return Err("ID duplicado".into());
        }
    }
    Ok(catalog)
}
pub fn validate_catalog(text: &str) -> Result<(), String> {
    load_catalog(text).map(|_| ())
}
pub fn demo_catalog() -> Catalog {
    load_catalog(include_str!("../assets/demo-catalog.json"))
        .expect("embedded catalog validated by tests")
}
