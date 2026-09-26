//! Deterministic, process-wide resource governor.
//!
//! Nova's priority order is fixed (master-plan §3.8):
//!
//! 1. Playback — represented by [`PlaybackActivityGuard`]s, never queued.
//! 2. Realtime transcription / translation — [`WorkClass::Realtime`].
//! 3. Interactive search / navigation — [`WorkClass::Interactive`].
//! 4. Explicit user import / refresh — [`WorkClass::Explicit`].
//! 5. Metadata enrichment — [`WorkClass::Metadata`].
//! 6. Background AI / indexing — [`WorkClass::Background`].
//! 7. Maintenance — [`WorkClass::Maintenance`].
//!
//! Two bounded lanes share one policy:
//!
//! - The **model lane** admits one native model initialization or inference
//!   at a time while adapter memory costs remain unmeasured.
//! - The **job lane** bounds how many persisted desktop jobs run at once,
//!   across every profile and job kind. A job holds its [`JobSlot`] for its
//!   whole run and yields it at cooperative [`JobSlot::checkpoint`]s.
//!
//! Selection within a lane is deterministic: realtime first, then any waiter
//! that has been bypassed [`AGING_LIMIT`] times (bounded starvation), then
//! class rank, then FIFO. While any playback guard is alive, metadata,
//! background and maintenance work is suspended; it resumes when the last
//! player session ends. Explicit import/refresh stays admissible during
//! playback because it never uses the model lane. Already-running native
//! calls finish their bounded unit — ONNX/CTranslate2 cannot be preempted.

use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::model_runtime::RuntimeCancellation;

/// A waiter bypassed this many times by lower-numbered-tier grants is served
/// before every class except realtime.
pub const AGING_LIMIT: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkClass {
    Realtime,
    Interactive,
    Explicit,
    Metadata,
    Background,
    Maintenance,
}

impl WorkClass {
    /// Every admissible class, highest priority first.
    pub const ALL: [WorkClass; 6] = [
        Self::Realtime,
        Self::Interactive,
        Self::Explicit,
        Self::Metadata,
        Self::Background,
        Self::Maintenance,
    ];

    /// Master-plan priority tier. Tier 1 is playback, which is not a class.
    pub fn tier(self) -> u8 {
        match self {
            Self::Realtime => 2,
            Self::Interactive => 3,
            Self::Explicit => 4,
            Self::Metadata => 5,
            Self::Background => 6,
            Self::Maintenance => 7,
        }
    }

    fn rank(self) -> u8 {
        8 - self.tier()
    }

    pub fn allowed_during_playback(self) -> bool {
        matches!(self, Self::Realtime | Self::Interactive | Self::Explicit)
    }

    /// Latency-sensitive work may exceed the resident-model budget when every
    /// resident model is leased, so a suspended low-priority lease holder can
    /// never block it (priority inversion).
    pub fn may_exceed_residency(self) -> bool {
        matches!(self, Self::Realtime | Self::Interactive)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Realtime => "realtime",
            Self::Interactive => "interactive",
            Self::Explicit => "explicit",
            Self::Metadata => "metadata",
            Self::Background => "background",
            Self::Maintenance => "maintenance",
        }
    }
}

/// Construction-time capacity policy. No OS memory-pressure signal exists yet;
/// [`ResourceBudget::LOW_RESOURCE`] is the deterministic constrained profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBudget {
    pub model_operations: usize,
    pub job_slots: usize,
    /// In-process heavyweight model backends allowed to stay resident.
    pub resident_models: usize,
}

impl ResourceBudget {
    pub const DEFAULT: Self = Self {
        model_operations: 1,
        job_slots: 2,
        resident_models: 2,
    };
    pub const LOW_RESOURCE: Self = Self {
        model_operations: 1,
        job_slots: 1,
        resident_models: 1,
    };

    fn capacity(self, lane: Lane) -> usize {
        match lane {
            Lane::Model => self.model_operations.max(1),
            Lane::Job => self.job_slots.max(1),
        }
    }
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Model,
    Job,
}

struct Waiter {
    ticket: u64,
    class: WorkClass,
    lane: Lane,
    bypassed: u32,
}

struct State {
    budget: ResourceBudget,
    playback_sessions: usize,
    next_ticket: u64,
    waiters: Vec<Waiter>,
    active: Vec<(u64, Lane, WorkClass)>,
}

impl State {
    fn eligible(&self, waiter: &Waiter) -> bool {
        self.playback_sessions == 0 || waiter.class.allowed_during_playback()
    }

    fn active_in(&self, lane: Lane) -> usize {
        self.active.iter().filter(|(_, l, _)| *l == lane).count()
    }

    /// Deterministic choice of the next waiter for `lane`.
    fn select(&self, lane: Lane) -> Option<u64> {
        self.waiters
            .iter()
            .filter(|w| w.lane == lane && self.eligible(w))
            .max_by_key(|w| {
                (
                    w.class == WorkClass::Realtime,
                    w.bypassed >= AGING_LIMIT,
                    w.class.rank(),
                    std::cmp::Reverse(w.ticket),
                )
            })
            .map(|w| w.ticket)
    }

    fn grant(&mut self, ticket: u64, lane: Lane, class: WorkClass) {
        self.waiters.retain(|w| w.ticket != ticket);
        let playback = self.playback_sessions;
        for w in &mut self.waiters {
            let eligible = playback == 0 || w.class.allowed_during_playback();
            if w.lane == lane && eligible && w.class.rank() < class.rank() {
                w.bypassed = w.bypassed.saturating_add(1);
            }
        }
        self.active.push((ticket, lane, class));
    }
}

