use pikker::{
    domain::*,
    meta::{bootstrap::*, model::*},
    runtime::IntegrationError,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn snapshot() -> MetaSnapshot {
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
            role: Role::Support,
            games: Some(10),
            wins: Some(5),
            win_rate: 0.5,
            scope: SourceScope::default(),
        }],
        details: vec![],
    }
}

#[test]
fn top_ten_each_role_plus_favorites_are_deduplicated_and_current_role_prioritized() {
    let mut stats = Vec::new();
    for role in Role::ALL {
        for id in 1..=15 {
            stats.push(RoleStats {
                champion_id: id,
                role,
                games: Some(100),
                wins: Some(50),
                win_rate: 0.5,
                scope: SourceScope::default(),
            });
        }
    }
    let mut profile = PlayerProfile::default();
    profile.preferences.insert(
        (15, Role::Support),
        Preference {
            favorite: true,
            proficiency: None,
        },
    );
    profile.preferences.insert(
        (1, Role::Top),
        Preference {
            favorite: true,
            proficiency: None,
        },
    );
    let requests = plan_details(&stats, &profile, Role::Support);
    assert_eq!(requests.len(), 51);
    assert_eq!(
        requests[0],
        DetailRequest {
            champion_id: 15,
            role: Role::Support
        }
    );
    assert_eq!(
        requests[1],
        DetailRequest {
            champion_id: 1,
            role: Role::Support
        }
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.champion_id == 1 && r.role == Role::Top)
            .count(),
        1
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.champion_id == 11 && r.role == Role::Mid)
    );
}

#[test]
fn sixty_attempt_budget_is_enforced() {
    let mut budget = AttemptBudget::default();
    for _ in 0..60 {
        assert!(budget.begin().is_ok());
    }
    assert!(budget.begin().is_err());
    assert_eq!(budget.attempts(), 60);
}

struct NoWait;
impl BootstrapWaiter for NoWait {
    fn wait(&self, _delay: Duration, control: &BootstrapControl) -> Result<(), IntegrationError> {
        control.checkpoint()
    }
}

