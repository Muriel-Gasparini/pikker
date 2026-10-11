use super::model::*;
use crate::{
    domain::*,
    runtime::{DraftSnapshot, IntegrationError},
};
use serde_json::Value;
use std::collections::BTreeSet;
fn invalid() -> IntegrationError {
    IntegrationError::Invalid("Sessão LCU incompatível")
}
fn position(value: &str) -> Option<Role> {
    match value.to_ascii_lowercase().as_str() {
        "top" => Some(Role::Top),
        "jungle" => Some(Role::Jungle),
        "middle" => Some(Role::Mid),
        "bottom" => Some(Role::Bot),
        "utility" => Some(Role::Support),
        _ => None,
    }
}
pub fn normalize_pickable_ids(raw: &Value) -> Result<BTreeSet<ChampionId>, IntegrationError> {
    let ids: Vec<u32> = serde_json::from_value(raw.clone()).map_err(|_| invalid())?;
    if ids.len() > 1024 || ids.contains(&0) {
        return Err(invalid());
    }
    Ok(ids.into_iter().collect())
}
pub fn normalize_session(
    raw: &Value,
    generation: u64,
    revision: u64,
) -> Result<DraftSnapshot, IntegrationError> {
    if generation == 0 || revision == 0 {
        return Err(invalid());
    }
    let mut state = DraftState {
        session_id: generation,
        revision,
        ..Default::default()
    };
    if raw.is_null() {
        return Ok(DraftSnapshot {
            source_epoch: 0,
            generation,
            revision,
            state,
            pickable_ids: None,
            local_role: None,
            local_champion_id: None,
        });
    }
    let mut session: Session = serde_json::from_value(raw.clone()).map_err(|_| invalid())?;
    if session.my_team.is_empty() || session.my_team.len() > 5 || session.their_team.len() > 5 {
        return Err(invalid());
    }
    let mut cells = BTreeSet::new();
    for player in session.my_team.iter().chain(&session.their_team) {
        if player.cell_id < 0 || !cells.insert(player.cell_id) {
            return Err(invalid());
        }
    }
    if !session
        .my_team
        .iter()
        .any(|p| p.cell_id == session.local_player_cell_id)
    {
        return Err(invalid());
    }
    let mut actions = Vec::new();
    for group in session.actions {
        let items = if let Value::Array(items) = group {
            items
        } else {
            vec![group]
        };
        if actions.len() + items.len() > 100 {
            return Err(invalid());
        }
        for item in items {
            let action: Action = serde_json::from_value(item).map_err(|_| invalid())?;
            if !cells.contains(&action.actor_cell_id)
                || !matches!(action.kind.as_str(), "pick" | "ban")
            {
                return Err(invalid());
            }
            actions.push(action);
        }
    }
    state.phase = match session.timer.phase.as_str() {
        "PLANNING" => Phase::Ban,
        "BAN_PICK" => {
            if actions
                .iter()
                .any(|a| a.kind == "ban" && !a.completed && a.is_in_progress)
            {
                Phase::Ban
            } else {
                Phase::Pick
            }
        }
        "FINALIZATION" | "GAME_STARTING" => Phase::Finalization,
        _ => return Err(invalid()),
    };
    let confirmed: BTreeSet<_> = actions
        .iter()
        .filter(|a| a.kind == "pick" && a.completed && a.champion_id > 0)
        .map(|a| a.actor_cell_id)
        .collect();
    let finalization = state.phase == Phase::Finalization;
    let pending: BTreeSet<_> = actions
        .iter()
        .filter(|a| a.kind == "pick" && !a.completed)
        .map(|a| a.actor_cell_id)
        .collect();
    let mut local_role = None;
    let mut local_champion_id = None;
    let mut picked = BTreeSet::new();
    let allied_cells: BTreeSet<_> = session
        .my_team
        .iter()
        .map(|player| player.cell_id)
        .collect();
    session.my_team.sort_by_key(|p| p.cell_id);
    session.their_team.sort_by_key(|p| p.cell_id);
    for (enemy, players) in [(false, session.my_team), (true, session.their_team)] {
        for (slot, player) in players.into_iter().enumerate() {
            let champion = if player.champion_id > 0
                && (confirmed.contains(&player.cell_id) || finalization)
                && !pending.contains(&player.cell_id)
            {
                Some(player.champion_id)
            } else {
                None
            };
            if champion.is_some_and(|id| !picked.insert(id)) {
                return Err(invalid());
            }
            let hover = if !enemy && champion.is_none() {
                let id = if player.champion_pick_intent > 0 {
                    player.champion_pick_intent
                } else {
                    player.champion_id
                };
                (id > 0).then_some(id)
            } else {
                None
            };
            let role = position(&player.assigned_position);
            if !enemy && player.cell_id == session.local_player_cell_id {
                local_role = role;
                local_champion_id = champion;
            }
            let selection = Selection {
                champion,
                hover,
                role,
            };
            if enemy {
                state.enemies[slot] = selection;
            } else {
                state.allies[slot] = selection;
            }
        }
    }
    state.role = local_role.unwrap_or(Role::Support);
    if session.bans.my_team_bans.len() + session.bans.their_team_bans.len() > 20 {
        return Err(invalid());
    }
    for id in session
        .bans
        .my_team_bans
        .into_iter()
        .chain(session.bans.their_team_bans)
    {
        if id < -1 || id > u32::MAX as i64 {
            return Err(invalid());
        }
        if id > 0 {
            state.bans.insert(id as u32);
        }
    }
    state.bans.extend(
        actions
            .into_iter()
            .filter(|a| {
                a.kind == "ban"
                    && a.completed
                    && a.champion_id > 0
                    && allied_cells.contains(&a.actor_cell_id)
            })
            .map(|a| a.champion_id),
    );
    Ok(DraftSnapshot {
        source_epoch: 0,
        generation,
        revision,
        state,
        pickable_ids: None,
        local_role,
        local_champion_id,
    })
}
