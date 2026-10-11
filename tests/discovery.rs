use pikker::lcu::discovery::{discover, lockfile_candidates, parse_lockfile};
#[test]
fn executable_paths_with_spaces_are_deduplicated_without_scanning_other_folders() {
    let paths = vec![
        "/tmp/My League/LeagueClient.exe".into(),
        "/tmp/My League/LeagueClientUx.exe".into(),
        "/tmp/Other/NotTheClient.exe".into(),
    ];
    assert_eq!(
        lockfile_candidates(&paths),
        vec![std::path::PathBuf::from("/tmp/My League/lockfile")]
    );
}
#[test]
fn configured_lockfile_is_bounded_and_credentials_are_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lockfile");
    assert!(discover(Some(&path)).unwrap().is_none());
    std::fs::write(&path, "LeagueClient:1234:54321:fake-password:https\n").unwrap();
    let credentials = discover(Some(&path)).unwrap().unwrap();
    let debug = format!("{credentials:?}");
    assert!(!debug.contains("fake-password"));
    assert!(debug.contains("54321"));
    std::fs::write(&path, vec![b'x'; 4097]).unwrap();
    assert!(discover(Some(&path)).is_err());
}
#[test]
fn malformed_credentials_fail_without_exposing_contents() {
    for text in [
        "LeagueClient:1:0:fake:https",
        "LeagueClient:1:65536:fake:https",
        "LeagueClient:0:1234:fake:https",
        "LeagueClient:1:1234::https",
        "Other:1:1234:fake:https",
        "LeagueClient:1:1234:fake:http",
        "LeagueClient:1:1234:fake\r\nInjected:https",
    ] {
        let error = parse_lockfile(text).unwrap_err().to_string();
        assert!(!error.contains("fake"));
    }
    assert!(parse_lockfile("LeagueClient:1:1234:fake-password:https").is_ok());
}
