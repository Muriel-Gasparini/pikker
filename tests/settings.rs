use pikker::settings::*;
#[test]
fn rank_selection_survives_restart_and_invalid_file_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    assert_eq!(load_settings(&path).unwrap().rank, None);
    save_settings(
        &path,
        &AppSettings {
            rank: Some("gold".into()),
        },
    )
    .unwrap();
    assert_eq!(load_settings(&path).unwrap().rank.as_deref(), Some("gold"));
    std::fs::write(&path, b"{broken").unwrap();
    assert!(load_settings(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
}
#[test]
fn unsupported_rank_does_not_replace_existing_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    save_settings(&path, &AppSettings::default()).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(
        save_settings(
            &path,
            &AppSettings {
                rank: Some("unknown".into())
            }
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
