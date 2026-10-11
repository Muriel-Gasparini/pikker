use super::model::*;
use super::{
    cache::{load_cache, save_cache},
    opgg::{HttpRequest, HttpResponse, HttpTransport, MetaProvider},
};
use crate::{
    domain::{ChampionId, PlayerProfile, Role},
    runtime::IntegrationError,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapStage {
    LocalCache,
    Catalog,
    GeneralMeta,
    Details,
    Ready,
    Partial,
    Paused,
}
#[derive(Clone, Debug)]
pub struct BootstrapProgress {
    pub stage: BootstrapStage,
    pub completed: usize,
    pub total: Option<usize>,
}
pub type ProviderFactory = Arc<
    dyn Fn(
            Option<&MetaSnapshot>,
            Option<&str>,
            BootstrapControl,
        ) -> Result<Box<dyn MetaProvider>, IntegrationError>
        + Send
        + Sync,
>;
pub trait BootstrapWaiter: Send + Sync {
    fn wait(&self, delay: Duration, control: &BootstrapControl) -> Result<(), IntegrationError>;
}
pub struct RealWaiter;
impl BootstrapWaiter for RealWaiter {
    fn wait(&self, delay: Duration, control: &BootstrapControl) -> Result<(), IntegrationError> {
        control.wait(delay)
    }
}
pub struct GatedTransport<T: HttpTransport> {
    pub inner: T,
    pub control: BootstrapControl,
}
impl<T: HttpTransport> HttpTransport for GatedTransport<T> {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, IntegrationError> {
        self.control.checkpoint()?;
        self.inner.send(request)
    }
}

fn progress(
    sender: &crate::runtime::EventSender,
    stage: BootstrapStage,
    completed: usize,
    total: Option<usize>,
) {
    sender(crate::runtime::RuntimeEvent::Progress(BootstrapProgress {
        stage,
        completed,
        total,
    }));
}
fn request<T>(
    config: &crate::runtime::RuntimeConfig,
    control: &BootstrapControl,
    budget: &mut AttemptBudget,
    started: &mut bool,
    mut operation: impl FnMut() -> Result<T, IntegrationError>,
) -> Result<T, IntegrationError> {
    for attempt in 1..=3 {
        control.checkpoint()?;
        if *started {
            config.waiter.wait(Duration::from_secs(1), control)?;
        }
        budget.begin()?;
        *started = true;
        match operation() {
            Ok(result) => return Ok(result),
            Err(error) => match retry_delay(&error, attempt) {
                Some(delay) => config.waiter.wait(delay, control)?,
                None => {
                    // Exhausting this query does not release the provider's
                    // rate limit for the next champion in the same bootstrap.
                    if let IntegrationError::Http {
                        status: 429,
                        retry_after,
                    } = &error
                    {
                        config
                            .waiter
                            .wait(retry_after.unwrap_or(Duration::from_secs(30)), control)?;
                    }
                    return Err(error);
                }
            },
        }
    }
    Err(IntegrationError::Invalid("Tentativas esgotadas"))
}

pub fn run_bootstrap(
    config: &crate::runtime::RuntimeConfig,
    control: &BootstrapControl,
    sender: &crate::runtime::EventSender,
    force: bool,
) -> Result<MetaSnapshot, IntegrationError> {
    use crate::runtime::RuntimeEvent;
    let now = (config.clock)();
    let key = MetaKey {
        source: "OP.GG".into(),
        schema_version: 1,
        requested_rank: config.rank.clone(),
    };
    progress(sender, BootstrapStage::LocalCache, 0, None);
    let mut writable = true;
    let cached = match load_cache(&config.cache_path, &key, now) {
        Ok(cache) => cache,
        Err(error) => {
            writable = false;
            sender(RuntimeEvent::Error(error.to_string()));
            None
        }
    };
    let publish = |snapshot| sender(RuntimeEvent::MetaReady(Box::new(snapshot)));
    if let Some(cache) = &cached {
        control.publish(cache.snapshot.clone(), &publish);
    }
    control.checkpoint()?;
    let needs_general = force || cached.as_ref().is_none_or(|c| c.stale);
    let mut budget = AttemptBudget::default();
    let mut started = false;
    let mut provider = None;
    let mut snapshot = if needs_general {
        progress(sender, BootstrapStage::Catalog, 0, None);
        let mut source = (config.provider_factory)(None, config.rank.as_deref(), control.clone())?;
        control.checkpoint()?;
        progress(sender, BootstrapStage::GeneralMeta, 0, None);
        let mut snapshot = request(config, control, &mut budget, &mut started, || {
            source.fetch_general()
        })?;
        snapshot.validate()?;
        if snapshot.key != key {
            return Err(IntegrationError::Invalid(
                "Fonte retornou filtros diferentes",
            ));
        }
        if let Some(old) = &cached
            && old.snapshot.labels.catalog_version == snapshot.labels.catalog_version
        {
            snapshot.details = old
                .snapshot
                .details
                .iter()
                .filter(|d| snapshot.catalog.iter().any(|c| c.id == d.champion_id))
                .cloned()
                .collect();
        }
        provider = Some(source);
        if writable && let Err(error) = save_cache(&config.cache_path, &snapshot) {
            writable = false;
            sender(RuntimeEvent::Error(error.to_string()));
        }
        control.publish(snapshot.clone(), &publish);
        snapshot
    } else {
        cached
            .as_ref()
            .expect("fresh cache exists")
            .snapshot
            .clone()
    };
    let pending: Vec<_> = plan_details(&snapshot.stats, &config.profile, config.current_role)
        .into_iter()
        .filter(|r| {
            !snapshot.details.iter().any(|d| {
                d.champion_id == r.champion_id
                    && d.role == r.role
                    && d.requested_rank == config.rank
                    && now
                        .checked_sub(d.fetched_at)
                        .is_some_and(|age| (-600..CACHE_TTL_SECONDS).contains(&age))
            })
        })
        .collect();
    let total = pending.len();
    let mut completed = 0;
    let mut failed = 0;
    if !pending.is_empty() && provider.is_none() {
        control.checkpoint()?;
        provider = Some((config.provider_factory)(
            Some(&snapshot),
            config.rank.as_deref(),
            control.clone(),
        )?);
    }
    progress(sender, BootstrapStage::Details, 0, Some(total));
    for item in pending {
        control.checkpoint()?;
        if budget.attempts() >= 60 {
            failed += 1;
            break;
        }
        let Some(champion) = snapshot
            .catalog
            .iter()
            .find(|c| c.id == item.champion_id)
            .cloned()
        else {
            failed += 1;
            continue;
        };
        let source = provider.as_mut().expect("pending details have provider");
        match request(config, control, &mut budget, &mut started, || {
            source.fetch_details(&champion, item.role, config.rank.as_deref())
        }) {
            Ok(details) => {
                details.validate()?;
                if details.champion_id != item.champion_id
                    || details.role != item.role
                    || details.requested_rank != config.rank
                {
                    return Err(IntegrationError::Invalid(
                        "Detalhes retornados para outro campeão/rota/elo",
                    ));
                }
                snapshot.details.retain(|d| {
                    !(d.champion_id == details.champion_id
                        && d.role == details.role
                        && d.requested_rank == details.requested_rank)
                });
                snapshot.details.push(details);
                if writable && let Err(error) = save_cache(&config.cache_path, &snapshot) {
                    writable = false;
                    sender(RuntimeEvent::Error(error.to_string()));
                }
                control.publish(snapshot.clone(), &publish);
            }
            Err(IntegrationError::Paused) => {
                progress(sender, BootstrapStage::Paused, completed, Some(total));
                return Err(IntegrationError::Paused);
            }
            Err(IntegrationError::Cancelled) => return Err(IntegrationError::Cancelled),
            Err(error) => {
                failed += 1;
                sender(RuntimeEvent::Error(error.to_string()));
            }
        }
        completed += 1;
        progress(sender, BootstrapStage::Details, completed, Some(total));
    }
    progress(
        sender,
        if failed == 0 {
            BootstrapStage::Ready
        } else {
            BootstrapStage::Partial
        },
        completed,
        Some(total),
    );
    Ok(snapshot)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailRequest {
    pub champion_id: ChampionId,
    pub role: Role,
}

pub fn plan_details(
    stats: &[RoleStats],
    favorites: &PlayerProfile,
    current_role: Role,
) -> Vec<DetailRequest> {
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    let roles =
        std::iter::once(current_role).chain(Role::ALL.into_iter().filter(|r| *r != current_role));
    for role in roles {
        let favorites = favorites
            .preferences
            .iter()
            .filter(|((_, r), p)| *r == role && p.favorite)
            .map(|((id, _), _)| *id);
        let top = stats
            .iter()
            .filter(|r| r.role == role)
            .take(10)
            .map(|r| r.champion_id);
        for id in favorites.chain(top) {
            if seen.insert((id, role)) {
                output.push(DetailRequest {
                    champion_id: id,
                    role,
                });
            }
        }
    }
    output
}

#[derive(Default)]
pub struct AttemptBudget {
    attempts: usize,
}
impl AttemptBudget {
    pub fn begin(&mut self) -> Result<(), IntegrationError> {
        if self.attempts >= 60 {
            return Err(IntegrationError::Invalid(
                "Limite de consultas desta atualização atingido",
            ));
        }
        self.attempts += 1;
        Ok(())
    }
    pub fn attempts(&self) -> usize {
        self.attempts
    }
}
pub fn retry_delay(error: &IntegrationError, attempt: usize) -> Option<Duration> {
    if !(1..3).contains(&attempt) {
        return None;
    }
    match error {
        IntegrationError::Http {
            status: 429,
            retry_after: Some(delay),
        } => Some(*delay),
        IntegrationError::Network
        | IntegrationError::Http {
            status: 408 | 429 | 500..=599,
            ..
        } => Some(Duration::from_secs([5, 15][attempt - 1])),
        _ => None,
    }
}

#[derive(Default)]
struct ControlState {
    draft_active: bool,
    cancelled: bool,
    deferred: Option<MetaSnapshot>,
}
#[derive(Default)]
struct ControlInner {
    state: Mutex<ControlState>,
    changed: Condvar,
}
#[derive(Clone, Default)]
pub struct BootstrapControl {
    inner: Arc<ControlInner>,
}
impl BootstrapControl {
    fn state_result(state: &ControlState) -> Result<(), IntegrationError> {
        if state.cancelled {
            Err(IntegrationError::Cancelled)
        } else if state.draft_active {
            Err(IntegrationError::Paused)
        } else {
            Ok(())
        }
    }
    pub fn checkpoint(&self) -> Result<(), IntegrationError> {
        Self::state_result(&self.inner.state.lock().unwrap_or_else(|e| e.into_inner()))
    }
    pub fn cancel(&self) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled = true;
        self.inner.changed.notify_all();
    }
    pub fn set_draft_active(&self, active: bool, sender: &dyn Fn(MetaSnapshot)) {
        if let Some(snapshot) = self.transition_draft(active) {
            sender(snapshot);
        }
    }
    pub(crate) fn transition_draft(&self, active: bool) -> Option<MetaSnapshot> {
        let pending = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            state.draft_active = active;
            if !active && !state.cancelled {
                state.deferred.take()
            } else {
                None
            }
        };
        self.inner.changed.notify_all();
        pending
    }
    pub fn publish(&self, snapshot: MetaSnapshot, sender: &dyn Fn(MetaSnapshot)) {
        let ready = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.cancelled {
                return;
            }
            if state.draft_active {
                if state
                    .deferred
                    .as_ref()
                    .is_none_or(|previous| previous.fetched_at <= snapshot.fetched_at)
                {
                    state.deferred = Some(snapshot);
                }
                None
            } else {
                Some(snapshot)
            }
        };
        if let Some(snapshot) = ready {
            sender(snapshot);
        }
    }
    pub fn wait(&self, delay: Duration) -> Result<(), IntegrationError> {
        let started = Instant::now();
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            Self::state_result(&state)?;
            let remaining = delay.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            state = self
                .inner
                .changed
                .wait_timeout(state, remaining.min(Duration::from_secs(60)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
