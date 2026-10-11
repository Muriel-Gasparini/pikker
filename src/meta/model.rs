use crate::{
    domain::{ChampionId, Role, unit},
    runtime::IntegrationError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024;
pub const CACHE_TTL_SECONDS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaKey {
    pub source: String,
    pub schema_version: u32,
    pub requested_rank: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceScope {
    pub patch: Option<String>,
    pub region: Option<String>,
    pub verified_rank: Option<String>,
    pub queue: Option<u32>,
    pub provider_timestamp: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChampionIdentity {
    pub id: ChampionId,
    pub api_name: String,
    pub display_name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoleStats {
    pub champion_id: ChampionId,
    pub role: Role,
    pub games: Option<u64>,
    pub wins: Option<u64>,
    pub win_rate: f32,
    pub scope: SourceScope,
}
impl RoleStats {
    pub fn observed_rate(&self) -> f32 {
        match (self.games, self.wins) {
            (Some(games), Some(wins)) if games > 0 => wins as f32 / games as f32,
            _ => self.win_rate,
        }
    }
    pub fn validate(&self) -> Result<(), IntegrationError> {
        if self.champion_id == 0 || !unit(self.win_rate) {
            return Err(IntegrationError::Invalid("Estatística inválida"));
        }
        match (self.games, self.wins) {
            (Some(games), Some(wins)) if games > 0 && wins <= games => Ok(()),
            (None, None) => Ok(()),
            _ => Err(IntegrationError::Invalid(
                "Contagens estatísticas inválidas",
            )),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildChoice {
    pub ids: Vec<u32>,
    pub names: Vec<String>,
    pub games: Option<u64>,
    pub wins: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Matchup {
    pub champion_id: ChampionId,
    #[serde(default)]
    pub role: Option<Role>,
    pub games: Option<u64>,
    pub wins: Option<u64>,
    pub win_rate: Option<f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChampionDetails {
    pub champion_id: ChampionId,
    pub role: Role,
    pub requested_rank: Option<String>,
    pub scope: SourceScope,
    pub runes: Vec<BuildChoice>,
    pub spells: Vec<BuildChoice>,
    pub counters: Vec<Matchup>,
    pub synergies: Vec<Matchup>,
    pub fetched_at: i64,
}
fn valid_counts(games: Option<u64>, wins: Option<u64>) -> bool {
    matches!((games, wins), (None, None))
        || matches!((games,wins), (Some(g),Some(w)) if g > 0 && w <= g)
}
impl ChampionDetails {
    pub fn validate(&self) -> Result<(), IntegrationError> {
        if self.champion_id == 0
            || self.fetched_at < 0
            || self.runes.iter().chain(&self.spells).any(|b| {
                b.ids.is_empty()
                    || b.ids.contains(&0)
                    || !valid_counts(b.games, b.wins)
                    || (!b.names.is_empty() && b.names.len() != b.ids.len())
            })
            || self.counters.iter().chain(&self.synergies).any(|m| {
                m.champion_id == 0
                    || !valid_counts(m.games, m.wins)
                    || m.win_rate.is_some_and(|r| !unit(r))
            })
        {
            return Err(IntegrationError::Invalid("Detalhes estatísticos inválidos"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaSnapshot {
    pub schema_version: u32,
    pub key: MetaKey,
    pub fetched_at: i64,
    pub catalog: Vec<ChampionIdentity>,
    #[serde(default)]
    pub labels: StaticLabels,
    pub stats: Vec<RoleStats>,
    pub details: Vec<ChampionDetails>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticLabels {
    pub catalog_version: Option<String>,
    pub spells: std::collections::BTreeMap<u32, String>,
    pub runes: std::collections::BTreeMap<u32, String>,
}
impl MetaSnapshot {
    pub fn validate(&self) -> Result<(), IntegrationError> {
        if self.schema_version != SCHEMA_VERSION
            || self.key.schema_version != SCHEMA_VERSION
            || self.key.source.trim().is_empty()
            || self.fetched_at < 0
            || self.catalog.is_empty()
            || self.stats.is_empty()
        {
            return Err(IntegrationError::Invalid(
                "Versão ou conteúdo de cache inválido",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut api_names = BTreeSet::new();
        for champion in &self.catalog {
            if champion.id == 0
                || champion.api_name.trim().is_empty()
                || champion.display_name.trim().is_empty()
                || !ids.insert(champion.id)
                || !api_names.insert(&champion.api_name)
            {
                return Err(IntegrationError::Invalid("Catálogo de cache inválido"));
            }
        }
        let mut pairs = BTreeSet::new();
        for row in &self.stats {
            row.validate()?;
            if !ids.contains(&row.champion_id) || !pairs.insert((row.champion_id, row.role)) {
                return Err(IntegrationError::Invalid(
                    "Campeão/rota duplicado ou desconhecido",
                ));
            }
        }
        let mut details = BTreeSet::new();
        for row in &self.details {
            row.validate()?;
            if !ids.contains(&row.champion_id)
                || !details.insert((row.champion_id, row.role, &row.requested_rank))
            {
                return Err(IntegrationError::Invalid(
                    "Detalhes duplicados ou desconhecidos",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct CachedMeta {
    pub snapshot: MetaSnapshot,
    pub stale: bool,
}
