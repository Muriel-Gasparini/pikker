use crate::domain::*;
#[derive(Clone, Debug)]
pub enum Action {
    NewSession,
    SetRole(Role),
    SetPhase(Phase),
    SetHover {
        enemy: bool,
        slot: usize,
        id: Option<ChampionId>,
    },
    ConfirmPick {
        enemy: bool,
        slot: usize,
        id: Option<ChampionId>,
        role: Option<Role>,
    },
    SetBan {
        id: ChampionId,
        banned: bool,
    },
    LoadPreset(usize),
}
#[derive(Clone, Debug)]
pub struct DraftCommand {
    pub session_id: u64,
    pub revision: u64,
    pub action: Action,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChangeKind {
    None,
    Presentation,
    Recommendation,
}
pub fn apply(draft: &mut DraftState, command: DraftCommand) -> Result<ChangeKind, String> {
    if command.session_id != draft.session_id || command.revision <= draft.revision {
        return Ok(ChangeKind::None);
    }
    let mut next = draft.clone();
    let mut kind = ChangeKind::Recommendation;
    match command.action {
        Action::NewSession => {
            next = DraftState {
                session_id: draft.session_id.checked_add(1).ok_or("Sessão esgotada")?,
                role: draft.role,
                ..Default::default()
            };
        }
        Action::SetRole(role) => next.role = role,
        Action::SetPhase(phase) => next.phase = phase,
        Action::SetHover { enemy, slot, id } => {
            let team = if enemy {
                &mut next.enemies
            } else {
                &mut next.allies
            };
            team.get_mut(slot).ok_or("Posição inválida")?.hover = id;
            kind = ChangeKind::Presentation;
        }
        Action::ConfirmPick {
            enemy,
            slot,
            id,
            role,
        } => {
            let team = if enemy {
                &mut next.enemies
            } else {
                &mut next.allies
            };
            let selection = team.get_mut(slot).ok_or("Posição inválida")?;
            selection.champion = id;
            selection.hover = None;
            selection.role = role;
        }
        Action::SetBan { id, banned } => {
            if id == 0 {
                return Err("ID inválido".into());
            }
            if banned {
                next.bans.insert(id);
            } else {
                next.bans.remove(&id);
            }
        }
        Action::LoadPreset(index) => {
            next = preset(draft.role, index);
            next.session_id = draft.session_id.checked_add(1).ok_or("Sessão esgotada")?;
        }
    }
    let mut ids = std::collections::BTreeSet::new();
    for s in next.allies.iter().chain(&next.enemies) {
        if let Some(id) = s.champion
            && (id == 0 || next.bans.contains(&id) || !ids.insert(id))
        {
            return Err("Campeão já escolhido ou banido".into());
        }
    }
    let mut before = draft.clone();
    before.revision = 0;
    let mut after = next.clone();
    after.revision = 0;
    if before == after {
        draft.revision = command.revision;
        return Ok(ChangeKind::None);
    }
    for slot in before.allies.iter_mut().chain(&mut before.enemies) {
        slot.hover = None;
    }
    for slot in after.allies.iter_mut().chain(&mut after.enemies) {
        slot.hover = None;
    }
    if before == after {
        kind = ChangeKind::Presentation;
    }
    next.revision = command.revision;
    *draft = next;
    Ok(kind)
}
#[derive(serde::Deserialize)]
struct Scenario {
    allies: Vec<Selection>,
    enemies: Vec<Selection>,
    bans: Vec<ChampionId>,
}
pub fn preset(role: Role, index: usize) -> DraftState {
    let scenarios: Vec<Scenario> = serde_json::from_str(include_str!("../assets/scenarios.json"))
        .expect("embedded scenarios tested");
    let mut d = DraftState {
        role,
        phase: Phase::Pick,
        ..Default::default()
    };
    if let Some(s) = scenarios.get(index) {
        for (target, source) in d.allies.iter_mut().zip(&s.allies) {
            *target = source.clone();
        }
        for (target, source) in d.enemies.iter_mut().zip(&s.enemies) {
            *target = source.clone();
        }
        d.bans.extend(&s.bans);
    }
    // Curated lane-partner/opponent scenarios, not inferred champion positions.
    for slot in &mut d.allies {
        if slot.role == Some(role) {
            *slot = Selection::default();
        }
    }
    if index == 1 {
        match role {
            Role::Bot => {
                d.allies[0] = Selection {
                    champion: Some(267),
                    hover: None,
                    role: Some(Role::Support),
                }
            }
            Role::Support => {}
            Role::Top | Role::Jungle | Role::Mid => {
                d.allies.fill(Selection::default());
                let id = match role {
                    Role::Top => 58,
                    Role::Jungle => 254,
                    _ => 134,
                };
                d.enemies[0] = Selection {
                    champion: Some(id),
                    hover: None,
                    role: Some(role),
                };
            }
        }
    } else if index == 2 && role == Role::Bot {
        d.allies[0] = Selection {
            champion: Some(267),
            hover: None,
            role: Some(Role::Support),
        };
    }
    d
}