pub struct AdmissionController {
    state: Mutex<State>,
    changed: Notify,
}

impl Default for AdmissionController {
    fn default() -> Self {
        Self::new(ResourceBudget::DEFAULT)
    }
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

/// Holds one lane position; released on drop, including panic unwind.
pub struct AdmissionPermit {
    controller: Arc<AdmissionController>,
    ticket: u64,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.state);
        state.active.retain(|(ticket, _, _)| *ticket != self.ticket);
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
        state.waiters.retain(|w| w.ticket != self.ticket);
        drop(state);
        self.controller.changed.notify_waiters();
    }
}

impl AdmissionController {
    pub fn new(budget: ResourceBudget) -> Self {
        Self {
            state: Mutex::new(State {
                budget,
                playback_sessions: 0,
                next_ticket: 0,
                waiters: Vec::new(),
                active: Vec::new(),
            }),
            changed: Notify::new(),
        }
    }

    pub fn budget(&self) -> ResourceBudget {
        lock(&self.state).budget
    }

    pub fn playback_started(self: &Arc<Self>) -> PlaybackActivityGuard {
        let mut state = lock(&self.state);
        state.playback_sessions = state.playback_sessions.saturating_add(1);
        drop(state);
        self.changed.notify_waiters();
        PlaybackActivityGuard {
            controller: Arc::clone(self),
        }
    }

    /// Admit one native model initialization or inference.
    pub async fn admit(
        self: &Arc<Self>,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<AdmissionPermit, ()> {
        self.acquire(Lane::Model, class, cancellation).await
    }

    /// Wait for one bounded job slot. Cancellation leaves no queued state.
    pub async fn acquire_job_slot(
        self: &Arc<Self>,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<JobSlot, ()> {
        let permit = self.acquire(Lane::Job, class, cancellation).await?;
        Ok(JobSlot {
            controller: Arc::clone(self),
            class,
            permit: Some(permit),
        })
    }

    async fn acquire(
        self: &Arc<Self>,
        lane: Lane,
        class: WorkClass,
        cancellation: &RuntimeCancellation,
    ) -> Result<AdmissionPermit, ()> {
        let ticket = {
            let mut state = lock(&self.state);
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1);
            state.waiters.push(Waiter {
                ticket,
                class,
                lane,
                bypassed: 0,
            });
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
                let free = state.active_in(lane) < state.budget.capacity(lane);
                if free && state.select(lane) == Some(ticket) {
                    state.grant(ticket, lane, class);
                    true
                } else {
                    false
                }
            };
            if granted {
                waiting.queued = false;
                // Capacity above one may admit the next waiter immediately.
                self.changed.notify_waiters();
                return Ok(AdmissionPermit {
                    controller: Arc::clone(self),
                    ticket,
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => return Err(()),
                _ = notified => {}
            }
        }
    }

    fn job_slot_must_yield(&self, class: WorkClass) -> bool {
        let state = lock(&self.state);
        if state.playback_sessions > 0 && !class.allowed_during_playback() {
            return true;
        }
        let full = state.active_in(Lane::Job) >= state.budget.capacity(Lane::Job);
        full && state
            .waiters
            .iter()
            .any(|w| w.lane == Lane::Job && state.eligible(w) && w.class.rank() > class.rank())
    }

    pub fn snapshot(&self) -> AdmissionSnapshot {
        let state = lock(&self.state);
        let count = |lane: Lane| state.waiters.iter().filter(|w| w.lane == lane).count();
        AdmissionSnapshot {
            playback_sessions: state.playback_sessions,
            queued_work: count(Lane::Model),
            active_work_class: state
                .active
                .iter()
                .find(|(_, lane, _)| *lane == Lane::Model)
                .map(|(_, _, class)| *class),
            queued_jobs: count(Lane::Job),
            active_jobs: state.active_in(Lane::Job),
            suspended_work: state.waiters.iter().filter(|w| !state.eligible(w)).count(),
            budget: state.budget,
        }
    }
}

/// A running job's bounded-concurrency position. Dropping it (completion,
/// failure, panic unwind) frees the slot.
pub struct JobSlot {
    controller: Arc<AdmissionController>,
    class: WorkClass,
    permit: Option<AdmissionPermit>,
}

impl JobSlot {
    pub fn class(&self) -> WorkClass {
        self.class
    }

    /// Whether the next [`checkpoint`](Self::checkpoint) will release the slot:
    /// playback suspends this class, or higher-priority work is waiting while
    /// every slot is busy.
    pub fn must_yield(&self) -> bool {
        self.permit.is_none() || self.controller.job_slot_must_yield(self.class)
    }

    /// Cooperative preemption point. Call only between bounded units while no
    /// database transaction or exclusive resource is held. Returns `Err` once
    /// cancelled; the slot is then released.
    pub async fn checkpoint(&mut self, cancellation: &RuntimeCancellation) -> Result<(), ()> {
        if cancellation.is_cancelled() {
            self.permit = None;
            return Err(());
        }
        if !self.must_yield() {
            return Ok(());
        }
        self.permit = None;
        let permit = self
            .controller
            .acquire(Lane::Job, self.class, cancellation)
            .await?;
        self.permit = Some(permit);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionSnapshot {
    pub playback_sessions: usize,
    /// Model-lane waiters.
    pub queued_work: usize,
    /// Class currently holding the model lane.
    pub active_work_class: Option<WorkClass>,
    pub queued_jobs: usize,
    pub active_jobs: usize,
    /// Waiters (either lane) held back by active playback.
    pub suspended_work: usize,
    pub budget: ResourceBudget,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
