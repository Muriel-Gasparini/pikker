use pikker::{catalog::demo_catalog, controller::Controller, domain::*, ranking::*};

#[test]
fn unknown_preferences_do_not_change_rank() {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.draft.phase = Phase::Pick;
    c.recompute().unwrap();
    let before: Vec<_> = c.rows.iter().map(|r| (r.id, r.score)).collect();
    c.profile.preferences.insert(
        (99999, Role::Support),
        Preference {
            favorite: true,
            proficiency: Some(1.0),
        },
    );
    c.recompute().unwrap();
    assert_eq!(
        before,
        c.rows.iter().map(|r| (r.id, r.score)).collect::<Vec<_>>()
    );
}
#[test]
fn preference_and_weights_validation_preserve_valid_state() {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    assert!(
        c.set_preference(
            1,
            Preference {
                favorite: true,
                proficiency: Some(2.0)
            }
        )
        .is_err()
    );
    assert!(c.profile.preferences.is_empty());
    assert!(
        c.set_weights(Weights {
            preference: 0.9,
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(c.weights.preference, 0.05);
}

#[test]
fn stale_unknown_champion_event_is_ignored() {
    use pikker::simulator::*;
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    let cmd = DraftCommand {
        session_id: 0,
        revision: 100,
        action: Action::SetBan {
            id: 999999,
            banned: true,
        },
    };
    assert!(c.dispatch(cmd).is_ok());
    assert!(c.draft.bans.is_empty());
    assert_eq!(c.computations, 0);
}
#[test]
fn source_mode_switch_preserves_selected_fallback_role() {
    let mut c = pikker::controller::Controller::new(
        pikker::catalog::demo_catalog(),
        pikker::domain::PlayerProfile::default(),
    );
    c.draft.role = pikker::domain::Role::Top;
    c.set_source_mode(pikker::controller::SourceMode::Connected)
        .unwrap();
    assert_eq!(c.draft.role, pikker::domain::Role::Top);
    c.set_source_mode(pikker::controller::SourceMode::Simulator)
        .unwrap();
    assert_eq!(c.draft.role, pikker::domain::Role::Top);
}
