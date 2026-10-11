use pikker::{
    catalog::demo_catalog,
    controller::{Controller, SourceMode},
    domain::*,
    meta::model::*,
    ranking::Weights,
    simulator::*,
};

fn snapshot() -> MetaSnapshot {
    let catalog = (0..12)
        .map(|i| ChampionIdentity {
            id: 10000 + i,
            api_name: format!("C{i}"),
            display_name: format!("C{i}"),
        })
        .collect();
    let stats = (0..12)
        .map(|i| RoleStats {
            champion_id: 10000 + i,
            role: Role::Support,
            games: Some(1000),
            wins: Some(510),
            win_rate: 0.51,
            scope: SourceScope::default(),
        })
        .collect();
    MetaSnapshot {
        schema_version: 1,
        key: MetaKey {
            source: "OP.GG".into(),
            schema_version: 1,
            requested_rank: None,
        },
        fetched_at: 1000,
        catalog,
        labels: StaticLabels::default(),
        stats,
        details: vec![],
    }
}
fn prepared() -> Controller {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.set_source_mode(SourceMode::Connected).unwrap();
    c.set_meta(snapshot()).unwrap();
    c.dispatch(DraftCommand {
        session_id: c.draft.session_id,
        revision: c.draft.revision + 1,
        action: Action::SetPhase(Phase::Pick),
    })
    .unwrap();
    c
}

#[test]
fn changing_elo_hides_old_details_even_when_refresh_never_succeeds() {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.set_source_mode(SourceMode::Connected).unwrap();
    c.set_requested_rank(Some("GOLD".into()));
    let mut data = snapshot();
    data.key.requested_rank = Some("GOLD".into());
    data.details.push(ChampionDetails {
        champion_id: 10000,
        role: Role::Support,
        requested_rank: Some("GOLD".into()),
        scope: SourceScope::default(),
        runes: vec![],
        spells: vec![],
        counters: vec![],
        synergies: vec![],
        fetched_at: 1000,
    });
    c.set_meta(data).unwrap();
    assert!(c.selected_details(10000, Role::Support).is_some());
    c.set_requested_rank(Some("DIAMOND".into()));
    assert!(c.selected_details(10000, Role::Support).is_none());
    assert_eq!(c.meta.as_ref().unwrap().stats.len(), 12);
    c.set_requested_rank(Some("GOLD".into()));
    assert!(c.selected_details(10000, Role::Support).is_some());
}

#[test]
fn all_source_candidates_not_only_top_ten_and_favorite_outside_shortlist_remains_candidate() {
    let mut c = prepared();
    assert_eq!(c.rows.len(), 12);
    c.set_preference(
        10011,
        Preference {
            favorite: true,
            proficiency: Some(0.8),
        },
    )
    .unwrap();
    assert!(
        c.rows
            .iter()
            .any(|r| r.id == 10011 && r.favorite && r.score.is_some())
    );
}

#[test]
fn missing_traits_omit_dimension_for_all_candidates() {
    let mut data = snapshot();
    data.catalog.push(ChampionIdentity {
        id: 25,
        api_name: "MORGANA".into(),
        display_name: "Morgana".into(),
    });
    data.stats.push(RoleStats {
        champion_id: 25,
        role: Role::Support,
        games: Some(1000),
        wins: Some(510),
        win_rate: 0.51,
        scope: SourceScope::default(),
    });
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.set_meta(data).unwrap();
    c.draft.phase = Phase::Pick;
    c.recompute().unwrap();
    assert!(c.rows.iter().all(|r| {
        r.contributions
            .iter()
            .all(|v| v.key != "composition" && v.key != "blind")
    }));
    let a = c.rows.iter().find(|r| r.id == 25).unwrap().score.unwrap();
    let b = c
        .rows
        .iter()
        .find(|r| r.id == 10000)
        .unwrap()
        .score
        .unwrap();
    assert!((a - b).abs() < 0.000001);
}

#[test]
fn zero_remaining_weights_produce_no_score() {
    let mut c = prepared();
    c.set_weights(Weights {
        viability: 0.0,
        proficiency: 0.0,
        composition: 1.0,
        blind: 0.0,
        preference: 0.0,
    })
    .unwrap();
    assert!(c.rows.iter().all(|r| r.score.is_none()));
}

#[test]
fn demo_evidence_not_used_connected() {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.set_source_mode(SourceMode::Connected).unwrap();
    c.draft.phase = Phase::Pick;
    c.recompute().unwrap();
    assert!(c.rows.is_empty());
    c.set_source_mode(SourceMode::Simulator).unwrap();
    c.draft.phase = Phase::Pick;
    c.recompute().unwrap();
    assert!(c.rows.iter().any(|r| r.score.is_some()));
}

#[test]
fn confirmed_own_pick_keeps_details_accessible_and_selection_has_no_io() {
    let mut c = prepared();
    c.draft.phase = Phase::Waiting; // End the test draft before publishing fresh data.
    let mut data = snapshot();
    data.details.push(ChampionDetails {
        champion_id: 10000,
        role: Role::Support,
        requested_rank: None,
        scope: SourceScope::default(),
        runes: vec![],
        spells: vec![BuildChoice {
            ids: vec![4, 14],
            names: vec!["Flash".into(), "Ignite".into()],
            games: Some(10),
            wins: Some(6),
        }],
        counters: vec![],
        synergies: vec![],
        fetched_at: 1000,
    });
    c.set_meta(data).unwrap();
    c.draft.phase = Phase::Pick;
    c.dispatch(DraftCommand {
        session_id: c.draft.session_id,
        revision: c.draft.revision + 1,
        action: Action::ConfirmPick {
            enemy: false,
            slot: 0,
            id: Some(10000),
            role: Some(Role::Support),
        },
    })
    .unwrap();
    assert!(!c.rows.iter().any(|r| r.id == 10000));
    assert_eq!(
        c.selected_details(10000, Role::Support).unwrap().spells[0].ids,
        vec![4, 14]
    );
    let before = c.computations;
    for _ in 0..100 {
        assert!(c.selected_details(10000, Role::Support).is_some());
    }
    assert_eq!(c.computations, before);
}

#[test]
fn meta_updates_during_draft_are_applied_only_after_session_ends() {
    let mut c = prepared();
    let before = c.rows[0].score;
    let mut data = snapshot();
    data.fetched_at = 1100;
    data.stats[0].wins = Some(990);
    c.set_meta(data).unwrap();
    assert_eq!(c.rows[0].score, before);
    c.dispatch(DraftCommand {
        session_id: c.draft.session_id,
        revision: c.draft.revision + 1,
        action: Action::SetPhase(Phase::Ended),
    })
    .unwrap();
    c.dispatch(DraftCommand {
        session_id: c.draft.session_id,
        revision: c.draft.revision + 1,
        action: Action::SetPhase(Phase::Pick),
    })
    .unwrap();
    assert_eq!(c.rows[0].id, 10000);
    assert!(c.rows[0].score.unwrap() > before.unwrap());
}
