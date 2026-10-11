use pikker::catalog::validate_catalog;
use pikker::{catalog::demo_catalog, domain::*};

#[test]
fn every_role_has_candidates() {
    let catalog = demo_catalog();
    for role in Role::ALL {
        assert!(
            catalog
                .values()
                .filter(|c| matches!(c.roles.get(&role), Some(RoleEvidence::Demonstrative { .. })))
                .count()
                >= 6
        );
    }
}

#[test]
fn duplicate_id_is_rejected() {
    let row = r#"{"id":1,"name":"A","roles":{"MID":{"kind":"insufficient"}},"difficulty":0.2}"#;
    assert!(validate_catalog(&format!("[{row},{row}]")).is_err());
}

#[test]
fn valid_catalog_is_accepted() {
    assert!(validate_catalog(r#"[{"id":1,"name":"Annie","roles":{"MID":{"kind":"demonstrative","viability":0.7}},"difficulty":0.2}]"#).is_ok());
}

#[test]
fn invalid_catalog_is_rejected() {
    for input in [
        "[]",
        "null",
        r#"[{"id":1,"name":"A","roles":{"INVALID":{"kind":"insufficient"}},"difficulty":0.2}]"#,
        r#"[{"id":1,"name":"A","roles":{"MID":{"kind":"demonstrative","viability":2.0}},"difficulty":0.2}]"#,
    ] {
        assert!(validate_catalog(input).is_err());
    }
}
