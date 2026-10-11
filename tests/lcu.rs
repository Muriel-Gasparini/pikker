use pikker::{
    catalog::demo_catalog,
    controller::{Controller, SourceMode},
    domain::*,
    lcu::normalize::{normalize_pickable_ids, normalize_session},
    meta::model::*,
};
use serde_json::{Value, json};
fn session() -> Value {
    serde_json::from_str(include_str!("support/lcu/session.json")).unwrap()
}
fn action(actor: i64, id: u32, completed: bool) -> Value {
    json!({"id":1,"actorCellId":actor,"championId":id,"type":"pick","completed":completed,"isInProgress":!completed})
}
fn controller() -> Controller {
    let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
    c.set_source_mode(SourceMode::Connected).unwrap();
    c.set_meta(MetaSnapshot {
        schema_version: 1,
        key: MetaKey {
            source: "OP.GG".into(),
            schema_version: 1,
            requested_rank: None,
        },
        fetched_at: 1000,
        catalog: [267, 412, 25, 89]
            .into_iter()
            .map(|id| ChampionIdentity {
                id,
                api_name: format!("C{id}"),
                display_name: format!("C{id}"),
            })
            .collect(),
        stats: [267, 412, 25, 89]
            .into_iter()
            .map(|id| RoleStats {
                champion_id: id,
                role: Role::Bot,
                games: Some(100),
                wins: Some(50),
                win_rate: 0.5,
                scope: SourceScope::default(),
            })
            .collect(),
        details: vec![],
        labels: StaticLabels::default(),
    })
    .unwrap();
    c
}
#[test]
fn five_roles_and_unknown_enemy_position() {
    let s = normalize_session(&session(), 7, 1).unwrap();
    assert_eq!(
        s.state.allies.iter().map(|p| p.role).collect::<Vec<_>>(),
        vec![
            Some(Role::Top),
            Some(Role::Jungle),
            Some(Role::Mid),
            Some(Role::Bot),
            Some(Role::Support)
        ]
    );
    assert!(s.state.enemies.iter().all(|p| p.role.is_none()));
    assert_eq!(s.local_role, Some(Role::Bot));
    assert_eq!(s.local_champion_id, None);
}
#[test]
fn ally_hover_not_a_pick() {
    let mut raw = session();
    raw["myTeam"][3]["championPickIntent"] = json!(267);
    raw["myTeam"][3]["championId"] = json!(267);
    let s = normalize_session(&raw, 1, 1).unwrap();
    assert_eq!(s.state.allies[3].champion, None);
    assert_eq!(s.state.allies[3].hover, Some(267));
    assert_eq!(s.local_champion_id, None);
}
#[test]
fn enemy_hover_always_ignored() {
    let mut raw = session();
    raw["theirTeam"][0]["championPickIntent"] = json!(55);
    let s = normalize_session(&raw, 1, 1).unwrap();
    assert_eq!(s.state.enemies[0].hover, None);
    assert_eq!(s.state.enemies[0].champion, None);
}
#[test]
fn pending_enemy_action_not_confirmed() {
    let mut raw = session();
    raw["theirTeam"][0]["championId"] = json!(55);
    raw["actions"] = json!([[action(5, 55, false)]]);
    assert_eq!(
        normalize_session(&raw, 1, 1).unwrap().state.enemies[0].champion,
        None
    );
    raw["actions"][0][0]["completed"] = json!(true);
    assert_eq!(
        normalize_session(&raw, 1, 2).unwrap().state.enemies[0].champion,
        Some(55)
    );
}
#[test]
fn pending_enemy_action_stays_hidden_even_when_timer_says_finalization() {
    let mut raw = session();
    raw["timer"]["phase"] = json!("FINALIZATION");
    raw["theirTeam"][0]["championId"] = json!(55);
    raw["actions"] = json!([[action(5, 55, false)]]);
    assert_eq!(
        normalize_session(&raw, 1, 1).unwrap().state.enemies[0].champion,
        None
    );
}
#[test]
fn simultaneous_bans() {
    let mut raw = session();
    raw["bans"]["myTeamBans"] = json!([25, 0]);
    raw["bans"]["theirTeamBans"] = json!([89, -1]);
    raw["actions"] = json!([[{"actorCellId":0,"championId":267,"type":"ban","completed":true,"isInProgress":false},{"actorCellId":5,"championId":412,"type":"ban","completed":false,"isInProgress":true}]]);
    let s = normalize_session(&raw, 1, 1).unwrap();
    assert_eq!(
        s.state.bans.into_iter().collect::<Vec<_>>(),
        vec![25, 89, 267]
    );
    assert_eq!(s.state.phase, Phase::Ban);
}
#[test]
fn completed_enemy_ban_is_not_exposed_before_public_team_bans() {
    let mut raw = session();
    raw["actions"] = json!([[{"actorCellId":5,"championId":267,"type":"ban","completed":true,"isInProgress":false}]]);
    assert!(
        !normalize_session(&raw, 1, 1)
            .unwrap()
            .state
            .bans
            .contains(&267)
    );
    raw["bans"]["theirTeamBans"] = json!([267]);
    assert!(
        normalize_session(&raw, 1, 2)
            .unwrap()
            .state
            .bans
            .contains(&267)
    );
}
#[test]
fn swap_after_lock_uses_current_team_pick() {
    let mut raw = session();
    raw["myTeam"][0]["championId"] = json!(89);
    raw["myTeam"][1]["championId"] = json!(25);
    raw["actions"] = json!([[action(0, 25, true), action(1, 89, true)]]);
    let s = normalize_session(&raw, 1, 1).unwrap();
    assert_eq!(s.state.allies[0].champion, Some(89));
    assert_eq!(s.state.allies[1].champion, Some(25));
}
#[test]
fn delete_and_new_generation_clear_old_draft() {
    let mut c = controller();
    let mut raw = session();
    raw["bans"]["myTeamBans"] = json!([25]);
    c.apply_snapshot(normalize_session(&raw, 1, 5).unwrap())
        .unwrap();
    c.apply_snapshot(normalize_session(&Value::Null, 2, 1).unwrap())
        .unwrap();
    assert!(c.draft.bans.is_empty());
    assert!(c.rows.is_empty());
    assert_eq!(c.draft.phase, Phase::Waiting);
    c.apply_snapshot(normalize_session(&session(), 3, 1).unwrap())
        .unwrap();
    assert_eq!(c.draft.session_id, 3);
    assert!(!c.rows.is_empty());
}
#[test]
fn stale_snapshot_ignored_before_payload_validation() {
    let mut c = controller();
    c.apply_snapshot(normalize_session(&session(), 4, 10).unwrap())
        .unwrap();
    let count = c.computations;
    let mut old = normalize_session(&session(), 3, 20).unwrap();
    old.state.allies.clear();
    c.apply_snapshot(old).unwrap();
    let mut old = normalize_session(&session(), 4, 9).unwrap();
    old.state.enemies.clear();
    c.apply_snapshot(old).unwrap();
    assert_eq!(c.draft.revision, 10);
    assert_eq!(c.computations, count);
}
#[test]
fn pickable_filter_respects_ownership() {
    let mut c = controller();
    let mut s = normalize_session(&session(), 1, 1).unwrap();
    s.pickable_ids = Some(normalize_pickable_ids(&json!([267, 412, 267])).unwrap());
    c.apply_snapshot(s).unwrap();
    assert_eq!(
        c.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![267, 412]
    );
}
#[test]
fn extra_fields_ignored_missing_critical_fields_rejected() {
    let mut raw = session();
    raw["unknownField"] = json!({"whatever":123});
    assert!(normalize_session(&raw, 1, 1).is_ok());
    for key in [
        "actions",
        "myTeam",
        "theirTeam",
        "localPlayerCellId",
        "timer",
        "bans",
    ] {
        let mut raw = session();
        raw.as_object_mut().unwrap().remove(key);
        assert!(normalize_session(&raw, 1, 1).is_err(), "{key}");
    }
    raw["myTeam"][0]["championId"] = json!(-2);
    assert!(normalize_session(&raw, 1, 1).is_err());
    assert!(normalize_pickable_ids(&json!([0, -3])).is_err());
}
#[test]
fn no_op_and_hover_consume_revision_without_recomputing() {
    let mut c = controller();
    let raw = session();
    c.apply_snapshot(normalize_session(&raw, 1, 1).unwrap())
        .unwrap();
    let count = c.computations;
    let mut raw = raw;
    raw["myTeam"][3]["championPickIntent"] = json!(267);
    c.apply_snapshot(normalize_session(&raw, 1, 2).unwrap())
        .unwrap();
    assert_eq!(c.computations, count);
    assert_eq!(c.draft.revision, 2);
    assert_eq!(c.draft.allies[3].hover, Some(267));
}
#[test]
fn flat_actions_and_finalization_use_confirmed_current_pick() {
    let mut raw = session();
    raw["actions"] = json!([action(3, 267, true)]);
    raw["myTeam"][3]["championId"] = json!(267);
    let s = normalize_session(&raw, 1, 1).unwrap();
    assert_eq!(s.local_champion_id, Some(267));
    raw["timer"]["phase"] = json!("FINALIZATION");
    assert_eq!(
        normalize_session(&raw, 1, 2).unwrap().state.phase,
        Phase::Finalization
    );
}
