use crate::meta::model::{ChampionDetails, MetaSnapshot};
use crate::{catalog::Catalog, domain::*, ranking::*, simulator::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceMode {
    Simulator,
    Connected,
}
pub struct Controller {
    pub catalog: Catalog,
    pub draft: DraftState,
    pub profile: PlayerProfile,
    pub weights: Weights,
    pub rows: Vec<RankedCandidate>,
    pub computations: u64,
    pub source_mode: SourceMode,
    pub meta: Option<MetaSnapshot>,
    requested_rank: Option<String>,
    pending_meta: Option<MetaSnapshot>,
    curated_catalog: Catalog,
    pickable_ids: Option<std::collections::BTreeSet<ChampionId>>,
    lcu_cursor: Option<(u64, u64)>,
    pub local_champion_id: Option<ChampionId>,
}
impl Controller {
    pub fn new(catalog: Catalog, profile: PlayerProfile) -> Self {
        Self {
            curated_catalog: catalog.clone(),
            catalog,
            draft: DraftState::default(),
            profile,
            weights: Weights::default(),
            rows: vec![],
            computations: 0,
            source_mode: SourceMode::Simulator,
            meta: None,
            requested_rank: None,
            pending_meta: None,
            pickable_ids: None,
            lcu_cursor: None,
            local_champion_id: None,
        }
    }
    pub fn dispatch(&mut self, command: DraftCommand) -> Result<(), String> {
        if command.session_id != self.draft.session_id || command.revision <= self.draft.revision {
            return Ok(());
        }
        match &command.action {
            Action::ConfirmPick { id: Some(id), .. }
            | Action::SetHover { id: Some(id), .. }
            | Action::SetBan { id, .. }
                if !self.catalog.contains_key(id) =>
            {
                return Err("Campeão desconhecido".into());
            }
            _ => {}
        }
        let change = apply(&mut self.draft, command)?;
        if matches!(self.draft.phase, Phase::Waiting | Phase::Ended)
            && let Some(snapshot) = self.pending_meta.take()
        {
            self.install_meta(snapshot);
        }
        if change == ChangeKind::Recommendation {
            self.recompute()?;
        }
        Ok(())
    }
    pub fn set_source_mode(&mut self, mode: SourceMode) -> Result<(), String> {
        if self.source_mode == mode {
            return Ok(());
        }
        self.source_mode = mode;
        self.pickable_ids = None;
        self.local_champion_id = None;
        self.draft = DraftState {
            session_id: self.draft.session_id + 1,
            role: self.draft.role,
            ..Default::default()
        };
        if let Some(snapshot) = self.pending_meta.take().or_else(|| self.meta.clone()) {
            self.install_meta(snapshot);
        } else {
            self.catalog = if mode == SourceMode::Simulator {
                self.curated_catalog.clone()
            } else {
                Catalog::new()
            };
        }
        self.recompute()
    }
    fn install_meta(&mut self, snapshot: MetaSnapshot) {
        let mut catalog = Catalog::new();
        for identity in &snapshot.catalog {
            let mut champion =
                self.curated_catalog
                    .get(&identity.id)
                    .cloned()
                    .unwrap_or(Champion {
                        id: identity.id,
                        name: identity.display_name.clone(),
                        difficulty: 0.0,
                        curated: false,
                        roles: Default::default(),
                        traits: Default::default(),
                    });
            champion.name = identity.display_name.clone();
            champion.roles = Role::ALL
                .into_iter()
                .map(|role| (role, RoleEvidence::Insufficient))
                .collect();
            catalog.insert(champion.id, champion);
        }
        for row in &snapshot.stats {
            let patch = row.scope.patch.as_deref().unwrap_or("não informado");
            let region = row.scope.region.as_deref().unwrap_or("não informada");
            let rank = row
                .scope
                .verified_rank
                .as_deref()
                .unwrap_or("não comprovado");
            if let Some(champion) = catalog.get_mut(&row.champion_id) {
                champion.roles.insert(
                    row.role,
                    RoleEvidence::Statistical {
                        win_rate: row.observed_rate(),
                        games: row.games,
                        provenance: format!(
                            "{} · patch {patch} · região {region} · elo {rank}",
                            snapshot.key.source
                        ),
                    },
                );
            }
        }
        self.catalog = catalog;
        self.meta = Some(snapshot);
    }
    pub fn set_meta(&mut self, snapshot: MetaSnapshot) -> Result<(), String> {
        if self
            .meta
            .as_ref()
            .is_some_and(|old| snapshot.fetched_at < old.fetched_at)
        {
            return Ok(());
        }
        snapshot.validate().map_err(|e| e.to_string())?;
        if matches!(
            self.draft.phase,
            Phase::Ban | Phase::Pick | Phase::Finalization
        ) {
            if self
                .pending_meta
                .as_ref()
                .is_none_or(|old| snapshot.fetched_at >= old.fetched_at)
            {
                self.pending_meta = Some(snapshot);
            }
            return Ok(());
        }
        self.install_meta(snapshot);
        self.recompute()
    }
    pub fn set_requested_rank(&mut self, rank: Option<String>) {
        self.requested_rank = rank;
    }
    pub fn selected_details(&self, id: ChampionId, role: Role) -> Option<&ChampionDetails> {
        let meta = self.meta.as_ref()?;
        meta.details.iter().find(|d| {
            d.champion_id == id && d.role == role && d.requested_rank == self.requested_rank
        })
    }
    pub fn apply_snapshot(
        &mut self,
        mut snapshot: crate::runtime::DraftSnapshot,
    ) -> Result<(), String> {
        if self.source_mode != SourceMode::Connected {
            return Ok(());
        }
        if self
            .lcu_cursor
            .is_some_and(|cursor| (snapshot.generation, snapshot.revision) <= cursor)
        {
            return Ok(());
        }
        if snapshot.generation == 0
            || snapshot.revision == 0
            || snapshot.state.session_id != snapshot.generation
            || snapshot.state.revision != snapshot.revision
            || snapshot.state.allies.len() != 5
            || snapshot.state.enemies.len() != 5
        {
            return Err("Snapshot LCU inválido".into());
        }
        let mut picked = std::collections::BTreeSet::new();
        for player in snapshot.state.allies.iter().chain(&snapshot.state.enemies) {
            if player
                .champion
                .is_some_and(|id| id == 0 || !picked.insert(id))
                || player.hover == Some(0)
            {
                return Err("Snapshot LCU inválido".into());
            }
        }
        if snapshot.state.bans.contains(&0)
            || snapshot.state.enemies.iter().any(|p| p.hover.is_some())
            || snapshot
                .pickable_ids
                .as_ref()
                .is_some_and(|ids| ids.contains(&0))
        {
            return Err("Snapshot LCU inválido".into());
        }
        if let Some(role) = snapshot.local_role {
            snapshot.state.role = role;
        } else {
            snapshot.state.role = self.draft.role;
        }
        let comparison = |state: &DraftState| {
            let mut value = state.clone();
            value.revision = 0;
            for player in value.allies.iter_mut().chain(&mut value.enemies) {
                player.hover = None;
            }
            value
        };
        let mut changed = comparison(&snapshot.state) != comparison(&self.draft)
            || self.pickable_ids != snapshot.pickable_ids;
        self.lcu_cursor = Some((snapshot.generation, snapshot.revision));
        self.draft = snapshot.state;
        self.pickable_ids = snapshot.pickable_ids;
        self.local_champion_id = snapshot.local_champion_id;
        if matches!(self.draft.phase, Phase::Waiting | Phase::Ended)
            && let Some(meta) = self.pending_meta.take()
        {
            self.install_meta(meta);
            changed = true;
        }
        if changed {
            self.recompute()?;
        }
        Ok(())
    }
    pub fn recompute(&mut self) -> Result<(), String> {
        if matches!(self.draft.phase, Phase::Waiting | Phase::Ended) {
            self.rows.clear();
            return Ok(());
        }
        self.rows = rank_with_availability(
            &self.catalog,
            &self.draft,
            &self.profile,
            &self.weights,
            self.pickable_ids.as_ref(),
        )?;
        self.computations += 1;
        Ok(())
    }
    pub fn set_preference(&mut self, id: ChampionId, preference: Preference) -> Result<(), String> {
        if !self.catalog.contains_key(&id) || preference.proficiency.is_some_and(|p| !unit(p)) {
            return Err("Preferência inválida".into());
        }
        let key = (id, self.draft.role);
        if self.profile.preferences.get(&key) == Some(&preference) {
            return Ok(());
        }
        self.profile.preferences.insert(key, preference);
        self.recompute()
    }
    pub fn set_weights(&mut self, weights: Weights) -> Result<(), String> {
        weights.validate()?;
        self.weights = weights;
        self.recompute()
    }
}
