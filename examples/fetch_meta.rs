//! Explicit read-only live smoke test, not part of the offline test suite.
use pikker::domain::Role;
use pikker::meta::{cache::save_cache, opgg::*};
use std::path::Path;

fn run() -> Result<(), pikker::runtime::IntegrationError> {
    let (catalog, labels) = fetch_static_catalog(&PublicTransport::default(), "br")?;
    println!("Official catalog: {} champions", catalog.len());
    let nami = catalog
        .iter()
        .find(|c| c.display_name == "Nami")
        .cloned()
        .unwrap();
    let mut provider = OpggProvider::new(PublicTransport::default(), catalog, Some("gold".into()))
        .with_labels(labels);
    let mut snapshot = provider.fetch_general()?;
    println!("Source combinations: {}", snapshot.stats.len());
    let details = provider.fetch_details(&nami, Role::Support, Some("gold"))?;
    println!(
        "Nami: {} rune builds / {} spell builds",
        details.runes.len(),
        details.spells.len()
    );
    snapshot.details.push(details);
    if let Some(path) = std::env::args().nth(1) {
        save_cache(Path::new(&path), &snapshot)?;
        println!("Validated cache saved");
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
