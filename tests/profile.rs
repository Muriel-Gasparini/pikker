use pikker::{domain::*, profile::*};
use std::{fs, path::PathBuf};
fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pikker-test-{}-{name}.json", std::process::id()))
}
#[test]
fn empty_and_large_profiles_roundtrip() {
    let path = path("roundtrip");
    let mut p = PlayerProfile::default();
    save_profile(&path, &p).unwrap();
    assert!(path.exists());
    assert_eq!(load_profile(&path).unwrap(), p);
    for id in 1..1000 {
        p.preferences.insert(
            (id, Role::Mid),
            Preference {
                favorite: true,
                proficiency: Some(0.7),
            },
        );
    }
    save_profile(&path, &p).unwrap();
    assert_eq!(load_profile(&path).unwrap(), p);
    fs::remove_file(path).unwrap();
}
#[test]
fn corrupt_profile_reports_error_and_does_not_get_overwritten() {
    let path = path("corrupt");
    fs::write(&path, "broken").unwrap();
    assert!(load_profile(&path).is_err());
    let mut p = PlayerProfile::default();
    p.preferences.insert(
        (1, Role::Mid),
        Preference {
            favorite: true,
            proficiency: Some(f32::NAN),
        },
    );
    assert!(save_profile(&path, &p).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
    fs::remove_file(path).unwrap();
}
#[test]
fn duplicate_preferences_normalized() {
    let path = path("duplicate");
    fs::write(&path,r#"[{"id":1,"role":"MID","favorite":false,"proficiency":null},{"id":1,"role":"MID","favorite":true,"proficiency":0.8}]"#).unwrap();
    let p = load_profile(&path).unwrap();
    assert_eq!(p.preferences.len(), 1);
    assert!(p.preferences[&(1, Role::Mid)].favorite);
    fs::remove_file(path).unwrap();
}

#[test]
fn favorite_edit_preserves_zero_and_fractional_proficiency() {
    for value in [Some(0.0), Some(0.7), None] {
        let p = Preference {
            favorite: false,
            proficiency: value,
        };
        let edited = edit_preference(&p, true, proficiency_level(&p), false).unwrap();
        assert_eq!(edited.proficiency, value);
        assert!(edited.favorite);
    }
    assert_ne!(
        proficiency_level(&Preference {
            favorite: false,
            proficiency: Some(0.0)
        }),
        proficiency_level(&Preference::default())
    );
    assert_eq!(
        edit_preference(&Preference::default(), true, 1, true)
            .unwrap()
            .proficiency,
        Some(0.0)
    );
}
