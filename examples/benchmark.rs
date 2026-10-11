use pikker::{catalog::demo_catalog, domain::*, ranking::*};
use std::{collections::BTreeMap, time::Instant};
fn measure(catalog: &pikker::catalog::Catalog, profile: &PlayerProfile, iterations: u32) {
    let draft = DraftState {
        role: Role::Support,
        phase: Phase::Pick,
        ..Default::default()
    };
    let mut durations = Vec::with_capacity(iterations as usize);
    let mut count = 0;
    for _ in 0..iterations {
        let start = Instant::now();
        count = std::hint::black_box(rank(catalog, &draft, profile, &Weights::default()).unwrap())
            .len();
        durations.push(start.elapsed());
    }
    let mean = durations.iter().map(|d| d.as_secs_f64()).sum::<f64>() / f64::from(iterations);
    durations.sort_unstable();
    let p95 = durations[(durations.len() * 95).div_ceil(100) - 1];
    println!(
        "catalog={} profile={} results={} iterations={} mean_ms={:.4} p95_ms={:.4}",
        catalog.len(),
        profile.preferences.len(),
        count,
        iterations,
        mean * 1000.0,
        p95.as_secs_f64() * 1000.0,
    );
}
fn main() {
    if let Some(path) = std::env::args().nth(1) {
        let data: pikker::meta::model::MetaSnapshot =
            serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap();
        data.validate().unwrap();
        let mut controller =
            pikker::controller::Controller::new(demo_catalog(), PlayerProfile::default());
        controller
            .set_source_mode(pikker::controller::SourceMode::Connected)
            .unwrap();
        controller.set_meta(data).unwrap();
        measure(&controller.catalog, &controller.profile, 1000);
    }
    let demo = demo_catalog();
    measure(&demo, &PlayerProfile::default(), 1000);
    let mut large = BTreeMap::new();
    let mut profile = PlayerProfile::default();
    for n in 0..100 {
        for champ in demo.values() {
            let mut c = champ.clone();
            c.id += n * 1000;
            profile.preferences.insert(
                (c.id, Role::Support),
                Preference {
                    favorite: true,
                    proficiency: Some(0.8),
                },
            );
            large.insert(c.id, c);
        }
    }
    measure(&large, &profile, 100);
}
