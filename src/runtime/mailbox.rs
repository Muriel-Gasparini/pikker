use super::{IntegrationError, RuntimeEvent};
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
};

#[derive(Default)]
struct Pending {
    events: VecDeque<RuntimeEvent>,
    closed: bool,
}
/// Bounded UI delivery with adjacent-event coalescing. Session boundaries stay FIFO.
#[derive(Default)]
pub struct EventMailbox {
    pending: Mutex<Pending>,
    available: Condvar,
}
impl EventMailbox {
    /// True means the caller must schedule one event-loop drain.
    pub fn push(&self, event: RuntimeEvent) -> Result<bool, IntegrationError> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.closed {
            return Err(IntegrationError::Cancelled);
        }
        let coalesce = pending
            .events
            .back()
            .is_some_and(|last| match (last, &event) {
                (RuntimeEvent::Draft(a), RuntimeEvent::Draft(b)) => {
                    a.source_epoch == b.source_epoch
                        && a.generation == b.generation
                        && a.state.phase == b.state.phase
                }
                (RuntimeEvent::Progress(_), RuntimeEvent::Progress(_))
                | (RuntimeEvent::Connection(_), RuntimeEvent::Connection(_))
                | (RuntimeEvent::MetaReady(_), RuntimeEvent::MetaReady(_)) => true,
                _ => false,
            });
        if coalesce {
            *pending.events.back_mut().unwrap() = event;
            return Ok(false);
        }
        pending = self
            .available
            .wait_while(pending, |s| !s.closed && s.events.len() >= 64)
            .unwrap_or_else(|e| e.into_inner());
        if pending.closed {
            return Err(IntegrationError::Cancelled);
        }
        let schedule = pending.events.is_empty();
        pending.events.push_back(event);
        Ok(schedule)
    }
    pub fn drain(&self) -> Vec<RuntimeEvent> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let events = pending.events.drain(..).collect();
        self.available.notify_all();
        events
    }
    pub fn close(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.closed = true;
        pending.events.clear();
        self.available.notify_all();
    }
}
