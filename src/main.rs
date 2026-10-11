#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use pikker::{
    catalog::demo_catalog,
    controller::{Controller, SourceMode},
    domain::*,
    meta::bootstrap::BootstrapStage,
    profile::*,
    ranking::*,
    runtime::*,
    settings::*,
    simulator::*,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::sync::{Arc, Mutex};
slint::include_modules!();
#[cfg(all(test, feature = "gui-test"))]
#[allow(dead_code)]
#[path = "../tests/support/lcu_fake.rs"]
mod lcu_fake;

#[cfg(all(test, feature = "gui-test"))]
mod ui_tests {
    use super::*;
    use pikker::meta::model::*;
    use slint::Model;
    #[test]
    fn about_can_be_opened_and_closed_without_changing_the_draft() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        ui.set_draft_active(true);
        ui.set_role_index(3);
        assert!(!ui.get_about_visible());
        ui.invoke_show_about();
        assert!(ui.get_about_visible());
        assert!(ui.get_legal_notice().contains("Riot Games"));
        ui.invoke_close_about();
        assert!(!ui.get_about_visible());
        assert!(ui.get_draft_active());
        assert_eq!(ui.get_role_index(), 3);
    }
    #[test]
    #[ignore = "release profiling against externally hosted fake; requires display and test paths"]
    fn native_external_fake_idle_profile() {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()
            .unwrap();
        let lockfile = std::path::PathBuf::from(
            std::env::var_os("PIKKER_TEST_LOCKFILE").expect("test lockfile"),
        );
        let certificate =
            std::fs::read(std::env::var_os("PIKKER_TEST_CERT").expect("test certificate")).unwrap();
        let cache =
            std::path::PathBuf::from(std::env::var_os("PIKKER_TEST_CACHE").expect("test cache"));
        let ui = AppWindow::new().unwrap();
        let c = Arc::new(Mutex::new(Controller::new(
            demo_catalog(),
            PlayerProfile::default(),
        )));
        c.lock()
            .unwrap()
            .set_source_mode(SourceMode::Connected)
            .unwrap();
        let target = Arc::new(Mutex::new(std::sync::Weak::new()));
        let mailbox = Arc::new(EventMailbox::default());
        let sender = ui_event_sender(&ui, c.clone(), target.clone(), mailbox.clone());
        let mut config = RuntimeConfig::with_provider(
            cache,
            PlayerProfile::default(),
            Role::Support,
            Some("gold".into()),
            Arc::new(|_, _, _| Err(IntegrationError::Network)),
        );
        config.lcu = Some(pikker::lcu::listener::LcuConfig::new(
            Arc::new(move || pikker::lcu::discovery::discover(Some(&lockfile))),
            pikker::lcu::transport::LcuTrust::from_der(&certificate).unwrap(),
        ));
        let runtime = Arc::new(start_runtime(config, sender).unwrap());
        *target.lock().unwrap() = Arc::downgrade(&runtime);
        runtime.send(RuntimeCommand::StartBootstrap).unwrap();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(70),
            || slint::quit_event_loop().unwrap(),
        );
        println!("PROFILE_PID={}", std::process::id());
        ui.run().unwrap();
        mailbox.close();
        runtime.shutdown().unwrap();
        assert_eq!(c.lock().unwrap().draft.role, Role::Bot);
        assert!(ui.get_connection_status().contains("somente leitura"));
        assert!(ui.get_draft_active());
    }
    #[test]
    #[ignore = "requires a native display; run separately under Xvfb"]
    fn native_fake_draft_smoke() {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()
            .unwrap();
        let fake = Arc::new(crate::lcu_fake::Fake::start());
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../tests/support/lcu/session.json")).unwrap();
        fake.route(
            "/lol-gameflow/v1/gameflow-phase",
            200,
            serde_json::json!("ChampSelect"),
        );
        fake.route("/lol-champ-select/v1/session", 200, raw.clone());
        let ui = AppWindow::new().unwrap();
        let c = Arc::new(Mutex::new(Controller::new(
            demo_catalog(),
            PlayerProfile::default(),
        )));
        c.lock()
            .unwrap()
            .set_source_mode(SourceMode::Connected)
            .unwrap();
        let target = Arc::new(Mutex::new(std::sync::Weak::new()));
        let mailbox = Arc::new(EventMailbox::default());
        let sender = ui_event_sender(&ui, c.clone(), target.clone(), mailbox.clone());
        let cache = tempfile::tempdir().unwrap();
        let mut config = RuntimeConfig::with_provider(
            cache.path().join("meta.json"),
            PlayerProfile::default(),
            Role::Support,
            None,
            Arc::new(|_, _, _| Err(IntegrationError::Network)),
        );
        let credentials = fake.credentials();
        config.lcu = Some(pikker::lcu::listener::LcuConfig::new(
            Arc::new(move || Ok(Some(credentials.clone()))),
            pikker::lcu::transport::LcuTrust::from_der(&fake.certificate).unwrap(),
        ));
        let runtime = Arc::new(start_runtime(config, sender).unwrap());
        *target.lock().unwrap() = Arc::downgrade(&runtime);
        runtime.send(RuntimeCommand::StartBootstrap).unwrap();
        let stage = std::rc::Rc::new(std::cell::Cell::new(0));
        let timer_stage = stage.clone();
        let w = ui.as_weak();
        let started = std::time::Instant::now();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                let ui = w.upgrade().unwrap();
                if started.elapsed() > std::time::Duration::from_secs(5) {
                    slint::quit_event_loop().unwrap();
                    return;
                }
                match timer_stage.get() {
                    0 if ui.get_draft_active() => {
                        assert_eq!(ui.get_role_index(), 3);
                        let mut update = raw.clone();
                        update["bans"]["theirTeamBans"] = serde_json::json!([25]);
                        update["myTeam"][3]["assignedPosition"] = serde_json::json!("utility");
                        update["theirTeam"][0]["championPickIntent"] = serde_json::json!(55);
                        fake.route("/lol-champ-select/v1/session", 200, update.clone());
                        fake.emit("/lol-champ-select/v1/session", "Update", update);
                        timer_stage.set(1);
                    }
                    1 if ui.get_bans().contains("#25") => {
                        assert_eq!(ui.get_role_index(), 4);
                        assert!(ui.get_local_role_known());
                        assert!(
                            c.lock()
                                .unwrap()
                                .draft
                                .enemies
                                .iter()
                                .all(|p| p.hover.is_none())
                        );
                        fake.route(
                            "/lol-gameflow/v1/gameflow-phase",
                            200,
                            serde_json::json!("Lobby"),
                        );
                        fake.emit(
                            "/lol-champ-select/v1/session",
                            "Delete",
                            serde_json::Value::Null,
                        );
                        timer_stage.set(2);
                    }
                    2 if !ui.get_draft_active() => {
                        assert!(ui.get_bans().is_empty() || !ui.get_bans().contains("#25"));
                        timer_stage.set(3);
                        slint::quit_event_loop().unwrap();
                    }
                    _ => {}
                }
            },
        );
        ui.run().unwrap();
        mailbox.close();
        runtime.shutdown().unwrap();
        assert_eq!(stage.get(), 3, "native fake did not finish within deadline");
    }
    #[test]
    fn draft_event_updates_native_ui_and_rejects_old_source_epoch() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        let c = Arc::new(Mutex::new(Controller::new(
            demo_catalog(),
            PlayerProfile::default(),
        )));
        c.lock()
            .unwrap()
            .set_source_mode(SourceMode::Connected)
            .unwrap();
        let path = tempfile::tempdir().unwrap();
        let runtime = Arc::new(
            start_runtime(
                RuntimeConfig::with_provider(
                    path.path().join("meta.json"),
                    PlayerProfile::default(),
                    Role::Support,
                    None,
                    Arc::new(|_, _, _| Err(IntegrationError::Network)),
                ),
                Arc::new(|_| {}),
            )
            .unwrap(),
        );
        let target = Arc::downgrade(&runtime);
        let raw = serde_json::from_str(include_str!("../tests/support/lcu/session.json")).unwrap();
        let mut snapshot = pikker::lcu::normalize::normalize_session(&raw, 1, 1).unwrap();
        snapshot.source_epoch = runtime.source_epoch();
        apply_runtime_event(
            &ui,
            &c,
            &target,
            RuntimeEvent::Draft(Box::new(snapshot.clone())),
        );
        assert_eq!(ui.get_role_index(), 3);
        assert!(ui.get_draft_active());
        assert_eq!(c.lock().unwrap().draft.role, Role::Bot);
        runtime.send(RuntimeCommand::SetAutomatic(false)).unwrap();
        runtime.send(RuntimeCommand::SetAutomatic(true)).unwrap();
        snapshot.generation = 2;
        snapshot.local_role = Some(Role::Top);
        apply_runtime_event(&ui, &c, &target, RuntimeEvent::Draft(Box::new(snapshot)));
        assert_eq!(c.lock().unwrap().draft.role, Role::Bot);
    }
    #[test]
    fn connected_without_meta_displays_confirmed_ids_not_empty_slots() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        let mut c = Controller::new(demo_catalog(), PlayerProfile::default());
        c.set_source_mode(SourceMode::Connected).unwrap();
        c.draft.phase = Phase::Pick;
        c.draft.allies[0].champion = Some(267);
        c.draft.allies[1].hover = Some(412);
        c.draft.bans.insert(25);
        refresh(&ui, &c);
        assert!(ui.get_allies().row_data(0).unwrap().title.contains("#267"));
        assert!(ui.get_allies().row_data(1).unwrap().detail.contains("#412"));
        assert!(ui.get_bans().contains("#25"));
        assert_eq!(ui.get_ranking().row_count(), 0);
    }
    #[test]
    fn actual_event_consumer_preserves_cache_and_displays_progress_and_offline_errors() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        let c = Arc::new(Mutex::new(Controller::new(
            demo_catalog(),
            PlayerProfile::default(),
        )));
        c.lock()
            .unwrap()
            .set_source_mode(SourceMode::Connected)
            .unwrap();
        let target = std::sync::Weak::new();
        refresh(&ui, &c.lock().unwrap());
        assert!(ui.get_data_status().contains("Sem dados estatísticos"));
        let data = MetaSnapshot {
            schema_version: 1,
            key: MetaKey {
                source: "OP.GG".into(),
                schema_version: 1,
                requested_rank: None,
            },
            fetched_at: 1000,
            catalog: vec![ChampionIdentity {
                id: 201,
                api_name: "BRAUM".into(),
                display_name: "Braum".into(),
            }],
            labels: StaticLabels::default(),
            stats: vec![RoleStats {
                champion_id: 201,
                role: Role::Support,
                games: Some(1000),
                wins: Some(520),
                win_rate: 0.52,
                scope: SourceScope::default(),
            }],
            details: vec![ChampionDetails {
                champion_id: 201,
                role: Role::Support,
                requested_rank: None,
                scope: SourceScope::default(),
                fetched_at: 1000,
                spells: vec![BuildChoice {
                    ids: vec![4, 14],
                    names: vec!["Flash".into(), "Incendiar".into()],
                    games: Some(100),
                    wins: Some(52),
                }],
                runes: vec![],
                counters: vec![],
                synergies: vec![],
            }],
        };
        apply_runtime_event(&ui, &c, &target, RuntimeEvent::MetaReady(Box::new(data)));
        assert!(ui.get_data_status().contains("cache vencido"));
        assert!(ui.get_details_status().contains("Flash · Incendiar"));
        assert!(ui.get_details_status().contains("100 partidas"));
        apply_runtime_event(
            &ui,
            &c,
            &target,
            RuntimeEvent::Progress(pikker::meta::bootstrap::BootstrapProgress {
                stage: BootstrapStage::Details,
                completed: 18,
                total: Some(50),
            }),
        );
        assert_eq!(ui.get_bootstrap_completed(), 18);
        assert_eq!(ui.get_bootstrap_total(), 50);
        assert!(ui.get_bootstrap_busy());
        apply_runtime_event(
            &ui,
            &c,
            &target,
            RuntimeEvent::Error("Sem conexão para atualizar".into()),
        );
        apply_runtime_event(
            &ui,
            &c,
            &target,
            RuntimeEvent::Progress(pikker::meta::bootstrap::BootstrapProgress {
                stage: BootstrapStage::Partial,
                completed: 18,
                total: Some(50),
            }),
        );
        assert!(!ui.get_bootstrap_busy());
        assert!(ui.get_data_error().contains("Sem conexão"));
        assert!(ui.get_details_status().contains("Flash"));
        assert!(c.lock().unwrap().meta.is_some());
        apply_runtime_event(
            &ui,
            &c,
            &target,
            RuntimeEvent::Progress(pikker::meta::bootstrap::BootstrapProgress {
                stage: BootstrapStage::Ready,
                completed: 50,
                total: Some(50),
            }),
        );
        assert_eq!(ui.get_bootstrap_completed(), 50);
        assert!(!ui.get_bootstrap_busy());
    }
}

