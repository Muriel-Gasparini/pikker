use pikker::{catalog::demo_catalog, domain::*, ranking::*};

fn draft(role: Role) -> DraftState {
    DraftState {
        role,
        phase: Phase::Pick,
        ..Default::default()
    }
}

#[test]
fn all_five_roles_with_empty_pool_have_more_than_three_results() {
    for role in Role::ALL {
        let rows = rank(
            &demo_catalog(),
            &draft(role),
            &PlayerProfile::default(),
            &Weights::default(),
        )
        .unwrap();
        assert!(rows.len() >= 6);
    }
}
#[test]
fn bans_and_confirmed_picks_excluded_but_hover_remains() {
    let mut d = draft(Role::Support);
    d.bans.insert(497);
    d.allies[0].champion = Some(111);
    d.enemies[0].champion = Some(267);
    d.enemies[1].hover = Some(201);
    let rows = rank(
        &demo_catalog(),
        &d,
        &PlayerProfile::default(),
        &Weights::default(),
    )
    .unwrap();
    assert!(rows.iter().all(|r| ![497, 111, 267].contains(&r.id)));
    assert!(rows.iter().any(|r| r.id == 201));
}
#[test]
fn favorite_does_not_override_viability_and_insufficient_is_marked() {
    let mut cat = demo_catalog();
    cat.get_mut(&89).unwrap().roles.insert(
        Role::Support,
        RoleEvidence::Demonstrative { viability: 0.0 },
    );
    let mut p = PlayerProfile::default();
    p.preferences.insert(
        (89, Role::Support),
        Preference {
            favorite: true,
            proficiency: Some(0.5),
        },
    );
    p.preferences.insert(
        (142, Role::Support),
        Preference {
            favorite: true,
            proficiency: None,
        },
    );
    let rows = rank(&cat, &draft(Role::Support), &p, &Weights::default()).unwrap();
    assert_ne!(rows[0].id, 89);
    assert_ne!(rows[0].id, 142);
    let zoe = rows.iter().find(|r| r.id == 142).unwrap();
    assert!(zoe.insufficient);
    assert!(zoe.score.is_none());
    assert!(rows.iter().find(|r| r.id == 497).unwrap().learning_notice);
}
#[test]
fn all_unavailable_returns_empty_and_ties_are_stable() {
    let cat = demo_catalog();
    let mut d = draft(Role::Support);
    d.bans.extend(cat.keys());
    assert!(
        rank(&cat, &d, &PlayerProfile::default(), &Weights::default())
            .unwrap()
            .is_empty()
    );
    let mut cat = demo_catalog();
    let a = cat[&1].clone();
    let mut b = a.clone();
    b.id = 2;
    cat.clear();
    cat.insert(1, a);
    cat.insert(2, b);
    let rows = rank(
        &cat,
        &draft(Role::Mid),
        &PlayerProfile::default(),
        &Weights::default(),
    )
    .unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
}
#[test]
fn invalid_weights_rejected_and_contributions_add_up() {
    let bad = Weights {
        viability: f32::NAN,
        ..Default::default()
    };
    assert!(
        rank(
            &demo_catalog(),
            &draft(Role::Top),
            &PlayerProfile::default(),
            &bad
        )
        .is_err()
    );
    for row in rank(
        &demo_catalog(),
        &draft(Role::Top),
        &PlayerProfile::default(),
        &Weights::default(),
    )
    .unwrap()
    {
        if let Some(score) = row.score {
            assert!(
                (score - row.contributions.iter().map(|c| c.value).sum::<f32>()).abs() < 0.00001
            );
        }
    }
}
