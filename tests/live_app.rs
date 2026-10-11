#[path = "support/lcu_fake.rs"]
mod lcu_fake;
use lcu_fake::Fake;
use pikker::lcu::listener::{LcuConfig, LcuControl, LcuWaiter, run_listener};
use pikker::lcu::transport::{LcuEvent, LcuTransport, LcuTrust};
use pikker::meta::bootstrap::BootstrapControl;
use pikker::runtime::{IntegrationError, RuntimeEvent};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

struct RecordedWaits(Mutex<Vec<Duration>>);
#[test]
fn draft_invalidation_pauses_external_requests_before_slow_resync() {
    let fake = Fake::start();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("Lobby"));
    let credentials = fake.credentials();
    let config = LcuConfig::new(
        Arc::new(move || Ok(Some(credentials.clone()))),
        LcuTrust::from_der(&fake.certificate).unwrap(),
    );
    let control = LcuControl::default();
    let bootstrap = BootstrapControl::default();
    let (worker_control, worker_bootstrap) = (control.clone(), bootstrap.clone());
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_listener(
            config,
            worker_control,
            worker_bootstrap,
            Arc::new(move |e| {
                let _ = send.send(e);
            }),
        )
    });
    loop {
        if matches!(
            receive.recv_timeout(Duration::from_secs(3)).unwrap(),
            RuntimeEvent::Connection(pikker::runtime::ConnectionStatus::Connected)
        ) {
            break;
        }
    }
    assert!(bootstrap.checkpoint().is_ok());
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    let (entered, release) = fake.pause_response("/lol-gameflow/v1/gameflow-phase");
    fake.emit("/lol-champ-select/v1/session", "Create", json!({}));
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    let paused = matches!(bootstrap.checkpoint(), Err(IntegrationError::Paused));
    control.cancel();
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(
        paused,
        "new external requests remain allowed while the draft resync is blocked"
    );
}
impl LcuWaiter for RecordedWaits {
    fn wait(
        &self,
        delay: Duration,
        control: &LcuControl,
        epoch: u64,
    ) -> Result<(), IntegrationError> {
        let mut waits = self.0.lock().unwrap();
        waits.push(delay);
        if waits.len() == 5 {
            control.cancel();
        }
        control.check(epoch)
    }
}
#[test]
fn client_absent_backoff_2_5_15_30() {
    let waiter = Arc::new(RecordedWaits(Mutex::new(Vec::new())));
    let mut config = LcuConfig::new(Arc::new(|| Ok(None)), LcuTrust::riot().unwrap());
    config.waiter = waiter.clone();
    let control = LcuControl::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    run_listener(
        config,
        control,
        BootstrapControl::default(),
        Arc::new(move |e| observed.lock().unwrap().push(e)),
    );
    assert_eq!(
        *waiter.0.lock().unwrap(),
        vec![
            Duration::from_secs(2),
            Duration::from_secs(5),
            Duration::from_secs(15),
            Duration::from_secs(30),
            Duration::from_secs(30)
        ]
    );
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(
                e,
                RuntimeEvent::Connection(pikker::runtime::ConnectionStatus::Absent)
            ))
            .count(),
        1
    );
}
#[test]
fn rediscovery_absent_does_not_pause_existing_bootstrap() {
    let bootstrap = BootstrapControl::default();
    let observed_bootstrap = bootstrap.clone();
    let allowed = Arc::new(Mutex::new(Vec::new()));
    let recorded = allowed.clone();
    let mut config = LcuConfig::new(
        Arc::new(move || {
            recorded
                .lock()
                .unwrap()
                .push(observed_bootstrap.checkpoint().is_ok());
            Ok(None)
        }),
        LcuTrust::riot().unwrap(),
    );
    config.waiter = Arc::new(RecordedWaits(Mutex::new(Vec::new())));
    run_listener(config, LcuControl::default(), bootstrap, Arc::new(|_| {}));
    assert_eq!(*allowed.lock().unwrap(), vec![true; 5]);
}
#[test]
fn obsolete_discovery_cannot_pause_manual_mode() {
    let bootstrap = BootstrapControl::default();
    let control = LcuControl::default();
    let changed = control.clone();
    let manual = bootstrap.clone();
    let config = LcuConfig::new(
        Arc::new(move || {
            changed.set_automatic(false);
            manual.set_draft_active(false, &|_| {});
            changed.cancel();
            Ok(Some(
                pikker::lcu::discovery::parse_lockfile("LeagueClient:1:1:fake:https").unwrap(),
            ))
        }),
        LcuTrust::riot().unwrap(),
    );
    run_listener(config, control, bootstrap.clone(), Arc::new(|_| {}));
    assert!(bootstrap.checkpoint().is_ok());
}
#[test]
fn listener_publishes_initial_draft_and_cancels_live_io() {
    let fake = Fake::start();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route("/lol-champ-select/v1/session", 200, session());
    let credentials = fake.credentials();
    let config = LcuConfig::new(
        Arc::new(move || Ok(Some(credentials.clone()))),
        LcuTrust::from_der(&fake.certificate).unwrap(),
    );
    let control = LcuControl::default();
    let worker_control = control.clone();
    let bootstrap = BootstrapControl::default();
    let worker_bootstrap = bootstrap.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_listener(
            config,
            worker_control,
            worker_bootstrap,
            Arc::new(move |e| {
                let _ = tx.send(e);
            }),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let initial = loop {
        if let RuntimeEvent::Draft(snapshot) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            break snapshot;
        }
    };
    assert_eq!(initial.local_role, Some(pikker::domain::Role::Bot));
    assert!(matches!(
        bootstrap.checkpoint(),
        Err(IntegrationError::Paused)
    ));
    let start = Instant::now();
    control.cancel();
    worker.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
}
#[test]
fn complete_draft_has_zero_opgg_requests() {
    use pikker::{
        domain::{PlayerProfile, Role},
        runtime::{RuntimeCommand, RuntimeConfig, start_runtime},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fake = Fake::start();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route("/lol-champ-select/v1/session", 200, session());
    let credentials = fake.credentials();
    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    let cache = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::with_provider(
        cache.path().join("meta.json"),
        PlayerProfile::default(),
        Role::Support,
        None,
        Arc::new(move |_, _, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(IntegrationError::Network)
        }),
    );
    config.lcu = Some(LcuConfig::new(
        Arc::new(move || Ok(Some(credentials.clone()))),
        LcuTrust::from_der(&fake.certificate).unwrap(),
    ));
    let (tx, rx) = mpsc::channel();
    let runtime = start_runtime(
        config,
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    )
    .unwrap();
    runtime.send(RuntimeCommand::StartBootstrap).unwrap();
    let mut draft = false;
    let mut paused = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !draft || !paused {
        match rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            RuntimeEvent::Draft(s) => {
                draft = true;
                assert_eq!(s.local_role, Some(Role::Bot));
            }
            RuntimeEvent::Progress(p)
                if p.stage == pikker::meta::bootstrap::BootstrapStage::Paused =>
            {
                paused = true
            }
            _ => {}
        }
    }
    let mut picked = session();
    picked["myTeam"][3]["championId"] = json!(22);
    picked["actions"][0][0]["championId"] = json!(22);
    picked["actions"][0][0]["completed"] = json!(true);
    picked["actions"][0][0]["isInProgress"] = json!(false);
    picked["timer"]["phase"] = json!("FINALIZATION");
    picked["bans"]["myTeamBans"] = json!([25]);
    fake.route("/lol-champ-select/v1/session", 200, picked.clone());
    fake.emit("/lol-champ-select/v1/session", "Update", picked.clone());
    assert_eq!(next_draft(&rx).local_champion_id, Some(22));
    // Swap: current team entry, not the older completed action, is authoritative.
    picked["myTeam"][3]["championId"] = json!(51);
    fake.route("/lol-champ-select/v1/session", 200, picked.clone());
    fake.emit("/lol-champ-select/v1/session", "Update", picked);
    assert_eq!(next_draft(&rx).local_champion_id, Some(51));
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("InProgress"));
    fake.emit(
        "/lol-gameflow/v1/gameflow-phase",
        "Update",
        json!("InProgress"),
    );
    assert_eq!(next_draft(&rx).state.phase, pikker::domain::Phase::Waiting);
    runtime.shutdown().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
}
fn next_draft(rx: &mpsc::Receiver<RuntimeEvent>) -> pikker::runtime::DraftSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let RuntimeEvent::Draft(s) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            return *s;
        }
    }
}
fn statistical_snapshot() -> pikker::meta::model::MetaSnapshot {
    use pikker::meta::model::*;
    MetaSnapshot {
        schema_version: 1,
        key: MetaKey {
            source: "OP.GG".into(),
            schema_version: 1,
            requested_rank: None,
        },
        fetched_at: 1000,
        catalog: vec![ChampionIdentity {
            id: 25,
            api_name: "MORGANA".into(),
            display_name: "Morgana".into(),
        }],
        labels: StaticLabels::default(),
        stats: vec![RoleStats {
            champion_id: 25,
            role: pikker::domain::Role::Support,
            games: Some(100),
            wins: Some(52),
            win_rate: 0.52,
            scope: SourceScope::default(),
        }],
        details: vec![],
    }
}
#[test]
fn ui_mailbox_coalesces_burst_without_losing_session_boundary() {
    use pikker::runtime::EventMailbox;
    let mailbox = EventMailbox::default();
    assert!(
        mailbox
            .push(RuntimeEvent::MetaReady(Box::new(statistical_snapshot())))
            .unwrap()
    );
    for revision in 1..=10_000 {
        let mut snapshot =
            pikker::lcu::normalize::normalize_session(&session(), 1, revision).unwrap();
        snapshot.source_epoch = 1;
        assert!(
            !mailbox
                .push(RuntimeEvent::Draft(Box::new(snapshot)))
                .unwrap()
        );
    }
    let empty = pikker::lcu::normalize::normalize_session(&Value::Null, 1, 10_001).unwrap();
    mailbox.push(RuntimeEvent::Draft(Box::new(empty))).unwrap();
    let new = pikker::lcu::normalize::normalize_session(&session(), 2, 1).unwrap();
    mailbox.push(RuntimeEvent::Draft(Box::new(new))).unwrap();
    let events = mailbox.drain();
    assert_eq!(events.len(), 4);
    assert!(matches!(&events[0], RuntimeEvent::MetaReady(_)));
    let RuntimeEvent::Draft(last) = &events[1] else {
        panic!("missing latest draft");
    };
    assert_eq!(last.revision, 10_000);
    let RuntimeEvent::Draft(deleted) = &events[2] else {
        panic!("missing clear");
    };
    assert_eq!(deleted.state.phase, pikker::domain::Phase::Waiting);
    mailbox.close();
    assert!(mailbox.push(RuntimeEvent::Error("closed".into())).is_err());
}
#[test]
fn closing_mailbox_unblocks_backpressure_producer() {
    let mailbox = Arc::new(pikker::runtime::EventMailbox::default());
    for _ in 0..64 {
        mailbox.push(RuntimeEvent::Error("source".into())).unwrap();
    }
    let worker_mailbox = mailbox.clone();
    let worker =
        std::thread::spawn(move || worker_mailbox.push(RuntimeEvent::Error("source".into())));
    mailbox.close();
    assert!(worker.join().unwrap().is_err());
}
struct SlowProvider {
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl pikker::meta::opgg::MetaProvider for SlowProvider {
    fn fetch_general(&mut self) -> Result<pikker::meta::model::MetaSnapshot, IntegrationError> {
        self.started.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(statistical_snapshot())
    }
    fn fetch_details(
        &mut self,
        _: &pikker::meta::model::ChampionIdentity,
        _: pikker::domain::Role,
        _: Option<&str>,
    ) -> Result<pikker::meta::model::ChampionDetails, IntegrationError> {
        Err(IntegrationError::Network)
    }
}
#[test]
fn active_draft_defers_bootstrap_result_without_delaying_lcu_events() {
    use pikker::{
        domain::{PlayerProfile, Role},
        runtime::{RuntimeCommand, RuntimeConfig, start_runtime},
    };
    let fake = Fake::start();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("Lobby"));
    let credentials = fake.credentials();
    let (began, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let source = Mutex::new(Some(SlowProvider {
        started: began,
        release: released,
    }));
    let cache = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::with_provider(
        cache.path().join("meta.json"),
        PlayerProfile::default(),
        Role::Support,
        None,
        Arc::new(move |_, _, _| Ok(Box::new(source.lock().unwrap().take().unwrap()))),
    );
    config.clock = Arc::new(|| 1000);
    config.lcu = Some(LcuConfig::new(
        Arc::new(move || Ok(Some(credentials.clone()))),
        LcuTrust::from_der(&fake.certificate).unwrap(),
    ));
    let (tx, rx) = mpsc::channel();
    let runtime = start_runtime(
        config,
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    )
    .unwrap();
    runtime.send(RuntimeCommand::StartBootstrap).unwrap();
    assert_eq!(next_draft(&rx).state.phase, pikker::domain::Phase::Waiting);
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route("/lol-champ-select/v1/session", 200, session());
    let before = Instant::now();
    fake.emit("/lol-champ-select/v1/session", "Create", session());
    assert_eq!(next_draft(&rx).state.phase, pikker::domain::Phase::Pick);
    assert!(before.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    loop {
        match rx.recv_timeout(Duration::from_secs(3)).unwrap() {
            RuntimeEvent::MetaReady(_) => panic!("meta changed during draft"),
            RuntimeEvent::Progress(p)
                if p.stage == pikker::meta::bootstrap::BootstrapStage::Paused =>
            {
                break;
            }
            _ => {}
        }
    }
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("Lobby"));
    fake.emit("/lol-gameflow/v1/gameflow-phase", "Update", json!("Lobby"));
    assert_eq!(next_draft(&rx).state.phase, pikker::domain::Phase::Waiting);
    let RuntimeEvent::MetaReady(data) = rx.recv_timeout(Duration::from_secs(3)).unwrap() else {
        panic!("deferred data not published");
    };
    assert_eq!(data.stats[0].champion_id, 25);
    runtime.shutdown().unwrap();
}
#[test]
fn starting_mid_draft_publishes_cache_before_draft() {
    use pikker::{
        domain::{PlayerProfile, Role},
        runtime::{RuntimeConfig, start_runtime},
    };
    let fake = Fake::start();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route("/lol-champ-select/v1/session", 200, session());
    let credentials = fake.credentials();
    let cache = tempfile::tempdir().unwrap();
    let path = cache.path().join("meta.json");
    pikker::meta::cache::save_cache(&path, &statistical_snapshot()).unwrap();
    let mut config = RuntimeConfig::with_provider(
        path,
        PlayerProfile::default(),
        Role::Support,
        None,
        Arc::new(|_, _, _| Err(IntegrationError::Network)),
    );
    config.clock = Arc::new(|| 1000);
    config.lcu = Some(LcuConfig::new(
        Arc::new(move || Ok(Some(credentials.clone()))),
        LcuTrust::from_der(&fake.certificate).unwrap(),
    ));
    let (tx, rx) = mpsc::channel();
    let runtime = start_runtime(
        config,
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    )
    .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        RuntimeEvent::MetaReady(_)
    ));
    assert_eq!(next_draft(&rx).state.phase, pikker::domain::Phase::Pick);
    runtime.shutdown().unwrap();
}
#[test]
fn disconnect_reconnect_resync_and_dodge_reject_old_payload() {
    let fake = Fake::start();
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route("/lol-champ-select/v1/session", 200, session());
    let credentials = fake.credentials();
    let config = LcuConfig::new(
        Arc::new(move || Ok(Some(credentials.clone()))),
        LcuTrust::from_der(&fake.certificate).unwrap(),
    );
    let control = LcuControl::default();
    let worker_control = control.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_listener(
            config,
            worker_control,
            BootstrapControl::default(),
            Arc::new(move |e| {
                let _ = tx.send(e);
            }),
        )
    });
    let first = next_draft(&rx);
    let mut newer = session();
    newer["bans"]["theirTeamBans"] = json!([25]);
    fake.route("/lol-champ-select/v1/session", 200, newer.clone());
    fake.disconnect_clients();
    let reconnected = next_draft(&rx);
    assert!(reconnected.generation > first.generation);
    assert!(reconnected.state.bans.contains(&25));
    // Old queued payload must not roll back the freshly fetched state.
    fake.emit("/lol-champ-select/v1/session", "Update", session());
    assert!(next_draft(&rx).state.bans.contains(&25));
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("Lobby"));
    fake.route("/lol-champ-select/v1/session", 404, Value::Null);
    fake.emit("/lol-champ-select/v1/session", "Delete", Value::Null);
    let empty = next_draft(&rx);
    assert_eq!(empty.state.phase, pikker::domain::Phase::Waiting);
    assert!(empty.state.bans.is_empty());
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route("/lol-champ-select/v1/session", 200, session());
    fake.emit("/lol-champ-select/v1/session", "Create", newer);
    let fresh = next_draft(&rx);
    assert!(fresh.generation > empty.generation);
    assert!(fresh.state.bans.is_empty());
    control.cancel();
    worker.join().unwrap();
}
fn session() -> Value {
    serde_json::from_str(include_str!("support/lcu/session.json")).unwrap()
}
#[test]
fn authenticated_get_then_websocket() {
    let fake = Fake::start();
    fake.route("/lol-champ-select/v1/session", 200, session());
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    fake.route(
        "/lol-champ-select/v1/pickable-champion-ids",
        200,
        json!([267, 412]),
    );
    let trust = LcuTrust::from_der(&fake.certificate).unwrap();
    let mut transport = LcuTransport::connect(fake.credentials(), &trust).unwrap();
    let initial = transport.read_initial_snapshot(1, 1).unwrap();
    assert_eq!(initial.local_role, Some(pikker::domain::Role::Bot));
    assert_eq!(
        initial
            .pickable_ids
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![267, 412]
    );
    fake.emit(
        "/lol-chat/v1/conversations",
        "Update",
        json!({"ignored":true}),
    );
    fake.emit("/lol-champ-select/v1/session", "Update", session());
    assert!(matches!(
        transport.next_event().unwrap(),
        Some(LcuEvent::Session(_))
    ));
    let requests = fake.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .all(|(_, auth)| auth == "Basic cmlvdDpmYWtlLXBhc3N3b3Jk")
    );
    assert!(
        requests
            .iter()
            .any(|(path, _)| path == "/lol-champ-select/v1/session")
    );
    transport.close();
}
#[test]
fn reject_non_loopback_redirect_and_untrusted_tls() {
    let fake = Fake::start();
    let trust = LcuTrust::riot().unwrap();
    assert!(LcuTransport::connect(fake.credentials(), &trust).is_err());
    let trust = LcuTrust::from_der(&fake.certificate).unwrap();
    let mut transport = LcuTransport::connect(fake.credentials(), &trust).unwrap();
    let before = fake.requests.lock().unwrap().len();
    assert!(transport.get_json("https://example.com/anything").is_err());
    assert_eq!(fake.requests.lock().unwrap().len(), before);
    fake.route("/lol-champ-select/v1/session", 302, json!(null));
    fake.route("/lol-gameflow/v1/gameflow-phase", 200, json!("ChampSelect"));
    assert!(transport.read_initial_snapshot(1, 1).is_err());
    transport.close();
}
#[test]
fn close_interrupts_idle_websocket_read() {
    let fake = Fake::start();
    let trust = LcuTrust::from_der(&fake.certificate).unwrap();
    let mut transport = LcuTransport::connect(fake.credentials(), &trust).unwrap();
    let stop = transport.interrupter();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender.send(()).unwrap();
        transport.next_event()
    });
    receiver.recv().unwrap();
    let start = std::time::Instant::now();
    stop.cancel();
    assert!(worker.join().unwrap().is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
}
#[test]
fn expired_certificate_is_rejected_even_with_dedicated_trust() {
    let fake = Fake::expired();
    let trust = LcuTrust::from_der(&fake.certificate).unwrap();
    assert!(LcuTransport::connect(fake.credentials(), &trust).is_err());
    assert!(fake.requests.lock().unwrap().is_empty());
}
#[test]
fn unknown_gameflow_shape_does_not_claim_inactive_draft() {
    let fake = Fake::start();
    fake.route(
        "/lol-gameflow/v1/gameflow-phase",
        200,
        json!({"changed": true}),
    );
    let trust = LcuTrust::from_der(&fake.certificate).unwrap();
    let mut transport = LcuTransport::connect(fake.credentials(), &trust).unwrap();
    assert!(transport.read_initial_snapshot(1, 1).is_err());
}
#[test]
fn response_size_limit_rejects_before_allocating_unbounded_body() {
    let fake = Fake::start();
    let trust = LcuTrust::from_der(&fake.certificate).unwrap();
    let mut transport = LcuTransport::connect(fake.credentials(), &trust).unwrap();
    fake.route(
        "/lol-champ-select/v1/session",
        200,
        json!("x".repeat(2 * 1024 * 1024)),
    );
    assert!(transport.get_json("/lol-champ-select/v1/session").is_err());
    transport.close();
}
#[test]
fn partial_tls_record_does_not_extend_fifteen_second_deadline() {
    use std::{
        io::Write,
        net::TcpListener,
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.write_all(&[0x16, 3, 3, 0x3f, 0xff]).unwrap();
        for _ in 0..36 {
            if socket.write_all(&[0]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    let credentials =
        pikker::lcu::discovery::parse_lockfile(&format!("LeagueClient:1:{port}:fake:https"))
            .unwrap();
    let trust = LcuTrust::riot().unwrap();
    let start = Instant::now();
    let result = LcuTransport::connect(credentials, &trust);
    let elapsed = start.elapsed();
    peer.join().unwrap();
    assert!(result.is_err());
    assert!(elapsed < Duration::from_secs(16), "{elapsed:?}");
}