fn names(c: &Controller) -> Vec<ChampionId> {
    let mut ids: Vec<_> = c.catalog.keys().copied().collect();
    ids.sort_by(|a, b| c.catalog[a].name.cmp(&c.catalog[b].name));
    ids
}
fn champion_name(c: &Controller, id: ChampionId) -> String {
    c.catalog
        .get(&id)
        .map(|champion| champion.name.clone())
        .unwrap_or_else(|| format!("#{id}"))
}
fn selected_id(ui: &AppWindow, c: &Controller) -> Option<ChampionId> {
    names(c).get(ui.get_selected_champion() as usize).copied()
}
fn preference_to_ui(ui: &AppWindow, c: &Controller) {
    let p = selected_id(ui, c)
        .and_then(|id| c.profile.preferences.get(&(id, c.draft.role)))
        .cloned()
        .unwrap_or_default();
    ui.set_favorite(p.favorite);
    ui.set_proficiency_index(proficiency_level(&p) as i32);
    ui.set_proficiency_edited(false);
    ui.set_proficiency_detail(
        p.proficiency
            .map(|v| format!("Domínio salvo: {:.1}% (autoavaliação)", v * 100.0))
            .unwrap_or("Domínio salvo: desconhecido".into())
            .into(),
    );
}
fn refresh(ui: &AppWindow, c: &Controller) {
    ui.set_source_mode_index(if c.source_mode == SourceMode::Connected {
        1
    } else {
        0
    });
    ui.set_draft_active(matches!(
        c.draft.phase,
        Phase::Ban | Phase::Pick | Phase::Finalization
    ));
    if let Some(meta) = &c.meta {
        let timestamp = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(meta.fetched_at.max(0) as u64))
            .map(httpdate::fmt_http_date)
            .unwrap_or_else(|| "desconhecido".into());
        let stale = pikker::meta::opgg::now_seconds().saturating_sub(meta.fetched_at) >= 86400;
        ui.set_data_status(format!("OP.GG · {} combinações · {} detalhes · coleta {}{} · filtros gerais não informados; detalhes solicitados: {}",
            meta.stats.len(),meta.details.len(),timestamp,if stale{" · cache vencido"}else{""},meta.key.requested_rank.as_deref().unwrap_or("todos os elos")).into());
    } else {
        ui.set_data_status(
            if c.source_mode == SourceMode::Simulator {
                "Simulador com dados demonstrativos; não são estatísticas do patch"
            } else {
                "Sem dados estatísticos · aguarde o bootstrap antes de entrar na fila"
            }
            .into(),
        );
    }
    ui.set_partner_preset_label(
        match c.draft.role {
            Role::Bot => "Suporte aliado conhecido",
            Role::Support => "ADC aliado conhecido",
            _ => "Adversário de rota conhecido",
        }
        .into(),
    );
    ui.set_role_index(Role::ALL.iter().position(|r| *r == c.draft.role).unwrap() as i32);
    ui.set_phase_index(match c.draft.phase {
        Phase::Waiting => 0,
        Phase::Ban => 1,
        Phase::Pick => 2,
        Phase::Finalization => 3,
        Phase::Ended => 4,
    });
    ui.set_status(
        format!(
            "Sessão {} · revisão {} · {} análises",
            c.draft.session_id, c.draft.revision, c.computations
        )
        .into(),
    );
    ui.set_bans(if c.draft.bans.is_empty() {
        "nenhum".into()
    } else {
        c.draft
            .bans
            .iter()
            .map(|id| champion_name(c, *id))
            .collect::<Vec<_>>()
            .join(", ")
            .into()
    });
    for (enemy, team) in [(false, &c.draft.allies), (true, &c.draft.enemies)] {
        let rows: Vec<SlotRow> = team
            .iter()
            .enumerate()
            .map(|(i, s)| SlotRow {
                title: format!(
                    "{} · {}",
                    i + 1,
                    s.champion
                        .map(|id| champion_name(c, id))
                        .unwrap_or_else(|| "—".into())
                )
                .into(),
                detail: format!(
                    "{}{}",
                    s.role.map(Role::label).unwrap_or("?"),
                    s.hover
                        .map(|id| format!(" · hover {}", champion_name(c, id)))
                        .unwrap_or_default()
                )
                .into(),
            })
            .collect();
        let model = ModelRc::new(VecModel::from(rows));
        if enemy {
            ui.set_enemies(model);
        } else {
            ui.set_allies(model);
        }
    }
    let rows:Vec<RankRow>=c.rows.iter().map(|r| {
        let mut tags=vec![if r.favorite{"Favorito"}else{"Fora da pool"}];
        if r.learning_notice { tags.push("Aprendizado / execução a avaliar"); }
        if r.insufficient { tags.push("Evidência insuficiente nesta rota"); }else if r.statistical {tags.push("Estatística OP.GG");}else{tags.push("Avaliação demonstrativa");}
        if r.coverage_limited&&!r.insufficient {tags.push("Mecânica/composição parcialmente não avaliadas");}
        let mut reasons=r.contributions.iter().map(|v|format!("{}: +{:.1}",v.reason,v.value*100.0)).collect::<Vec<_>>();
        if let Some(games)=r.games {reasons.push(format!("Amostra: {games} partidas"));}
        if let Some(source)=&r.provenance {reasons.push(source.clone());}
        if !r.insufficient&&r.score.is_none() {reasons.push("Sem pesos aplicáveis: ajuste os pesos disponíveis.".into());}
        RankRow {id:r.id as i32, name:c.catalog[&r.id].name.clone().into(),score:r.score.map(|s|format!("{:.1} pontos",s*100.0)).unwrap_or("Sem escore".into()).into(),tags:tags.join(" · ").into(),
            reasons:if r.insufficient{"Não há evidência suficiente para avaliar competitivamente nesta rota. Favoritar não resolve essa ausência.".into()}else{reasons.join("\n").into()},insufficient:r.insufficient }
    }).collect();
    ui.set_ranking(ModelRc::new(VecModel::from(rows)));
    let favorites = c
        .profile
        .preferences
        .iter()
        .filter(|((_, role), p)| *role == c.draft.role && p.favorite)
        .filter_map(|((id, _), _)| c.catalog.get(id).map(|x| x.name.clone()))
        .collect::<Vec<_>>();
    ui.set_pool_summary(
        format!(
            "Favoritos em {}: {}",
            c.draft.role.label(),
            if favorites.is_empty() {
                "nenhum".into()
            } else {
                favorites.join(", ")
            }
        )
        .into(),
    );
    details_to_ui(ui, c);
}
fn details_to_ui(ui: &AppWindow, c: &Controller) {
    let Some(id) = selected_id(ui, c) else {
        ui.set_details_status("Selecione um candidato.".into());
        return;
    };
    let name = c
        .catalog
        .get(&id)
        .map(|c| c.name.as_str())
        .unwrap_or("Campeão");
    let Some(details) = c.selected_details(id, c.draft.role) else {
        ui.set_details_status(format!("{name} · {}\nDetalhes não disponíveis neste cache. Nenhuma consulta é feita durante o draft.",c.draft.role.label()).into());
        return;
    };
    let mut lines = vec![
        format!("{name} · {}", c.draft.role.label()),
        format!(
            "OP.GG · elo solicitado {} (população não comprovada)",
            details.requested_rank.as_deref().unwrap_or("todos")
        ),
    ];
    for (title, builds) in [("Feitiços", &details.spells), ("Runas", &details.runes)] {
        if builds.is_empty() {
            lines.push(format!("{title}: indisponíveis"));
        }
        for build in builds {
            let names = if build.names.len() == build.ids.len() {
                build.names.join(" · ")
            } else {
                build
                    .ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            lines.push(format!("{title}: {names}"));
            if let Some(games) = build.games {
                lines.push(format!("Amostra: {games} partidas"));
            }
        }
    }
    if !details.counters.is_empty() {
        lines.push("Adversários com evidência de matchup:".into());
        for row in &details.counters {
            let name = c
                .catalog
                .get(&row.champion_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("#{}", row.champion_id));
            lines.push(format!(
                "{name} · {} partidas",
                row.games
                    .map(|g| g.to_string())
                    .unwrap_or_else(|| "amostra desconhecida".into())
            ));
        }
    }
    lines.push(format!(
        "Sinergias em cache: {} · patch/região dos detalhes não comprovados",
        details.synergies.len()
    ));
    ui.set_details_status(lines.join("\n\n").into());
}
fn update_names(ui: &AppWindow, c: &Controller, previous: Option<ChampionId>) {
    let ids = names(c);
    ui.set_champions(ModelRc::new(VecModel::from(
        ids.iter()
            .map(|id| c.catalog[id].name.clone().into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    ui.set_selected_champion(
        previous
            .and_then(|id| ids.iter().position(|i| *i == id))
            .unwrap_or(0) as i32,
    );
    preference_to_ui(ui, c);
}
fn selected_rank(ui: &AppWindow) -> Option<String> {
    RANKS
        .get(ui.get_rank_index().wrapping_sub(1) as usize)
        .map(|s| (*s).to_owned())
}
fn sync_runtime(ui: &AppWindow, c: &Controller, runtime: &RuntimeHandle) {
    if c.source_mode == SourceMode::Simulator {
        let _ = runtime.send(RuntimeCommand::SetDraftActive(matches!(
            c.draft.phase,
            Phase::Ban | Phase::Pick | Phase::Finalization
        )));
    }
    let _ = runtime.send(RuntimeCommand::UpdatePreferences(
        c.profile.clone(),
        c.draft.role,
        selected_rank(ui),
    ));
}
fn dispatch(ui: &AppWindow, c: &mut Controller, action: Action, runtime: &RuntimeHandle) {
    let cmd = DraftCommand {
        session_id: c.draft.session_id,
        revision: c.draft.revision + 1,
        action,
    };
    match c.dispatch(cmd) {
        Ok(()) => ui.set_error("".into()),
        Err(e) => ui.set_error(e.into()),
    }
    refresh(ui, c);
    sync_runtime(ui, c, runtime);
}
fn apply_runtime_event(
    ui: &AppWindow,
    c: &Arc<Mutex<Controller>>,
    target: &std::sync::Weak<RuntimeHandle>,
    event: RuntimeEvent,
) {
    match event {
        RuntimeEvent::Progress(p) => {
            let label = match p.stage {
                BootstrapStage::LocalCache => "Carregando dados locais",
                BootstrapStage::Catalog => "Verificando catálogo",
                BootstrapStage::GeneralMeta => "Atualizando meta",
                BootstrapStage::Details => "Preparando runas e feitiços",
                BootstrapStage::Ready => "Pronto",
                BootstrapStage::Partial => "Dados parciais",
                BootstrapStage::Paused => "Atualização pausada durante o draft",
            };
            ui.set_bootstrap_stage(label.into());
            ui.set_bootstrap_completed(p.completed as i32);
            ui.set_bootstrap_total(p.total.unwrap_or(0) as i32);
            ui.set_bootstrap_total_known(p.total.is_some());
            ui.set_bootstrap_busy(!matches!(
                p.stage,
                BootstrapStage::Ready | BootstrapStage::Partial | BootstrapStage::Paused
            ));
        }
        RuntimeEvent::MetaReady(snapshot) => {
            let mut c = c.lock().unwrap();
            let old_ids = names(&c);
            let previous = selected_id(ui, &c);
            if let Err(error) = c.set_meta(*snapshot) {
                ui.set_data_error(error.into());
            }
            if names(&c) != old_ids {
                update_names(ui, &c, previous);
            }
            refresh(ui, &c);
        }
        RuntimeEvent::Connection(status) => ui.set_connection_status(
            match status {
                ConnectionStatus::Absent => "Cliente LoL ausente",
                ConnectionStatus::Connecting => "Conectando ao LoL",
                ConnectionStatus::Connected => "Cliente LoL conectado (somente leitura)",
                ConnectionStatus::Reconnecting => "Reconectando ao LoL",
                ConnectionStatus::Incompatible => "Cliente/contrato incompatível",
            }
            .into(),
        ),
        RuntimeEvent::Draft(snapshot) => {
            let Some(runtime) = target.upgrade() else {
                return;
            };
            if snapshot.source_epoch != runtime.source_epoch() {
                return;
            }
            let mut c = c.lock().unwrap();
            if c.source_mode != SourceMode::Connected {
                return;
            }
            let previous = selected_id(ui, &c);
            let role_known = snapshot.local_role.is_some();
            if let Err(error) = c.apply_snapshot(*snapshot) {
                ui.set_connection_status(format!("Draft incompatível: {error}").into());
                return;
            }
            ui.set_local_role_known(role_known);
            update_names(ui, &c, c.local_champion_id.or(previous));
            refresh(ui, &c);
            preference_to_ui(ui, &c);
            sync_runtime(ui, &c, &runtime);
        }
        RuntimeEvent::Error(error) => ui.set_data_error(error.into()),
    }
}
fn ui_event_sender(
    ui: &AppWindow,
    c: Arc<Mutex<Controller>>,
    target: Arc<Mutex<std::sync::Weak<RuntimeHandle>>>,
    mailbox: Arc<EventMailbox>,
) -> EventSender {
    let event_ui = ui.as_weak();
    Arc::new(move |event| {
        if !matches!(mailbox.push(event), Ok(true)) {
            return;
        }
        let c = c.clone();
        let target = target.clone();
        let pending = mailbox.clone();
        if event_ui
            .upgrade_in_event_loop(move |ui| {
                for event in pending.drain() {
                    apply_runtime_event(&ui, &c, &target.lock().unwrap(), event);
                }
            })
            .is_err()
        {
            mailbox.close();
        }
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let path = profile_path();
    let (profile, load_error) = match load_profile(&path) {
        Ok(p) => (p, None),
        Err(e) => (PlayerProfile::default(), Some(e)),
    };
    let settings_path = path.with_file_name("settings.json");
    let (settings, settings_error) = match load_settings(&settings_path) {
        Ok(s) => (s, None),
        Err(e) => (AppSettings::default(), Some(e.to_string())),
    };
    let c = Arc::new(Mutex::new(Controller::new(demo_catalog(), profile)));
    c.lock().unwrap().set_source_mode(SourceMode::Connected)?;
    c.lock().unwrap().set_requested_rank(settings.rank.clone());
    let ui = AppWindow::new()?;
    ui.set_rank_index(
        settings
            .rank
            .as_deref()
            .and_then(|r| RANKS.iter().position(|v| *v == r))
            .map(|i| i as i32 + 1)
            .unwrap_or(0),
    );
    update_names(&ui, &c.lock().unwrap(), None);
    refresh(&ui, &c.lock().unwrap());
    let runtime_target: Arc<Mutex<std::sync::Weak<RuntimeHandle>>> =
        Arc::new(Mutex::new(std::sync::Weak::new()));
    let mailbox = Arc::new(EventMailbox::default());
    let sender = ui_event_sender(&ui, c.clone(), runtime_target.clone(), mailbox.clone());
    let mut config = {
        let c = c.lock().unwrap();
        RuntimeConfig::live(
            path.with_file_name("meta.json"),
            c.profile.clone(),
            c.draft.role,
            settings.rank.clone(),
        )
    };
    let lockfile = std::env::var_os("PIKKER_LCU_LOCKFILE").map(std::path::PathBuf::from);
    config.lcu = Some(pikker::lcu::listener::LcuConfig::new(
        Arc::new(move || pikker::lcu::discovery::discover(lockfile.as_deref())),
        pikker::lcu::transport::LcuTrust::riot()?,
    ));
    let runtime = Arc::new(start_runtime(config, sender)?);
    *runtime_target.lock().unwrap() = Arc::downgrade(&runtime);
    if let Some(e) = &settings_error {
        ui.set_data_error(format!("Configuração inválida: {e}. Arquivo preservado; salvamento bloqueado até reiniciar.").into());
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_refresh_data(move || {
            if let Some(ui) = w.upgrade() {
                sync_runtime(&ui, &c.lock().unwrap(), &runtime);
                ui.set_data_error("".into());
                if let Err(e) = runtime.send(RuntimeCommand::Refresh) {
                    ui.set_data_error(e.to_string().into());
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_source_mode_changed(move |index| {
            if let Some(ui) = w.upgrade() {
                let mut c = c.lock().unwrap();
                let previous = selected_id(&ui, &c);
                match c.set_source_mode(if index == 0 {
                    SourceMode::Simulator
                } else {
                    SourceMode::Connected
                }) {
                    Ok(()) => {
                        let _ = runtime.send(RuntimeCommand::SetAutomatic(
                            c.source_mode == SourceMode::Connected,
                        ));
                        ui.set_local_role_known(false);
                        update_names(&ui, &c, previous);
                        refresh(&ui, &c);
                        sync_runtime(&ui, &c, &runtime);
                    }
                    Err(e) => ui.set_error(e.into()),
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_rank_changed(move |_| {
            if let Some(ui) = w.upgrade() {
                if settings_error.is_some() {
                    ui.set_data_error(
                        "Configuração não salva: corrija o arquivo original e reinicie.".into(),
                    );
                    return;
                }
                let settings = AppSettings {
                    rank: selected_rank(&ui),
                };
                if let Err(e) = save_settings(&settings_path, &settings) {
                    ui.set_data_error(e.to_string().into());
                    return;
                }
                {
                    let mut c = c.lock().unwrap();
                    c.set_requested_rank(settings.rank.clone());
                    details_to_ui(&ui, &c);
                    sync_runtime(&ui, &c, &runtime);
                }
                if let Err(e) = runtime.send(RuntimeCommand::Refresh) {
                    ui.set_data_error(e.to_string().into());
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        ui.on_select_candidate(move |id| {
            if let Some(ui) = w.upgrade() {
                let c = c.lock().unwrap();
                if let Some(index) = names(&c)
                    .iter()
                    .position(|candidate| *candidate == id as u32)
                {
                    ui.set_selected_champion(index as i32);
                    preference_to_ui(&ui, &c);
                    details_to_ui(&ui, &c);
                }
            }
        });
    }
    if let Some(e) = &load_error {
        ui.set_error(
            format!(
                "Erro ao ler preferências: {e}. Salvamento bloqueado para preservar o arquivo."
            )
            .into(),
        );
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_role_changed(move |i| {
            if let (Some(ui), Some(role)) = (w.upgrade(), Role::ALL.get(i as usize)) {
                dispatch(
                    &ui,
                    &mut c.lock().unwrap(),
                    Action::SetRole(*role),
                    &runtime,
                );
                preference_to_ui(&ui, &c.lock().unwrap());
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_phase_changed(move |i| {
            if let Some(ui) = w.upgrade() {
                let phases = [
                    Phase::Waiting,
                    Phase::Ban,
                    Phase::Pick,
                    Phase::Finalization,
                    Phase::Ended,
                ];
                if let Some(p) = phases.get(i as usize) {
                    dispatch(&ui, &mut c.lock().unwrap(), Action::SetPhase(*p), &runtime);
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_load_preset(move |i| {
            if let Some(ui) = w.upgrade() {
                dispatch(
                    &ui,
                    &mut c.lock().unwrap(),
                    Action::LoadPreset(i as usize),
                    &runtime,
                );
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_new_session(move || {
            if let Some(ui) = w.upgrade() {
                dispatch(&ui, &mut c.lock().unwrap(), Action::NewSession, &runtime);
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        ui.on_select_slot(move |team, slot| {
            if let Some(ui) = w.upgrade() {
                ui.set_selected_team(team);
                ui.set_selected_slot(slot);
                let c = c.lock().unwrap();
                let s = if team == 1 {
                    &c.draft.enemies
                } else {
                    &c.draft.allies
                };
                if let Some(s) = s.get(slot as usize) {
                    ui.set_selected_position(
                        s.role
                            .and_then(|r| Role::ALL.iter().position(|x| *x == r))
                            .map(|i| i as i32 + 1)
                            .unwrap_or(0),
                    );
                    if let Some(id) = s.champion.or(s.hover)
                        && let Some(i) = names(&c).iter().position(|x| *x == id)
                    {
                        ui.set_selected_champion(i as i32);
                    }
                }
                preference_to_ui(&ui, &c);
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        ui.on_select_champion(move |_| {
            if let Some(ui) = w.upgrade() {
                let c = c.lock().unwrap();
                preference_to_ui(&ui, &c);
                details_to_ui(&ui, &c);
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_edit_pick(move |mode| {
            if let Some(ui) = w.upgrade() {
                let mut c = c.lock().unwrap();
                let id = selected_id(&ui, &c);
                let enemy = ui.get_selected_team() == 1;
                let slot = ui.get_selected_slot() as usize;
                let role = Role::ALL
                    .get(ui.get_selected_position().wrapping_sub(1) as usize)
                    .copied();
                let action = if mode == 0 {
                    Action::SetHover { enemy, slot, id }
                } else {
                    Action::ConfirmPick {
                        enemy,
                        slot,
                        id: if mode == 2 { None } else { id },
                        role: if mode == 2 { None } else { role },
                    }
                };
                dispatch(&ui, &mut c, action, &runtime);
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_ban_champion(move |banned| {
            if let Some(ui) = w.upgrade() {
                let mut c = c.lock().unwrap();
                if let Some(id) = selected_id(&ui, &c) {
                    dispatch(&ui, &mut c, Action::SetBan { id, banned }, &runtime);
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        let runtime = runtime.clone();
        ui.on_save_preference(move || {
            if let Some(ui) = w.upgrade() {
                if load_error.is_some() {
                    ui.set_error(
                        "Preferências não salvas: corrija o arquivo original e reinicie.".into(),
                    );
                    return;
                }
                let mut c = c.lock().unwrap();
                if let Some(id) = selected_id(&ui, &c) {
                    let original = c
                        .profile
                        .preferences
                        .get(&(id, c.draft.role))
                        .cloned()
                        .unwrap_or_default();
                    let result = edit_preference(
                        &original,
                        ui.get_favorite(),
                        ui.get_proficiency_index() as usize,
                        ui.get_proficiency_edited(),
                    )
                    .and_then(|p| c.set_preference(id, p))
                    .and_then(|_| save_profile(&path, &c.profile));
                    match result {
                        Ok(()) => ui.set_error("".into()),
                        Err(e) => ui.set_error(e.into()),
                    }
                    refresh(&ui, &c);
                    preference_to_ui(&ui, &c);
                    sync_runtime(&ui, &c, &runtime);
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        let c = c.clone();
        ui.on_apply_weights(move || {
            if let Some(ui) = w.upgrade() {
                let weights = Weights {
                    viability: ui.get_weight_viability() as f32 / 100.0,
                    proficiency: ui.get_weight_proficiency() as f32 / 100.0,
                    composition: ui.get_weight_composition() as f32 / 100.0,
                    blind: ui.get_weight_blind() as f32 / 100.0,
                    preference: ui.get_weight_preference() as f32 / 100.0,
                };
                let mut c = c.lock().unwrap();
                match c.set_weights(weights) {
                    Ok(()) => ui.set_error("".into()),
                    Err(e) => ui.set_error(e.into()),
                }
                refresh(&ui, &c);
            }
        });
    }
    runtime.send(RuntimeCommand::StartBootstrap)?;
    let result = ui.run();
    mailbox.close();
    runtime.shutdown()?;
    result?;
    Ok(())
}
