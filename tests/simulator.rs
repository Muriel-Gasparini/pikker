use pikker::{catalog::demo_catalog, controller::Controller, domain::*, simulator::*};
fn command(d: &DraftState, action: Action) -> DraftCommand {
    DraftCommand {
        session_id: d.session_id,
        revision: d.revision + 1,
        action,
    }
}

#[test]
fn presets_for_every_role_and_unknown_positions_preserved() {
    for role in Role::ALL {
        for i in 0..4 {
            let d = preset(role, i);
            assert_eq!(d.role, role);
            assert_eq!(d.phase, Phase::Pick);
        }
    }
    let d = preset(Role::Support, 2);
    assert!(
        d.enemies
            .iter()
            .any(|s| s.champion.is_none() && s.role.is_none())
    );
}
#[test]
fn stale_events_and_new_session() {
    let mut d = preset(Role::Mid, 1);
    let original = d.clone();
    assert_eq!(
        apply(
            &mut d,
            DraftCommand {
                session_id: 0,
                revision: 99,
                action: Action::SetRole(Role::Top)
            }
        )
        .unwrap(),
        ChangeKind::None
    );
    assert_eq!(d, original);
    let cmd = command(&d, Action::NewSession);
    apply(&mut d, cmd).unwrap();
    assert!(d.allies.iter().all(|s| s.champion.is_none()));
    assert!(d.bans.is_empty());
    assert_ne!(d.session_id, original.session_id);
}
#[test]
fn duplicate_selection_rejected_without_mutation() {
    let mut d = preset(Role::Support, 0);
    let cmd = command(
        &d,
        Action::ConfirmPick {
            enemy: false,
            slot: 0,
            id: Some(111),
            role: None,
        },
    );
    apply(&mut d, cmd).unwrap();
    let original = d.clone();
    let cmd = command(
        &d,
        Action::ConfirmPick {
            enemy: true,
            slot: 1,
            id: Some(111),
            role: None,
        },
    );
    assert!(apply(&mut d, cmd).is_err());
    assert_eq!(d, original);
    let cmd = command(
        &d,
        Action::SetBan {
            id: 111,
            banned: true,
        },
    );
    assert!(apply(&mut d, cmd).is_err());
}
#[test]
fn hover_does_not_recompute_and_waiting_does_not_analyze() {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.recompute().unwrap();
    assert_eq!(c.computations, 0);
    c.dispatch(command(&c.draft, Action::SetPhase(Phase::Pick)))
        .unwrap();
    assert_eq!(c.computations, 1);
    c.dispatch(command(
        &c.draft,
        Action::SetHover {
            enemy: true,
            slot: 0,
            id: Some(1),
        },
    ))
    .unwrap();
    assert_eq!(c.computations, 1);
    c.dispatch(command(&c.draft, Action::SetRole(Role::Top)))
        .unwrap();
    assert_eq!(c.computations, 2);
}

#[test]
fn noop_consumes_revision_and_blocks_older_events() {
    let mut d = preset(Role::Support, 0);
    let cmd = DraftCommand {
        session_id: d.session_id,
        revision: 10,
        action: Action::SetRole(Role::Support),
    };
    assert_eq!(apply(&mut d, cmd).unwrap(), ChangeKind::None);
    let cmd = DraftCommand {
        session_id: d.session_id,
        revision: 9,
        action: Action::SetRole(Role::Top),
    };
    assert_eq!(apply(&mut d, cmd).unwrap(), ChangeKind::None);
    assert_eq!(d.role, Role::Support);
}

#[test]
fn bot_preset_has_known_support_partner_and_player_candidates_available() {
    let d = preset(Role::Bot, 1);
    assert!(
        d.allies
            .iter()
            .any(|s| s.role == Some(Role::Support) && s.champion.is_some())
    );
    assert!(d.allies.iter().all(|s| s.role != Some(Role::Bot)));
}

#[test]
fn clearing_hover_only_slot_does_not_recompute() {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.dispatch(command(&c.draft, Action::SetPhase(Phase::Pick)))
        .unwrap();
    c.dispatch(command(
        &c.draft,
        Action::SetHover {
            enemy: true,
            slot: 0,
            id: Some(1),
        },
    ))
    .unwrap();
    let count = c.computations;
    c.dispatch(command(
        &c.draft,
        Action::ConfirmPick {
            enemy: true,
            slot: 0,
            id: None,
            role: None,
        },
    ))
    .unwrap();
    assert_eq!(c.computations, count);
    assert!(c.draft.enemies[0].hover.is_none());
}
