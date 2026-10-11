//! Development-only fake. Not included in the application distribution.
#[allow(dead_code)]
#[path = "../tests/support/lcu_fake.rs"]
mod lcu_fake;
fn main() {
    let directory = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("temporary output directory required"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    let fake = lcu_fake::Fake::start();
    fake.route(
        "/lol-gameflow/v1/gameflow-phase",
        200,
        serde_json::json!("ChampSelect"),
    );
    fake.route(
        "/lol-champ-select/v1/session",
        200,
        serde_json::from_str(include_str!("../tests/support/lcu/session.json")).unwrap(),
    );
    std::fs::write(directory.join("certificate.der"), &fake.certificate).unwrap();
    std::fs::write(
        directory.join("lockfile"),
        format!("LeagueClient:1:{}:fake-password:https", fake.port),
    )
    .unwrap();
    println!("FAKE_PID={} ready", std::process::id());
    // Keep private test keys in this server's memory, never on disk.
    loop {
        std::thread::park();
    }
}
