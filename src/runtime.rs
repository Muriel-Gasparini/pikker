use crate::{
    domain::{DraftState, PlayerProfile, Role},
    meta::{bootstrap::*, model::MetaSnapshot},
};
use std::fmt;
mod mailbox;
pub use mailbox::EventMailbox;
use std::time::Duration;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Absent,
    Connecting,
    Connected,
    Reconnecting,
    Incompatible,
}
#[derive(Clone, Debug)]
pub struct DraftSnapshot {
    pub source_epoch: u64,
    pub generation: u64,
    pub revision: u64,
    pub state: DraftState,
    pub pickable_ids: Option<std::collections::BTreeSet<u32>>,
    pub local_role: Option<Role>,
    pub local_champion_id: Option<u32>,
}
#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    Progress(BootstrapProgress),
    MetaReady(Box<MetaSnapshot>),
    Connection(ConnectionStatus),
    Draft(Box<DraftSnapshot>),
    Error(String),
}
pub type EventSender = Arc<dyn Fn(RuntimeEvent) + Send + Sync>;
#[derive(Clone)]
pub struct RuntimeConfig {
    pub lcu: Option<crate::lcu::listener::LcuConfig>,
    pub cache_path: PathBuf,
    pub profile: PlayerProfile,
    pub current_role: Role,
    pub rank: Option<String>,
    pub provider_factory: ProviderFactory,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub waiter: Arc<dyn BootstrapWaiter>,
}
impl RuntimeConfig {
    pub fn with_provider(
        cache_path: PathBuf,
        profile: PlayerProfile,
        current_role: Role,
        rank: Option<String>,
        provider_factory: ProviderFactory,
    ) -> Self {
        Self {
            lcu: None,
            cache_path,
            profile,
            current_role,
            rank,
            provider_factory,
            clock: Arc::new(crate::meta::opgg::now_seconds),
            waiter: Arc::new(RealWaiter),
        }
    }
    pub fn live(
        cache_path: PathBuf,
        profile: PlayerProfile,
        current_role: Role,
        rank: Option<String>,
    ) -> Self {
        use crate::meta::opgg::*;
        let factory: ProviderFactory = Arc::new(|cached, rank, control| {
            let transport = GatedTransport {
                inner: PublicTransport::default(),
                control,
            };
            let (catalog, labels) = if let Some(cache) = cached {
                (cache.catalog.clone(), cache.labels.clone())
            } else {
                fetch_static_catalog(&transport, "br")?
            };
            Ok(Box::new(
                OpggProvider::new(transport, catalog, rank.map(str::to_owned)).with_labels(labels),
            ))
        });
        Self::with_provider(cache_path, profile, current_role, rank, factory)
    }
}
pub enum RuntimeCommand {
    SetAutomatic(bool),
    StartBootstrap,
    Refresh,
    SetDraftActive(bool),
    UpdatePreferences(PlayerProfile, Role, Option<String>),
    Shutdown,
}
pub struct RuntimeHandle {
    config: Mutex<RuntimeConfig>,
    control: BootstrapControl,
    sender: EventSender,
    bootstrap: Mutex<Option<JoinHandle<()>>>,
    lcu_control: crate::lcu::listener::LcuControl,
    listener: Mutex<Option<JoinHandle<()>>>,
}
pub fn start_runtime(
    config: RuntimeConfig,
    sender: EventSender,
) -> Result<RuntimeHandle, IntegrationError> {
    let control = BootstrapControl::default();
    let lcu_control = crate::lcu::listener::LcuControl::default();
    let listener = if let Some(lcu) = config.lcu.clone() {
        let (worker_control, worker_bootstrap, worker_sender) =
            (lcu_control.clone(), control.clone(), sender.clone());
        let cache_path = config.cache_path.clone();
        let rank = config.rank.clone();
        let clock = config.clock.clone();
        Some(
            std::thread::Builder::new()
                .name("pikker-lcu".into())
                .stack_size(512 * 1024)
                .spawn(move || {
                    // Publish validated cache before the first draft, including mid-draft startup.
                    let key = crate::meta::model::MetaKey {
                        source: "OP.GG".into(),
                        schema_version: 1,
                        requested_rank: rank,
                    };
                    if let Ok(Some(cache)) =
                        crate::meta::cache::load_cache(&cache_path, &key, clock())
                    {
                        worker_sender(RuntimeEvent::MetaReady(Box::new(cache.snapshot)));
                    }
                    crate::lcu::listener::run_listener(
                        lcu,
                        worker_control,
                        worker_bootstrap,
                        worker_sender,
                    );
                })?,
        )
    } else {
        None
    };
    Ok(RuntimeHandle {
        config: Mutex::new(config),
        control,
        sender,
        bootstrap: Mutex::new(None),
        lcu_control,
        listener: Mutex::new(listener),
    })
}
impl RuntimeHandle {
    pub fn send(&self, command: RuntimeCommand) -> Result<(), IntegrationError> {
        match command {
            RuntimeCommand::SetAutomatic(automatic) => {
                let enabled = self
                    .config
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .lcu
                    .is_some();
                self.lcu_control
                    .set_automatic_gated(automatic, enabled.then_some(&self.control));
                Ok(())
            }
            RuntimeCommand::Shutdown => self.shutdown(),
            RuntimeCommand::SetDraftActive(active) => {
                self.control.set_draft_active(active, &|snapshot| {
                    (self.sender)(RuntimeEvent::MetaReady(Box::new(snapshot)))
                });
                Ok(())
            }
            RuntimeCommand::UpdatePreferences(profile, role, rank) => {
                let mut config = self.config.lock().unwrap_or_else(|e| e.into_inner());
                config.profile = profile;
                config.current_role = role;
                config.rank = rank;
                Ok(())
            }
            RuntimeCommand::StartBootstrap | RuntimeCommand::Refresh => {
                if matches!(self.control.checkpoint(), Err(IntegrationError::Cancelled)) {
                    return Err(IntegrationError::Cancelled);
                }
                let mut slot = self.bootstrap.lock().unwrap_or_else(|e| e.into_inner());
                if slot.as_ref().is_some_and(|w| !w.is_finished()) {
                    return Err(IntegrationError::Invalid(
                        "Atualização já está em andamento",
                    ));
                }
                if let Some(worker) = slot.take() {
                    let _ = worker.join();
                }
                let config = self
                    .config
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let sender = self.sender.clone();
                let control = self.control.clone();
                let lcu_control = self.lcu_control.clone();
                let force = matches!(command, RuntimeCommand::Refresh);
                *slot = Some(
                    std::thread::Builder::new()
                        .name("pikker-bootstrap".into())
                        .stack_size(512 * 1024)
                        .spawn(move || {
                            if config.lcu.is_some() && lcu_control.wait_initial().is_err() {
                                return;
                            }
                            if let Err(error) = run_bootstrap(&config, &control, &sender, force) {
                                match error {
                                    IntegrationError::Cancelled => {}
                                    IntegrationError::Paused => {
                                        sender(RuntimeEvent::Progress(BootstrapProgress {
                                            stage: BootstrapStage::Paused,
                                            completed: 0,
                                            total: None,
                                        }))
                                    }
                                    error => {
                                        sender(RuntimeEvent::Error(error.to_string()));
                                        sender(RuntimeEvent::Progress(BootstrapProgress {
                                            stage: BootstrapStage::Partial,
                                            completed: 0,
                                            total: None,
                                        }));
                                    }
                                }
                            }
                        })?,
                );
                Ok(())
            }
        }
    }
    pub fn shutdown(&self) -> Result<(), IntegrationError> {
        self.control.cancel();
        self.lcu_control.cancel();
        if let Some(listener) = self
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            listener
                .join()
                .map_err(|_| IntegrationError::Invalid("Conexão local encerrou inesperadamente"))?;
        }
        let worker = self
            .bootstrap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(worker) = worker {
            worker.join().map_err(|_| {
                IntegrationError::Invalid("Worker de atualização encerrou inesperadamente")
            })?;
        }
        Ok(())
    }
    pub fn control(&self) -> BootstrapControl {
        self.control.clone()
    }
    pub fn source_epoch(&self) -> u64 {
        self.lcu_control.epoch()
    }
}
impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Debug)]
pub enum IntegrationError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(&'static str),
    Http {
        status: u16,
        retry_after: Option<Duration>,
    },
    Network,
    Cancelled,
    Paused,
}
impl fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => f.write_str("Não foi possível acessar os dados locais"),
            Self::Json(_) => f.write_str("Formato de dados inválido"),
            Self::Invalid(message) => f.write_str(message),
            Self::Http { status, .. } => write!(f, "Fonte retornou HTTP {status}"),
            Self::Network => f.write_str("Não foi possível conectar à fonte"),
            Self::Cancelled => f.write_str("Atualização cancelada"),
            Self::Paused => f.write_str("Atualização pausada durante o draft"),
        }
    }
}
impl std::error::Error for IntegrationError {}
impl From<std::io::Error> for IntegrationError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for IntegrationError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
