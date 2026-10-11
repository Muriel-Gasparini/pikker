use crate::{catalog::Catalog, domain::*};
#[derive(Clone, Debug)]
pub struct Weights {
    pub viability: f32,
    pub proficiency: f32,
    pub composition: f32,
    pub blind: f32,
    pub preference: f32,
}
impl Default for Weights {
    fn default() -> Self {
        Self {
            viability: 0.40,
            proficiency: 0.25,
            composition: 0.20,
            blind: 0.10,
            preference: 0.05,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Contribution {
    pub key: &'static str,
    pub value: f32,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub struct RankedCandidate {
    pub id: ChampionId,
    pub score: Option<f32>,
    pub contributions: Vec<Contribution>,
    pub favorite: bool,
    pub insufficient: bool,
    pub learning_notice: bool,
    pub statistical: bool,
    pub games: Option<u64>,
    pub provenance: Option<String>,
    pub coverage_limited: bool,
}
impl Weights {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.viability,
            self.proficiency,
            self.composition,
            self.blind,
            self.preference,
        ];
        if !values.into_iter().all(unit) || (values.iter().sum::<f32>() - 1.0).abs() > 0.001 {
            return Err("Pesos devem estar entre 0 e 1 e somar 100%".into());
        }
        Ok(())
    }
}
pub fn rank(
    catalog: &Catalog,
    draft: &DraftState,
    profile: &PlayerProfile,
    weights: &Weights,
) -> Result<Vec<RankedCandidate>, String> {
    rank_with_availability(catalog, draft, profile, weights, None)
}
pub fn rank_with_availability(
    catalog: &Catalog,
    draft: &DraftState,
    profile: &PlayerProfile,
    weights: &Weights,
    pickable: Option<&std::collections::BTreeSet<ChampionId>>,
) -> Result<Vec<RankedCandidate>, String> {
    weights.validate()?;
    let unavailable: std::collections::BTreeSet<_> = draft
        .bans
        .iter()
        .copied()
        .chain(
            draft
                .allies
                .iter()
                .chain(&draft.enemies)
                .filter_map(|s| s.champion),
        )
        .collect();
    let ally_traits: Vec<_> = draft
        .allies
        .iter()
        .filter_map(|s| s.champion.and_then(|id| catalog.get(&id)))
        .map(|c| &c.traits)
        .collect();
    let enemy_traits: Vec<_> = draft
        .enemies
        .iter()
        .filter_map(|s| s.champion.and_then(|id| catalog.get(&id)))
        .map(|c| &c.traits)
        .collect();
    let enemy_engage = enemy_traits
        .iter()
        .map(|t| t.engage)
        .fold(0.0_f32, f32::max);
    let ally_front = ally_traits
        .iter()
        .map(|t| t.frontline)
        .fold(0.0_f32, f32::max);
    let ally_engage = ally_traits.iter().map(|t| t.engage).fold(0.0_f32, f32::max);
    let blind = !draft
        .enemies
        .iter()
        .any(|s| s.role == Some(draft.role) && s.champion.is_some());
    let eligible: Vec<_> = catalog
        .values()
        .filter(|c| pickable.is_none_or(|ids| ids.contains(&c.id)))
        .filter(|c| {
            !unavailable.contains(&c.id)
                && c.roles.get(&draft.role).is_some_and(|r| {
                    matches!(
                        r,
                        RoleEvidence::Demonstrative { .. } | RoleEvidence::Statistical { .. }
                    )
                })
        })
        .collect();
    let difficulty_covered = eligible.iter().all(|c| c.curated);
    let traits_covered = difficulty_covered
        && draft
            .allies
            .iter()
            .chain(&draft.enemies)
            .filter_map(|s| s.champion)
            .all(|id| catalog.get(&id).is_some_and(|c| c.curated));
    let active_weight = weights.viability
        + weights.proficiency
        + weights.preference
        + if traits_covered {
            weights.composition + weights.blind
        } else {
            0.0
        };
    let mut rows = Vec::new();
    for c in catalog.values() {
        if pickable.is_some_and(|ids| !ids.contains(&c.id)) {
            continue;
        }
        if unavailable.contains(&c.id) {
            continue;
        }
        let Some(evidence) = c.roles.get(&draft.role) else {
            continue;
        };
        if matches!(evidence, RoleEvidence::Unsupported) {
            continue;
        }
        let pref = profile
            .preferences
            .get(&(c.id, draft.role))
            .cloned()
            .unwrap_or_default();
        if pref.proficiency.is_some_and(|p| !unit(p)) {
            return Err("Domínio inválido".into());
        }
        let learning = pref.proficiency.is_none() || pref.proficiency.is_some_and(|p| p < 0.4);
        let mut row = RankedCandidate {
            id: c.id,
            score: None,
            contributions: vec![],
            favorite: pref.favorite,
            insufficient: matches!(evidence, RoleEvidence::Insufficient),
            learning_notice: learning,
            statistical: matches!(evidence, RoleEvidence::Statistical { .. }),
            games: if let RoleEvidence::Statistical { games, .. } = evidence {
                *games
            } else {
                None
            },
            provenance: if let RoleEvidence::Statistical { provenance, .. } = evidence {
                Some(provenance.clone())
            } else {
                None
            },
            coverage_limited: !traits_covered || !difficulty_covered,
        };
        let viability = match evidence {
            RoleEvidence::Demonstrative { viability } => Some(*viability),
            RoleEvidence::Statistical { win_rate, .. } => Some(*win_rate),
            _ => None,
        };
        if let Some(viability) = viability.filter(|_| active_weight > 0.0) {
            let t = &c.traits;
            let needs_front = if ally_traits.is_empty() {
                0.5
            } else {
                1.0 - ally_front
            };
            let needs_engage = if ally_traits.is_empty() {
                0.5
            } else {
                1.0 - ally_engage
            };
            let composition = match draft.role {
                Role::Support => {
                    (t.peel * enemy_engage
                        + t.protection
                        + t.engage * needs_engage
                        + t.frontline * needs_front)
                        / 4.0
                }
                Role::Top | Role::Jungle => {
                    (t.frontline * needs_front + t.engage * needs_engage + t.peel * enemy_engage)
                        / 3.0
                }
                Role::Mid => (t.magic + t.range + t.peel * enemy_engage) / 3.0,
                Role::Bot => {
                    (t.range + t.poke * (ally_engage.max(0.5)) + t.blind * enemy_engage) / 3.0
                }
            };
            let proficiency = pref.proficiency.unwrap_or(0.5);
            // Unknown experience is penalized for difficult champions, without claiming incompetence.
            let execution = if difficulty_covered {
                proficiency * (1.0 - c.difficulty * (1.0 - proficiency) * 0.5)
            } else {
                proficiency
            };
            let factors = [
                (
                    "viability",
                    weights.viability * viability,
                    if row.statistical {
                        "Taxa observada na rota (não é chance pessoal de vitória)"
                    } else {
                        "Viabilidade demonstrativa nesta função"
                    },
                ),
                (
                    "proficiency",
                    weights.proficiency * execution,
                    if !difficulty_covered {
                        "Domínio informado ou neutro; mecânica não avaliada"
                    } else if pref.proficiency.is_none() {
                        "Domínio desconhecido; dificuldade considerada"
                    } else {
                        "Domínio informado e dificuldade"
                    },
                ),
                (
                    "composition",
                    weights.composition * composition,
                    "Encaixe com características conhecidas do draft",
                ),
                (
                    "blind",
                    weights.blind * if blind { t.blind } else { 0.5 },
                    if blind {
                        "Adversário de rota desconhecido: segurança de blind pick"
                    } else {
                        "Adversário de rota conhecido; sem estatística de matchup"
                    },
                ),
                (
                    "preference",
                    weights.preference * if pref.favorite { 1.0 } else { 0.0 },
                    "Preferência pessoal (peso secundário)",
                ),
            ];
            row.contributions = factors
                .into_iter()
                .filter(|(key, _, _)| traits_covered || !matches!(*key, "composition" | "blind"))
                .map(|(key, value, reason)| Contribution {
                    key,
                    value: value / active_weight,
                    reason: reason.into(),
                })
                .collect();
            row.score = Some(row.contributions.iter().map(|c| c.value).sum());
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| {
        a.insufficient
            .cmp(&b.insufficient)
            .then_with(|| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)))
            .then(a.id.cmp(&b.id))
    });
    Ok(rows)
}