// The final failed query must not discard a provider-wide Retry-After.
#[test]
fn exhausted_429_respects_cooldown_before_next_champion() {
    use std::sync::Mutex;
    struct TraceWaiter(Arc<Mutex<Vec<u64>>>);
    impl BootstrapWaiter for TraceWaiter {
        fn wait(&self, delay: Duration, c: &BootstrapControl) -> Result<(), IntegrationError> {
            self.0.lock().unwrap().push(delay.as_secs());
            c.checkpoint()
        }
    }
    struct Limited(Arc<Mutex<Vec<u64>>>);
    impl pikker::meta::opgg::MetaProvider for Limited {
        fn fetch_general(&mut self) -> Result<MetaSnapshot, IntegrationError> {
            unreachable!("fresh cache")
        }
        fn fetch_details(
            &mut self,
            c: &ChampionIdentity,
            role: Role,
            rank: Option<&str>,
        ) -> Result<ChampionDetails, IntegrationError> {
            self.0.lock().unwrap().push(100 + u64::from(c.id));
            if c.id == 25 {
                return Err(IntegrationError::Http {
                    status: 429,
                    retry_after: Some(Duration::from_secs(17)),
                });
            }
            Ok(ChampionDetails {
                champion_id: c.id,
                role,
                requested_rank: rank.map(str::to_owned),
                scope: SourceScope::default(),
                runes: vec![],
                spells: vec![],
                counters: vec![],
                synergies: vec![],
                fetched_at: 1000,
            })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut data = snapshot();
    data.catalog.push(ChampionIdentity {
        id: 26,
        api_name: "ZILEAN".into(),
        display_name: "Zilean".into(),
    });
    let mut stats = data.stats[0].clone();
    stats.champion_id = 26;
    data.stats.push(stats);
    pikker::meta::cache::save_cache(&path, &data).unwrap();
    let trace = Arc::new(Mutex::new(vec![]));
    let captured = trace.clone();
    let mut config = pikker::runtime::RuntimeConfig::with_provider(
        path,
        PlayerProfile::default(),
        Role::Support,
        None,
        Arc::new(move |_, _, _| Ok(Box::new(Limited(captured.clone())))),
    );
    config.clock = Arc::new(|| 1001);
    config.waiter = Arc::new(TraceWaiter(trace.clone()));
    let sender: pikker::runtime::EventSender = Arc::new(|_| {});
    run_bootstrap(&config, &BootstrapControl::default(), &sender, false).unwrap();
    assert_eq!(
        *trace.lock().unwrap(),
        vec![125, 17, 1, 125, 17, 1, 125, 17, 1, 126]
    );
}
struct Provider {
    data: MetaSnapshot,
    general: Arc<std::sync::atomic::AtomicUsize>,
    details: Arc<std::sync::atomic::AtomicUsize>,
    fail: bool,
}
impl pikker::meta::opgg::MetaProvider for Provider {
    fn fetch_general(&mut self) -> Result<MetaSnapshot, IntegrationError> {
        self.general
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail {
            Err(IntegrationError::Network)
        } else {
            Ok(self.data.clone())
        }
    }
    fn fetch_details(
        &mut self,
        c: &ChampionIdentity,
        role: Role,
        rank: Option<&str>,
    ) -> Result<ChampionDetails, IntegrationError> {
        self.details
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ChampionDetails {
            champion_id: c.id,
            role,
            requested_rank: rank.map(str::to_owned),
            scope: SourceScope::default(),
            runes: vec![],
            spells: vec![],
            counters: vec![],
            synergies: vec![],
            fetched_at: 1000,
        })
    }
}
fn config(
    path: std::path::PathBuf,
    data: MetaSnapshot,
    general: Arc<std::sync::atomic::AtomicUsize>,
    details: Arc<std::sync::atomic::AtomicUsize>,
    fail: bool,
) -> pikker::runtime::RuntimeConfig {
    let factory: ProviderFactory = Arc::new(move |_, _, _| {
        Ok(Box::new(Provider {
            data: data.clone(),
            general: general.clone(),
            details: details.clone(),
            fail,
        }))
    });
    let mut config = pikker::runtime::RuntimeConfig::with_provider(
        path,
        PlayerProfile::default(),
        Role::Support,
        None,
        factory,
    );
    config.clock = Arc::new(|| 1001);
    config.waiter = Arc::new(NoWait);
    config
}

#[test]
fn fresh_cache_skips_meta_fetch_and_fetches_only_missing_details() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let data = snapshot();
    pikker::meta::cache::save_cache(&path, &data).unwrap();
    let general = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let details = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let config = config(path.clone(), data, general.clone(), details.clone(), false);
    let events = Arc::new(std::sync::Mutex::new(vec![]));
    let capture = events.clone();
    let sender: pikker::runtime::EventSender =
        Arc::new(move |event| capture.lock().unwrap().push(event));
    run_bootstrap(&config, &BootstrapControl::default(), &sender, false).unwrap();
    assert_eq!(general.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(details.load(std::sync::atomic::Ordering::SeqCst), 1);
    run_bootstrap(&config, &BootstrapControl::default(), &sender, false).unwrap();
    assert_eq!(details.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(events.lock().unwrap().iter().any(
        |e| matches!(e,pikker::runtime::RuntimeEvent::Progress(p) if p.stage==BootstrapStage::Ready)
    ));
}

#[test]
fn partial_refresh_keeps_valid_cache() {
    let sender: pikker::runtime::EventSender = Arc::new(|_| {});
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let data = snapshot();
    pikker::meta::cache::save_cache(&path, &data).unwrap();
    let previous = std::fs::read(&path).unwrap();
    let general = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let config = config(
        path.clone(),
        data,
        general.clone(),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        true,
    );
    assert!(run_bootstrap(&config, &BootstrapControl::default(), &sender, true).is_err());
    assert_eq!(general.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(std::fs::read(&path).unwrap(), previous);
}

#[test]
fn paused_bootstrap_makes_no_provider_calls() {
    let sender: pikker::runtime::EventSender = Arc::new(|_| {});
    let dir = tempfile::tempdir().unwrap();
    let general = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let details = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let config = config(
        dir.path().join("meta.json"),
        snapshot(),
        general.clone(),
        details.clone(),
        false,
    );
    let control = BootstrapControl::default();
    control.set_draft_active(true, &|_| {});
    assert!(matches!(
        run_bootstrap(&config, &control, &sender, false),
        Err(IntegrationError::Paused)
    ));
    assert_eq!(general.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(details.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn complete_bootstrap_enforces_60_attempts_and_exposes_remaining_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = snapshot();
    data.catalog.clear();
    data.stats.clear();
    for id in 1..=70 {
        data.catalog.push(ChampionIdentity {
            id,
            api_name: format!("C{id}"),
            display_name: format!("C{id}"),
        });
        data.stats.push(RoleStats {
            champion_id: id,
            role: Role::Support,
            games: Some(10),
            wins: Some(5),
            win_rate: 0.5,
            scope: SourceScope::default(),
        });
    }
    let general = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let details = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut config = config(
        dir.path().join("meta.json"),
        data,
        general.clone(),
        details.clone(),
        false,
    );
    for id in 1..=70 {
        config.profile.preferences.insert(
            (id, Role::Support),
            Preference {
                favorite: true,
                proficiency: None,
            },
        );
    }
    let events = Arc::new(std::sync::Mutex::new(vec![]));
    let capture = events.clone();
    let sender: pikker::runtime::EventSender =
        Arc::new(move |event| capture.lock().unwrap().push(event));
    let data = run_bootstrap(&config, &BootstrapControl::default(), &sender, false).unwrap();
    assert_eq!(general.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(details.load(std::sync::atomic::Ordering::SeqCst), 59);
    assert_eq!(data.details.len(), 59);
    assert!(events.lock().unwrap().iter().any(|e|matches!(e,pikker::runtime::RuntimeEvent::Progress(p) if p.stage==BootstrapStage::Partial&&p.completed==59&&p.total==Some(70))));
}

#[test]
fn one_request_in_flight_and_draft_defers_inflight_result() {
    use pikker::runtime::*;
    let dir = tempfile::tempdir().unwrap();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    struct BlockingProvider {
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    }
    impl pikker::meta::opgg::MetaProvider for BlockingProvider {
        fn fetch_general(&mut self) -> Result<MetaSnapshot, IntegrationError> {
            self.entered.wait();
            self.release.wait();
            Ok(snapshot())
        }
        fn fetch_details(
            &mut self,
            _: &ChampionIdentity,
            _: Role,
            _: Option<&str>,
        ) -> Result<ChampionDetails, IntegrationError> {
            panic!("no detail request may start during draft")
        }
    }
    let worker_entered = entered.clone();
    let worker_release = release.clone();
    let factory: ProviderFactory = Arc::new(move |_, _, _| {
        Ok(Box::new(BlockingProvider {
            entered: worker_entered.clone(),
            release: worker_release.clone(),
        }))
    });
    let mut config = RuntimeConfig::with_provider(
        dir.path().join("meta.json"),
        PlayerProfile::default(),
        Role::Support,
        None,
        factory,
    );
    config.clock = Arc::new(|| 1001);
    config.waiter = Arc::new(NoWait);
    let events = Arc::new(std::sync::Mutex::new(vec![]));
    let capture = events.clone();
    let handle = start_runtime(config, Arc::new(move |e| capture.lock().unwrap().push(e))).unwrap();
    handle.send(RuntimeCommand::StartBootstrap).unwrap();
    entered.wait();
    assert!(handle.send(RuntimeCommand::Refresh).is_err());
    handle.send(RuntimeCommand::SetDraftActive(true)).unwrap();
    release.wait();
    // Wait for the actual terminal Paused event, not an arbitrary sleep.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e,RuntimeEvent::Progress(p) if p.stage==BootstrapStage::Paused))
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, RuntimeEvent::MetaReady(_)))
    );
    handle.send(RuntimeCommand::SetDraftActive(false)).unwrap();
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::MetaReady(_)))
            .count(),
        1
    );
    handle.shutdown().unwrap();
}

