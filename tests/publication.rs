use std::{fs, path::PathBuf};

#[test]
fn distributed_source_includes_license_and_third_party_notices() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for name in ["LICENSE", "THIRD_PARTY_NOTICES.txt"] {
        let text = fs::read_to_string(root.join(name)).expect("required distribution notice");
        assert!(text.len() > 100, "empty distribution notice: {name}");
    }
}

#[test]
fn public_fixtures_retain_a_verifiable_contract_pin() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pin: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("tests/support/lcu/lcu-contract-pin.json"))
            .expect("public contract pin"),
    )
    .unwrap();
    assert_eq!(pin["client_version"], "16.19.820.7193");
    assert_eq!(pin["sha256"].as_str().unwrap().len(), 64);
    assert!(pin["required_consumed_fields"].as_object().unwrap().len() >= 3);
}
