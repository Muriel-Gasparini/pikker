use pikker::{
    domain::Role,
    meta::{cache::*, model::*},
};

fn snapshot() -> MetaSnapshot {
    MetaSnapshot {
        schema_version: 1,
        key: MetaKey {
            source: "OP.GG".into(),
            schema_version: 1,
            requested_rank: Some("gold".into()),
        },
        fetched_at: 1_000_000,
        catalog: vec![ChampionIdentity {
            id: 25,
            api_name: "MORGANA".into(),
            display_name: "Morgana".into(),
        }],
        labels: StaticLabels::default(),
        stats: vec![RoleStats {
            champion_id: 25,
            role: Role::Support,
            games: Some(100),
            wins: Some(51),
            win_rate: 0.51,
            scope: SourceScope::default(),
        }],
        details: vec![],
    }
}

#[test]
fn fresh_cache_under_24h_and_expired_at_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let data = snapshot();
    assert!(load_cache(&path, &data.key, 1_000_000).unwrap().is_none());
    save_cache(&path, &data).unwrap();
    assert!(
        !load_cache(&path, &data.key, 1_086_399)
            .unwrap()
            .unwrap()
            .stale
    );
    assert!(
        load_cache(&path, &data.key, 1_086_400)
            .unwrap()
            .unwrap()
            .stale
    );
}

#[test]
fn unknown_scope_stays_unknown_and_rank_filter_does_not_become_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let data = snapshot();
    save_cache(&path, &data).unwrap();
    let restored = load_cache(&path, &data.key, 1_000_001).unwrap().unwrap();
    assert_eq!(restored.snapshot.stats[0].scope, SourceScope::default());
    let mut other = data.key.clone();
    other.requested_rank = Some("diamond".into());
    assert!(load_cache(&path, &other, 1_000_001).unwrap().is_none());
}

#[test]
fn wrong_schema_rejected_and_corrupt_original_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    std::fs::write(&path, b"{broken").unwrap();
    assert!(load_cache(&path, &snapshot().key, 1_000_001).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    let mut data = snapshot();
    data.schema_version = 2;
    assert!(save_cache(&path, &data).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
}

#[test]
fn atomic_validation_failure_preserves_previous_and_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let profile = dir.path().join("profile.json");
    std::fs::write(&profile, b"personal data").unwrap();
    let mut data = snapshot();
    save_cache(&path, &data).unwrap();
    let original = std::fs::read(&path).unwrap();
    data.stats[0].wins = Some(101);
    assert!(save_cache(&path, &data).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read(&profile).unwrap(), b"personal data");
}

#[test]
fn invalid_counts_nan_duplicate_rows_and_future_cache_rejected() {
    for (games, wins, rate) in [
        (Some(0), Some(0), 0.5),
        (Some(2), Some(3), 0.5),
        (Some(2), None, 0.5),
        (None, Some(1), 0.5),
        (None, None, f32::NAN),
        (None, None, 1.1),
    ] {
        let mut data = snapshot();
        data.stats[0].games = games;
        data.stats[0].wins = wins;
        data.stats[0].win_rate = rate;
        assert!(data.validate().is_err());
    }
    let mut data = snapshot();
    data.stats.push(data.stats[0].clone());
    assert!(data.validate().is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let data = snapshot();
    save_cache(&path, &data).unwrap();
    assert!(load_cache(&path, &data.key, 900_000).is_err());
}

#[test]
fn rounded_rates_do_not_fabricate_counts_and_counts_determine_rate() {
    let mut data = snapshot();
    data.stats[0].games = None;
    data.stats[0].wins = None;
    assert!(data.validate().is_ok());
    assert_eq!(data.stats[0].observed_rate(), 0.51);
    assert_eq!(data.stats[0].games, None);
    data.stats[0].games = Some(3);
    data.stats[0].wins = Some(2);
    data.stats[0].win_rate = 0.67;
    assert!((data.stats[0].observed_rate() - 2.0 / 3.0).abs() < 0.00001);
}
