//! Process-wide priority admission for optional model work.
//!
//! One native model operation or model initialization runs at a time while
//! adapter memory requirements remain unknown. Playback blocks metadata,
//! enrichment and maintenance work; realtime captions and interactive search
//! retain access. Already-running native calls finish their bounded unit.

use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::model_runtime::RuntimeCancellation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkClass {
    Realtime,
    Interactive,
    Foreground,
    Metadata,
    Background,
    Maintenance,
}

impl WorkClass {
    fn rank(self) -> u8 {
        match self {
            Self::Realtime => 5,
            Self::Interactive => 4,
            Self::Foreground => 3,
            Self::Metadata => 2,
            Self::Background => 1,
            Self::Maintenance => 0,
        }
    }

    fn allowed_during_playback(self) -> bool {
        matches!(self, Self::Realtime | Self::Interactive)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Realtime => "realtime",
            Self::Interactive => "interactive",
            Self::Foreground => "foreground",
            Self::Metadata => "metadata",
            Self::Background => "background",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Default)]
struct State {
    active: Option<WorkClass>,
    playback_sessions: usize,
    next_ticket: u64,
    waiters: Vec<(u64, WorkClass)>,
}

#[derive(Default)]
pub struct AdmissionController {
    state: Mutex<State>,
    changed: Notify,
}

pub struct PlaybackActivityGuard {
    controller: Arc<AdmissionController>,
}

impl Drop for PlaybackActivityGuard {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.state);
        state.playback_sessions = state.playback_sessions.saturating_sub(1);
        drop(state);
        self.controller.changed.notify_waiters();
    }
}

pub struct AdmissionPermit {
    controller: Arc<AdmissionController>,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.state);
        state.active = None;
        drop(state);
        self.controller.changed.notify_waiters();
    }
}

struct WaitingTicket {
    controller: Arc<AdmissionController>,
    ticket: u64,
    queued: bool,
}

impl Drop for WaitingTicket {
    fn drop(&mut self) {
        if !self.queued {
            return;
        }
        let mut state = lock(&self.controller.state);
        state.waiters.retain(|(ticket, _)| *ticket != self.ticket);
        drop(state);
        self.controller.changed.notify_waiters();
    }
}

impl AdmissionController {
    pub fn playback_started(self: &Arc<Self>) -> PlaybackActivityGuard {
        let mut state = lock(&self.state);
        state.playback_sessions = state.playback_sessions.saturating_add(1);
        drop(state);
        self.changed.notify_waiters();
        PlaybackActivityGuard {
            controller: Arc::clone(self),
        }
    }

    pub async fn admit(
        self: &Arc<Self>,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<AdmissionPermit, ()> {
        let ticket = {
            let mut state = lock(&self.state);
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1);
            state.waiters.push((ticket, class));
            ticket
        };
        let mut waiting = WaitingTicket {
            controller: Arc::clone(self),
            ticket,
            queued: true,
        };

        loop {
            // Register before examining the state so release notifications
            // between the state check and await cannot be lost.
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if cancellation.is_cancelled() {
                return Err(());
            }
            let granted = {
                let mut state = lock(&self.state);
                let selected = state
                    .waiters
                    .iter()
                    .filter(|(_, queued_class)| {
                        state.playback_sessions == 0 || queued_class.allowed_during_playback()
                    })
                    .max_by_key(|(queued_ticket, queued_class)| {
                        (queued_class.rank(), std::cmp::Reverse(*queued_ticket))
                    })
                    .map(|(selected_ticket, _)| *selected_ticket);
                if state.active.is_none() && selected == Some(ticket) {
                    state
                        .waiters
                        .retain(|(queued_ticket, _)| *queued_ticket != ticket);
                    state.active = Some(class);
                    true
                } else {
                    false
                }
            };
            if granted {
                waiting.queued = false;
                return Ok(AdmissionPermit {
                    controller: Arc::clone(self),
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => return Err(()),
                _ = notified => {}
            }
        }
    }

    pub fn snapshot(&self) -> AdmissionSnapshot {
        let state = lock(&self.state);
        AdmissionSnapshot {
            playback_sessions: state.playback_sessions,
            queued_work: state.waiters.len(),
            active_work_class: state.active,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdmissionSnapshot {
    pub playback_sessions: usize,
    pub queued_work: usize,
    pub active_work_class: Option<WorkClass>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