#[test]
fn retry_after_and_three_attempt_limit() {
    let limited = IntegrationError::Http {
        status: 429,
        retry_after: Some(Duration::from_secs(17)),
    };
    assert_eq!(retry_delay(&limited, 1), Some(Duration::from_secs(17)));
    assert_eq!(
        retry_delay(&IntegrationError::Network, 1),
        Some(Duration::from_secs(5))
    );
    assert_eq!(
        retry_delay(&IntegrationError::Network, 2),
        Some(Duration::from_secs(15))
    );
    assert_eq!(retry_delay(&IntegrationError::Network, 3), None);
    assert_eq!(
        retry_delay(
            &IntegrationError::Http {
                status: 400,
                retry_after: None
            },
            1
        ),
        None
    );
}

#[test]
fn draft_pauses_pending_network_and_new_meta_is_deferred_until_next_draft() {
    let control = BootstrapControl::default();
    let published = Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = published.clone();
    let sender = move |snapshot: MetaSnapshot| capture.lock().unwrap().push(snapshot.fetched_at);
    control.set_draft_active(true, &sender);
    assert!(matches!(
        control.checkpoint(),
        Err(IntegrationError::Paused)
    ));
    control.publish(snapshot(), &sender);
    assert!(published.lock().unwrap().is_empty());
    control.set_draft_active(false, &sender);
    assert_eq!(*published.lock().unwrap(), vec![1000]);
    assert!(control.checkpoint().is_ok());
}

#[test]
fn close_during_backoff_exits_without_waiting_for_timeout() {
    let control = BootstrapControl::default();
    let worker_control = control.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender.send(()).unwrap();
        worker_control.wait(Duration::from_secs(100))
    });
    receiver.recv().unwrap();
    let start = Instant::now();
    control.cancel();
    assert!(matches!(
        worker.join().unwrap(),
        Err(IntegrationError::Cancelled)
    ));
    assert!(start.elapsed() < Duration::from_secs(2));
}
