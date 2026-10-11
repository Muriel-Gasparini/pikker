//! Event-driven, read-only client listener. No external statistical requests here.
use super::{
    discovery::LcuCredentials,
    transport::{ConnectionStop, LcuTransport, LcuTrust},
};
use crate::{
    domain::Phase as DraftPhase,
    meta::bootstrap::BootstrapControl,
    runtime::{ConnectionStatus, EventSender, IntegrationError, RuntimeEvent},
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub type Discovery =
    Arc<dyn Fn() -> Result<Option<LcuCredentials>, IntegrationError> + Send + Sync>;
pub trait LcuWaiter: Send + Sync {
    fn wait(
        &self,
        delay: Duration,
        control: &LcuControl,
        epoch: u64,
    ) -> Result<(), IntegrationError>;
}
struct RealWaiter;
impl LcuWaiter for RealWaiter {
    fn wait(
        &self,
        delay: Duration,
        control: &LcuControl,
        epoch: u64,
    ) -> Result<(), IntegrationError> {
        control.wait(delay, epoch)
    }
}
#[derive(Clone)]
pub struct LcuConfig {
    pub discovery: Discovery,
    pub trust: LcuTrust,
    pub waiter: Arc<dyn LcuWaiter>,
}
impl LcuConfig {
    pub fn new(discovery: Discovery, trust: LcuTrust) -> Self {
        Self {
            discovery,
            trust,
            waiter: Arc::new(RealWaiter),
        }
    }
}
struct State {
    cancelled: bool,
    automatic: bool,
    epoch: u64,
    ready: bool,
    socket: Option<ConnectionStop>,
}
#[derive(Clone)]
pub struct LcuControl(Arc<(Mutex<State>, Condvar)>);
impl Default for LcuControl {
    fn default() -> Self {
        Self(Arc::new((
            Mutex::new(State {
                cancelled: false,
                automatic: true,
                epoch: 1,
                ready: false,
                socket: None,
            }),
            Condvar::new(),
        )))
    }
}
impl LcuControl {
    pub fn cancel(&self) {
        let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        state.cancelled = true;
        if let Some(socket) = state.socket.take() {
            socket.cancel();
        }
        self.0.1.notify_all();
    }
    pub fn set_automatic(&self, automatic: bool) {
        self.set_automatic_gated(automatic, None);
    }
    pub(crate) fn set_automatic_gated(
        &self,
        automatic: bool,
        bootstrap: Option<&BootstrapControl>,
    ) {
        let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.automatic != automatic {
            state.automatic = automatic;
            state.epoch = state.epoch.saturating_add(1);
            state.ready = !automatic;
            if automatic && let Some(bootstrap) = bootstrap {
                bootstrap.transition_draft(true);
            }
            if let Some(socket) = state.socket.take() {
                socket.cancel();
            }
            self.0.1.notify_all();
        }
    }
    pub fn epoch(&self) -> u64 {
        self.0.0.lock().unwrap_or_else(|e| e.into_inner()).epoch
    }
    pub fn check(&self, epoch: u64) -> Result<(), IntegrationError> {
        let state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.cancelled || !state.automatic || state.epoch != epoch {
            Err(IntegrationError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn activity(
        &self,
        epoch: u64,
        bootstrap: &BootstrapControl,
        active: bool,
    ) -> Result<Option<crate::meta::model::MetaSnapshot>, IntegrationError> {
        let state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.cancelled || !state.automatic || state.epoch != epoch {
            return Err(IntegrationError::Cancelled);
        }
        // No sender callback while holding the mode mutex. Mode changes and
        // source gating are atomic; callbacks may safely re-enter the runtime.
        Ok(bootstrap.transition_draft(active))
    }
    fn wait(&self, delay: Duration, epoch: u64) -> Result<(), IntegrationError> {
        let state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        let guard = self
            .0
            .1
            .wait_timeout_while(state, delay, |s| {
                !s.cancelled && s.automatic && s.epoch == epoch
            })
            .unwrap_or_else(|e| e.into_inner());
        drop(guard);
        self.check(epoch)
    }
    fn enabled_epoch(&self) -> Option<u64> {
        let state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        let state = self
            .0
            .1
            .wait_while(state, |s| !s.cancelled && !s.automatic)
            .unwrap_or_else(|e| e.into_inner());
        (!state.cancelled).then_some(state.epoch)
    }
    fn attach(&self, epoch: u64, socket: ConnectionStop) -> Result<(), IntegrationError> {
        let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.cancelled || !state.automatic || state.epoch != epoch {
            socket.cancel();
            return Err(IntegrationError::Cancelled);
        }
        state.socket = Some(socket);
        Ok(())
    }
    fn ready(&self, epoch: u64) {
        let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.epoch == epoch {
            state.ready = true;
            self.0.1.notify_all();
        }
    }
    pub fn wait_initial(&self) -> Result<(), IntegrationError> {
        let state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        let state = self
            .0
            .1
            .wait_while(state, |s| !s.cancelled && !s.ready)
            .unwrap_or_else(|e| e.into_inner());
        if state.cancelled {
            Err(IntegrationError::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub fn run_listener(
    config: LcuConfig,
    control: LcuControl,
    bootstrap: BootstrapControl,
    sender: EventSender,
) {
    let mut generation = 0u64;
    let mut failures = 0usize;
    let mut previous_status = None;
    let mut status = |value| {
        if previous_status != Some(value) {
            previous_status = Some(value);
            sender(RuntimeEvent::Connection(value));
        }
    };
    while let Some(epoch) = control.enabled_epoch() {
        generation += 1;
        let result = (|| -> Result<(), IntegrationError> {
            let Some(credentials) = (config.discovery)()? else {
                control.check(epoch)?;
                status(ConnectionStatus::Absent);
                let mut empty =
                    super::normalize::normalize_session(&serde_json::Value::Null, generation, 1)?;
                empty.source_epoch = epoch;
                sender(RuntimeEvent::Draft(Box::new(empty)));
                if let Some(s) = control.activity(epoch, &bootstrap, false)? {
                    sender(RuntimeEvent::MetaReady(Box::new(s)));
                }
                control.ready(epoch);
                return Ok(());
            };
            // Startup I/O waits on readiness. Later absent-client probes must not
            // pause an already running bootstrap, but a discovered client must.
            control.activity(epoch, &bootstrap, true)?;
            status(if failures == 0 {
                ConnectionStatus::Connecting
            } else {
                ConnectionStatus::Reconnecting
            });
            let stop = ConnectionStop::default();
            control.attach(epoch, stop.clone())?;
            let mut transport =
                LcuTransport::connect_cancellable(credentials, &config.trust, stop)?;
            let mut revision = 0u64;
            loop {
                control.check(epoch)?;
                revision += 1;
                let mut snapshot = transport.read_initial_snapshot(generation, revision)?;
                control.check(epoch)?;
                snapshot.source_epoch = epoch;
                let active = matches!(
                    snapshot.state.phase,
                    DraftPhase::Ban | DraftPhase::Pick | DraftPhase::Finalization
                );
                if active {
                    control.activity(epoch, &bootstrap, true)?;
                }
                sender(RuntimeEvent::Draft(Box::new(snapshot)));
                if !active && let Some(s) = control.activity(epoch, &bootstrap, false)? {
                    sender(RuntimeEvent::MetaReady(Box::new(s)));
                }
                control.ready(epoch);
                status(ConnectionStatus::Connected);
                failures = 0;
                // URI-filtered events are invalidations. Re-read current local REST
                // so an old queued event cannot resurrect a pre-resync draft.
                loop {
                    control.check(epoch)?;
                    if transport.next_event()?.is_some() {
                        // An invalidation makes inactivity uncertain. Freeze the
                        // external source before any potentially slow local GET.
                        control.activity(epoch, &bootstrap, true)?;
                        break;
                    }
                }
                if !active {
                    generation += 1;
                    revision = 0;
                }
            }
        })();
        if control.check(epoch).is_err() {
            failures = 0;
            continue;
        }
        if result.is_err() {
            if control.activity(epoch, &bootstrap, true).is_err() {
                continue;
            }
            status(ConnectionStatus::Incompatible);
            // Unknown state stays frozen; permit bootstrap to finish as Paused.
            control.ready(epoch);
        }
        let delay = [2, 5, 15, 30][failures.min(3)];
        failures = failures.saturating_add(1);
        let _ = config
            .waiter
            .wait(Duration::from_secs(delay), &control, epoch);
    }
}
