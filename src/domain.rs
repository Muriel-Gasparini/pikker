use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type ChampionId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Top,
    Jungle,
    Mid,
    Bot,
    Support,
}
impl Role {
    pub const ALL: [Self; 5] = [Self::Top, Self::Jungle, Self::Mid, Self::Bot, Self::Support];
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "TOP",
            Self::Jungle => "JUNGLE",
            Self::Mid => "MID",
            Self::Bot => "BOT",
            Self::Support => "SUPPORT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Waiting,
    Ban,
    Pick,
    Finalization,
    Ended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RoleEvidence {
    Unsupported,
    Insufficient,
    Demonstrative {
        viability: f32,
    },
    Statistical {
        win_rate: f32,
        games: Option<u64>,
        provenance: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Traits {
    pub engage: f32,
    pub peel: f32,
    pub protection: f32,
    pub poke: f32,
    pub blind: f32,
    pub range: f32,
    pub magic: f32,
    pub frontline: f32,
}
impl Traits {
    pub fn values(&self) -> [f32; 8] {
        [
            self.engage,
            self.peel,
            self.protection,
            self.poke,
            self.blind,
            self.range,
            self.magic,
            self.frontline,
        ]
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Champion {
    pub id: ChampionId,
    pub name: String,
    pub difficulty: f32,
    #[serde(default = "curated_default")]
    pub curated: bool,
    pub roles: BTreeMap<Role, RoleEvidence>,
    #[serde(default)]
    pub traits: Traits,
}
fn curated_default() -> bool {
    true
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    pub favorite: bool,
    pub proficiency: Option<f32>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerProfile {
    pub preferences: BTreeMap<(ChampionId, Role), Preference>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub champion: Option<ChampionId>,
    pub hover: Option<ChampionId>,
    pub role: Option<Role>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftState {
    pub session_id: u64,
    pub revision: u64,
    pub role: Role,
    pub phase: Phase,
    pub allies: Vec<Selection>,
    pub enemies: Vec<Selection>,
    pub bans: BTreeSet<ChampionId>,
}
impl Default for DraftState {
    fn default() -> Self {
        Self {
            session_id: 1,
            revision: 0,
            role: Role::Support,
            phase: Phase::Waiting,
            allies: vec![Selection::default(); 5],
            enemies: vec![Selection::default(); 5],
            bans: BTreeSet::new(),
        }
    }
}
pub fn unit(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
