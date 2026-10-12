//! Rust-owned playback policy for the native (libmpv) session (bootstrap §24):
//! resume, progress persistence, completion and the next episode.
//!
//! The engine keeps executing commands and rendering; React keeps the
//! presentation. What used to be decided in the React player — seek to the
//! resume point, write progress on a timer, decide "finished", pick the next
//! episode — is decided here, from the engine's own state, so it can't be lost
//! to a webview reload, an unmount race or a stale callback.
//!
//! [`SessionPolicy`] is the pure decision core (state snapshot in, actions
//! out). [`PlaybackCoordinator`] runs it on one ordered actor task: engine
//! wakeups only *poke* the actor, which snapshots the engine itself, so
//! snapshots are handled in order and never delay the `mpv:state` stream;
//! `begin`/`finish` go through the same queue, so a session's final write
//! can't race its own progress writes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Instant;

use nova_core::catalog::{EntityId, VariantId};
use nova_core::SeriesEpisode;
use nova_mpv_player::{MpvEngine, MpvPlaybackPhase, MpvPlayerState};
use nova_storage::{
    CanonicalRepository, ChannelRepository, ProfileSettingsRepository, SeriesRepository,
    WatchHistoryRepository,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};

/// Progress is persisted at most this often while playing.
pub const PROGRESS_INTERVAL_MS: i64 = 5_000;
/// Watched this share of the duration counts as finished.
pub const COMPLETED_THRESHOLD: f64 = 0.95;
/// A stored position this close to the start isn't worth resuming.
const MIN_RESUME_SEC: f64 = 5.0;
/// The resume seek counts as done once the reported position is this close.
const RESUME_TOLERANCE_SEC: f64 = 2.0;
/// Re-issue the resume seek when it hasn't taken effect after this long.
const RESUME_RETRY_MS: i64 = 2_000;
/// Give up resuming after this many seeks: playback continues from where the
/// engine actually is, and that becomes the stored position.
const RESUME_MAX_SEEKS: u32 = 3;
/// A source counts as working once it has played this long (a source that
/// plays briefly and then fails is not "healthy").
const HEALTHY_AFTER_MS: i64 = 20_000;
/// Recovery per title: one retry of the same item, then at most this many
/// switches to other sources...
const MAX_SOURCE_SWITCHES: u32 = 3;
/// ...and never more than this many recoveries in [`RECOVERY_WINDOW_MS`],
/// however often a source plays and fails again.
const MAX_RECOVERIES_PER_WINDOW: u32 = 5;
const RECOVERY_WINDOW_MS: i64 = 10 * 60 * 1000;
/// After a wake, how long to give Wi-Fi to come back before reconnecting a
/// live channel (a reconnect before the network is up would just fail).
const WAKE_SETTLE_MS: u64 = 3_000;

/// What the policy asks the coordinator to do.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyAction {
    Seek(f64),
    Record {
        position_sec: f64,
        duration_sec: Option<f64>,
        completed: bool,
    },
    /// The media played to its end (emitted once per session).
    Ended,
    /// The source has played for [`HEALTHY_AFTER_MS`] (once per session).
    Healthy,
    /// The engine reported an error (once per session): try to recover.
    Failed,
}

#[derive(Debug)]
struct PendingResume {
    at: f64,
    seeks: u32,
    last_seek_ms: i64,
}

/// The playback decisions of one loaded media item.
#[derive(Debug)]
pub struct SessionPolicy {
    live: bool,
    resume_at: Option<f64>,
    pending: Option<PendingResume>,
    ready: bool,
    loaded_position: Option<f64>,
    ended: bool,
    failed: bool,
    healthy: bool,
    /// When playback was first seen after the resume took effect.
    first_play_ms: Option<i64>,
    /// Playing or buffering on the previous snapshot (a pause after it is
    /// written at once).
    active: bool,
    last_write_ms: Option<i64>,
    position_sec: f64,
    duration_sec: Option<f64>,
}

impl SessionPolicy {
    /// An explicit dialogue timestamp takes precedence even for completed
    /// titles and moments near the beginning. Keep one second of lead-in.
    fn with_moment(mut self, start_sec: Option<f64>) -> Self {
        if !self.live {
            if let Some(start) = start_sec.filter(|s| s.is_finite() && *s >= 0.0) {
                self.resume_at = Some((start - 1.0).max(0.0));
            }
        }
        self
    }

    /// `resume` is the stored `(position, completed)`, if any.
    pub fn new(live: bool, resume: Option<(f64, bool)>) -> Self {
        let resume_at = resume
            .filter(|(position, completed)| !live && !completed && *position > MIN_RESUME_SEC)
            .map(|(position, _)| position);
        Self {
            live,
            resume_at,
            pending: None,
            ready: false,
            loaded_position: None,
            ended: false,
            failed: false,
            healthy: false,
            first_play_ms: None,
            active: false,
            last_write_ms: None,
            position_sec: 0.0,
            duration_sec: None,
        }
    }

    /// Actual engine position only once startup resume has settled. A source
    /// without a duration can still caption when there is no seek to perform.
    fn caption_position(&self) -> Option<f64> {
        if self.failed || self.ended || self.resume_at.is_some() || self.pending.is_some() {
            return None;
        }
        self.loaded_position
    }

    /// A live stream that was playing: after the Mac sleeps its connection
    /// is gone, so it reconnects (at the live edge) on wake.
    pub fn reconnects_on_wake(&self) -> bool {
        self.live && self.ready && self.active && !self.ended && !self.failed
    }

    fn completed(&self, position_sec: f64) -> bool {
        !self.live
            && self
                .duration_sec
                .filter(|d| *d > 0.0)
                .is_some_and(|d| position_sec / d >= COMPLETED_THRESHOLD)
    }

    fn record(&mut self, now_ms: i64) -> PolicyAction {
        self.last_write_ms = Some(now_ms);
        PolicyAction::Record {
            position_sec: self.position_sec,
            duration_sec: self.duration_sec,
            completed: self.completed(self.position_sec),
        }
    }

    /// Decide on one engine snapshot of this session. `now_ms` is monotonic.
    pub fn observe(
        &mut self,
        phase: MpvPlaybackPhase,
        position_sec: f64,
        duration_sec: Option<f64>,
        now_ms: i64,
    ) -> Vec<PolicyAction> {
        let mut actions = Vec::new();
        if matches!(phase, MpvPlaybackPhase::Error) {
            if !self.failed && !self.ended {
                self.failed = true;
                actions.push(PolicyAction::Failed);
            }
            return actions;
        }
        if self.ended || self.failed {
            return actions;
        }
        if matches!(phase, MpvPlaybackPhase::Playing | MpvPlaybackPhase::Paused)
            && position_sec.is_finite()
            && position_sec >= 0.0
        {
            self.loaded_position = Some(position_sec);
        }
        let was_active = self.active;
        self.active = matches!(
            phase,
            MpvPlaybackPhase::Playing | MpvPlaybackPhase::Buffering
        );
        if let Some(d) = duration_sec.filter(|d| d.is_finite() && *d > 0.0) {
            self.duration_sec = Some(d);
        }

        if !self.ready {
            let loaded = matches!(
                phase,
                MpvPlaybackPhase::Playing | MpvPlaybackPhase::Paused | MpvPlaybackPhase::Ended
            );
            // A VOD item is ready once the engine knows its duration (before
            // that a seek is rejected and positions are meaningless).
            if !loaded || (!self.live && self.duration_sec.is_none()) {
                return actions;
            }
            self.ready = true;
            if let Some(at) = self.resume_at.take() {
                self.pending = Some(PendingResume {
                    at,
                    seeks: 1,
                    last_seek_ms: now_ms,
                });
                self.position_sec = at;
                actions.push(PolicyAction::Seek(at));
                return actions;
            }
        }

        // Resume in flight: the stored position stands until the engine
        // reports the seek; re-seek a bounded number of times.
        if let Some(pending) = self.pending.as_mut() {
            if position_sec.is_finite() && position_sec >= pending.at - RESUME_TOLERANCE_SEC {
                self.pending = None;
            } else if now_ms - pending.last_seek_ms >= RESUME_RETRY_MS {
                if pending.seeks < RESUME_MAX_SEEKS {
                    pending.seeks += 1;
                    pending.last_seek_ms = now_ms;
                    actions.push(PolicyAction::Seek(pending.at));
                    return actions;
                }
                self.pending = None; // the engine won't resume: keep playing
            } else {
                return actions;
            }
        }

        if position_sec.is_finite() {
            self.position_sec = position_sec;
        }
        if matches!(phase, MpvPlaybackPhase::Playing) {
            let since = *self.first_play_ms.get_or_insert(now_ms);
            if !self.healthy && now_ms - since >= HEALTHY_AFTER_MS {
                self.healthy = true;
                actions.push(PolicyAction::Healthy);
            }
        }
        match phase {
            MpvPlaybackPhase::Ended => {
                self.ended = true;
                let end = self.duration_sec.unwrap_or(self.position_sec);
                self.position_sec = end;
                self.last_write_ms = Some(now_ms);
                actions.push(PolicyAction::Record {
                    position_sec: end,
                    duration_sec: self.duration_sec,
                    completed: !self.live,
                });
                actions.push(PolicyAction::Ended);
            }
            MpvPlaybackPhase::Paused if was_active => actions.push(self.record(now_ms)),
            MpvPlaybackPhase::Playing
                if self
                    .last_write_ms
                    .is_none_or(|last| now_ms - last >= PROGRESS_INTERVAL_MS) =>
            {
                actions.push(self.record(now_ms))
            }
            _ => {}
        }
        actions
    }

    /// The final write when the session closes or is replaced: nothing if it
    /// never became ready, already ended, or its resume seek hasn't taken
    /// effect (the stored position is the right one then).
    pub fn finish(&mut self, now_ms: i64) -> Option<PolicyAction> {
        if !self.ready || self.ended || self.pending.is_some() {
            return None;
        }
        Some(self.record(now_ms))
    }
}

/// `playback:ended` — the loaded media played to its end.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEnded {
    pub token: String,
    /// The episode after this one, when it is an episode with a successor.
    pub next_episode: Option<SeriesEpisode>,
    /// The profile's "autoplay next episode" setting, read at the end.
    pub autoplay: bool,
}

/// Why the player is asked to reopen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverReason {
    /// The engine reported an error.
    Failure,
    /// The Mac woke from sleep while a live channel was playing.
    Wake,
}

/// `playback:recover` — the native session failed; the coordinator chose
/// what to try next (the same item once, then another source of the title).
/// Also sent on wake to reconnect a live channel (`reason: wake`, outside
/// the recovery budget).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackRecover {
    /// The failed session.
    pub token: String,
    pub media_kind: String,
    pub media_id: i64,
    /// 1-based recovery attempt for this title.
    pub attempt: u32,
    /// Another source of the title rather than a retry of the same one.
    pub other_source: bool,
    pub reason: RecoverReason,
}

/// What the coordinator announces to the player.
#[derive(Debug, Clone)]
pub enum PlaybackEvent {
    Ended(Box<PlaybackEnded>),
    Recover(PlaybackRecover),
}

/// Where playback events go (the Tauri event bus; a channel in tests).
pub type EventSink = Arc<dyn Fn(PlaybackEvent) + Send + Sync>;

struct ActiveSession {
    token: String,
    pool: Arc<SqlitePool>,
    media_kind: String,
    media_id: i64,
    /// A movie's canonical entity and the variant being played.
    variant: Option<(EntityId, VariantId)>,
    live_identity: Option<(i64, String, String)>,
    preferred_lang: Option<String>,
    caption_mode: Option<String>,
    policy: SessionPolicy,
}

impl ActiveSession {
    /// Recovery is tracked per title: the entity for a movie, else the item.
    fn chain_key(&self) -> String {
        match &self.variant {
            Some((entity, _)) => entity.as_str().to_string(),
            None => self
                .live_identity
                .as_ref()
                .map(|(source, key, _)| format!("live:{source}:{key}"))
                .unwrap_or_else(|| format!("{}:{}", self.media_kind, self.media_id)),
        }
    }
}

/// What recovery has tried for one title.
#[derive(Debug, Default)]
struct RecoveryChain {
    retried_same: bool,
    switches: u32,
    tried_variants: Vec<VariantId>,
    tried_live: Vec<String>,
    window_start_ms: i64,
    recoveries_in_window: u32,
}

impl RecoveryChain {
    /// The title played for a while: its sources get a fresh start, but the
    /// per-window cap keeps counting (a play-then-fail loop stays bounded).
    fn source_worked(&mut self) {
        self.retried_same = false;
        self.switches = 0;
        self.tried_variants.clear();
        self.tried_live.clear();
    }
}

enum Msg {
    /// The engine woke up: snapshot it and decide.
    Poke,
    /// Decide on a given snapshot at a given monotonic time (tests; the
    /// engine path uses `Poke`).
    Observe(Box<MpvPlayerState>, i64, oneshot::Sender<()>),
    Begin {
        token: String,
        pool: Arc<SqlitePool>,
        media_kind: String,
        media_id: i64,
        start_sec: Option<f64>,
        done: oneshot::Sender<()>,
    },
    Finish {
        token: Option<String>,
        done: oneshot::Sender<()>,
    },
    CaptionPosition {
        token: String,
        done: oneshot::Sender<Option<f64>>,
    },
    CaptionMode {
        token: String,
        mode: Option<String>,
    },
    /// The Mac is going to sleep (`done` for tests).
    Sleep(Option<oneshot::Sender<()>>),
    /// The Mac woke up (`done` for tests).
    Wake(Option<oneshot::Sender<()>>),
    /// The network had time to come back after a wake: reconnect `token`
    /// unless the Mac slept again since (`sleep_seq`).
    Reconnect {
        token: String,
        sleep_seq: u64,
    },
}

/// Owns the one native playback session's policy, on an ordered actor task.
pub struct PlaybackCoordinator {
    tx: mpsc::UnboundedSender<Msg>,
    rx: StdMutex<Option<mpsc::UnboundedReceiver<Msg>>>,
    engine: Arc<MpvEngine>,
    sink: OnceLock<EventSink>,
    clock: Instant,
    /// How long a wake waits before reconnecting a live channel.
    wake_settle_ms: AtomicU64,
}

impl PlaybackCoordinator {
    pub fn new(engine: Arc<MpvEngine>) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            tx,
            rx: StdMutex::new(Some(rx)),
            engine,
            sink: OnceLock::new(),
            clock: Instant::now(),
            wake_settle_ms: AtomicU64::new(WAKE_SETTLE_MS),
        })
    }

    /// Start the actor (once) with where playback events go.
    pub fn start(self: &Arc<Self>, sink: EventSink) {
        let Some(rx) = self.rx.lock().ok().and_then(|mut rx| rx.take()) else {
            return;
        };
        let _ = self.sink.set(sink);
        let this = self.clone();
        tauri::async_runtime::spawn(async move { this.run(rx).await });
    }

    /// The engine has new state; never blocks.
    pub fn poke(&self) {
        let _ = self.tx.send(Msg::Poke);
    }

    /// Start the policy of a freshly opened engine token, finishing any
    /// session still active.
    pub async fn begin(
        &self,
        token: String,
        pool: Arc<SqlitePool>,
        media_kind: String,
        media_id: i64,
        start_sec: Option<f64>,
    ) {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Msg::Begin {
            token,
            pool,
            media_kind,
            media_id,
            start_sec,
            done,
        });
        let _ = wait.await;
    }

    /// Final progress write of the active session (closing, or replaced by
    /// other media), then forget it. `token` limits this to that session.
    pub async fn finish(&self, token: Option<&str>) {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Msg::Finish {
            token: token.map(str::to_string),
            done,
        });
        let _ = wait.await;
    }

    /// Read startup readiness in order with native snapshots and resume seeks.
    pub async fn caption_position(&self, token: &str) -> Option<f64> {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Msg::CaptionPosition {
            token: token.to_string(),
            done,
        });
        wait.await.ok().flatten()
    }

    /// The user's caption selection, persisted with the session's progress.
    pub fn set_caption_mode(&self, token: &str, mode: Option<String>) {
        let _ = self.tx.send(Msg::CaptionMode {
            token: token.to_string(),
            mode,
        });
    }

    /// The Mac is about to sleep: pause the native session (the pause writes
    /// its progress) and remember whether a live stream should reconnect.
    /// Never blocks: called on the main thread by the OS notification, and
    /// queued in order with the wake that follows.
    pub fn notify_sleep(&self) {
        let _ = self.tx.send(Msg::Sleep(None));
    }

    /// The Mac woke up: a live channel that was playing reconnects once the
    /// network had [`WAKE_SETTLE_MS`] to come back; anything else stays
    /// paused for the user. Never blocks.
    pub fn notify_wake(&self) {
        let _ = self.tx.send(Msg::Wake(None));
    }

    /// [`Self::notify_sleep`], returning once handled (tests).
    pub async fn sleep(&self) {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Msg::Sleep(Some(done)));
        let _ = wait.await;
    }

    /// [`Self::notify_wake`], returning once handled (tests).
    pub async fn wake(&self) {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Msg::Wake(Some(done)));
        let _ = wait.await;
    }

    /// Shorten the wake settle (tests).
    pub fn set_wake_settle_ms(&self, ms: u64) {
        self.wake_settle_ms.store(ms, Ordering::Relaxed);
    }

    /// Decide on one given snapshot, in order with everything else (tests).
    pub async fn observe(&self, state: MpvPlayerState) {
        self.observe_at(state, self.now_ms()).await;
    }

    /// [`Self::observe`] at a given monotonic time in ms (tests).
    pub async fn observe_at(&self, state: MpvPlayerState, now_ms: i64) {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Msg::Observe(Box::new(state), now_ms, done));
        let _ = wait.await;
    }

    fn now_ms(&self) -> i64 {
        self.clock.elapsed().as_millis() as i64
    }

    async fn run(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<Msg>) {
        let mut active: Option<ActiveSession> = None;
        let mut chains: HashMap<String, RecoveryChain> = HashMap::new();
        // The item a `playback:recover` asked the player to open next: when
        // it arrives its chain continues; any other open is the user's.
        let mut expected: Option<(String, i64)> = None;
        // The live session (by token) to reconnect when the Mac wakes, and
        // how many sleeps began (a reconnect after a newer sleep is stale).
        let mut reconnect_on_wake: Option<String> = None;
        let mut sleep_seq: u64 = 0;
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::Poke => {
                    let Some(session) = active.as_mut() else {
                        continue;
                    };
                    let Ok(state) = self.engine.state(Some(&session.token)) else {
                        continue; // a replaced/closed engine session
                    };
                    self.apply(session, &state, self.now_ms(), &mut chains, &mut expected)
                        .await;
                }
                Msg::Observe(state, now, done) => {
                    if let Some(session) = active.as_mut() {
                        self.apply(session, &state, now, &mut chains, &mut expected)
                            .await;
                    }
                    let _ = done.send(());
                }
                Msg::Begin {
                    token,
                    pool,
                    media_kind,
                    media_id,
                    start_sec,
                    done,
                } => {
                    if let Some(previous) = active.take() {
                        self.close_session(previous).await;
                    }
                    let from_recovery = expected.take() == Some((media_kind.clone(), media_id));
                    let session = Self::open_session(
                        token,
                        pool,
                        media_kind,
                        media_id,
                        if from_recovery { None } else { start_sec },
                    )
                    .await;
                    if !from_recovery {
                        // The user started this: a fresh set of recoveries.
                        chains.remove(&session.chain_key());
                    }
                    active = Some(session);
                    let _ = done.send(());
                }
                Msg::Finish { token, done } => {
                    let stale = token
                        .as_deref()
                        .is_some_and(|t| active.as_ref().is_some_and(|s| s.token != t));
                    if !stale {
                        if let Some(session) = active.take() {
                            self.close_session(session).await;
                        }
                        // A recovery the player never acted on doesn't make
                        // the user's next open of that item a recovery.
                        expected = None;
                    }
                    let _ = done.send(());
                }
                Msg::CaptionPosition { token, done } => {
                    let position = active
                        .as_ref()
                        .filter(|s| s.token == token)
                        .and_then(|s| s.policy.caption_position());
                    let _ = done.send(position);
                }
                Msg::CaptionMode { token, mode } => {
                    if let Some(session) = active.as_mut().filter(|s| s.token == token) {
                        session.caption_mode = mode;
                    }
                }
                Msg::Sleep(done) => {
                    sleep_seq += 1;
                    reconnect_on_wake = None;
                    if let Some(session) = active.as_ref() {
                        if session.policy.reconnects_on_wake() {
                            reconnect_on_wake = Some(session.token.clone());
                        }
                        if let Err(e) = self.engine.set_pause(&session.token, true) {
                            tracing::debug!(error = %e, "could not pause for sleep");
                        }
                    }
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
                Msg::Wake(done) => {
                    if let Some(token) = reconnect_on_wake.take() {
                        let tx = self.tx.clone();
                        let settle = self.wake_settle_ms.load(Ordering::Relaxed);
                        let seq = sleep_seq;
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(settle)).await;
                            let _ = tx.send(Msg::Reconnect {
                                token,
                                sleep_seq: seq,
                            });
                        });
                    }
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
                Msg::Reconnect {
                    token,
                    sleep_seq: seq,
                } => {
                    if seq != sleep_seq {
                        continue; // the Mac slept again since
                    }
                    if let Some(session) = active.as_ref().filter(|s| s.token == token) {
                        // A connection dropped by sleep isn't a failing
                        // source: the title gets a fresh recovery budget.
                        chains.remove(&session.chain_key());
                        expected = Some((session.media_kind.clone(), session.media_id));
                        self.emit(PlaybackEvent::Recover(PlaybackRecover {
                            token: session.token.clone(),
                            media_kind: session.media_kind.clone(),
                            media_id: session.media_id,
                            attempt: 0,
                            other_source: false,
                            reason: RecoverReason::Wake,
                        }));
                    }
                }
            }
        }
    }

    async fn open_session(
        token: String,
        pool: Arc<SqlitePool>,
        media_kind: String,
        media_id: i64,
        start_sec: Option<f64>,
    ) -> ActiveSession {
        let live = media_kind == "live";
        let variant = if media_kind == "vod" {
            let canonical = CanonicalRepository::new(&pool);
            match (
                canonical.entity_for_vod(media_id).await,
                canonical.variant_for_vod(media_id).await,
            ) {
                (Ok(Some(entity)), Ok(Some(variant))) => Some((entity, variant)),
                _ => None,
            }
        } else {
            None
        };
        let live_identity = if live {
            ChannelRepository::new(&pool)
                .live_channel_for_row(media_id)
                .await
                .ok()
                .flatten()
                .and_then(|c| {
                    c.variants
                        .iter()
                        .find(|v| v.channel_id == media_id)
                        .map(|v| (c.source_id, c.key.clone(), v.variant_key.clone()))
                })
        } else {
            None
        };
        let preferred_lang = ProfileSettingsRepository::new(&pool)
            .get()
            .await
            .ok()
            .and_then(|s| s.preferred_audio_lang);
        let resume = if live {
            None
        } else {
            WatchHistoryRepository::new(&pool)
                .resume_position(&media_kind, media_id)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "resume lookup failed; starting from the beginning");
                    None
                })
                .map(|r| (r.position_sec, r.completed))
        };
        ActiveSession {
            token,
            pool,
            media_kind,
            media_id,
            variant,
            live_identity,
            preferred_lang,
            caption_mode: None,
            policy: SessionPolicy::new(live, resume).with_moment(start_sec),
        }
    }

    async fn close_session(&self, mut session: ActiveSession) {
        if let Some(action) = session.policy.finish(self.now_ms()) {
            Self::record(&session, &action).await;
        }
    }

    async fn apply(
        &self,
        session: &mut ActiveSession,
        state: &MpvPlayerState,
        now_ms: i64,
        chains: &mut HashMap<String, RecoveryChain>,
        expected: &mut Option<(String, i64)>,
    ) {
        if state.token.as_deref() != Some(session.token.as_str()) {
            return; // a snapshot of a replaced session
        }
        let actions = session.policy.observe(
            state.phase.clone(),
            state.position_sec,
            state.duration_sec,
            now_ms,
        );
        for action in actions {
            match action {
                PolicyAction::Seek(at) => {
                    if let Err(e) = self.engine.seek_absolute(&session.token, at) {
                        tracing::warn!(error = %e, "resume seek rejected; will retry");
                    }
                }
                PolicyAction::Record { .. } => Self::record(session, &action).await,
                PolicyAction::Ended => self.announce_end(session),
                PolicyAction::Healthy => {
                    Self::record_outcome(session, true).await;
                    if let Some(chain) = chains.get_mut(&session.chain_key()) {
                        chain.source_worked();
                    }
                }
                PolicyAction::Failed => {
                    Self::record_outcome(session, false).await;
                    let chain = chains.entry(session.chain_key()).or_default();
                    if let Some(recover) = Self::next_attempt(session, chain, now_ms).await {
                        *expected = Some((recover.media_kind.clone(), recover.media_id));
                        self.emit(PlaybackEvent::Recover(recover));
                    }
                }
            }
        }
    }

    fn emit(&self, event: PlaybackEvent) {
        if let Some(sink) = self.sink.get() {
            sink(event);
        }
    }

    async fn record_outcome(session: &ActiveSession, success: bool) {
        if let Some((source, key, variant)) = &session.live_identity {
            if let Err(e) = ChannelRepository::new(&session.pool)
                .record_live_variant(*source, key, variant, success)
                .await
            {
                tracing::warn!(error = %e, "could not record live variant health");
            }
            return;
        }
        let Some((_, variant)) = &session.variant else {
            return;
        };
        if let Err(e) = CanonicalRepository::new(&session.pool)
            .record_variant_outcome(variant, success)
            .await
        {
            tracing::warn!(error = %e, "could not record variant health");
        }
    }

    /// Bounded recovery (bootstrap §25): one retry of the same item per title
    /// (transient failures), then the next-ranked untried source, at most
    /// [`MAX_SOURCE_SWITCHES`] times, and never more than
    /// [`MAX_RECOVERIES_PER_WINDOW`] recoveries per [`RECOVERY_WINDOW_MS`].
    /// `None` leaves the error to the user.
    async fn next_attempt(
        session: &ActiveSession,
        chain: &mut RecoveryChain,
        now_ms: i64,
    ) -> Option<PlaybackRecover> {
        if chain.recoveries_in_window == 0 || now_ms - chain.window_start_ms >= RECOVERY_WINDOW_MS {
            chain.window_start_ms = now_ms;
            chain.recoveries_in_window = 0;
        }
        if chain.recoveries_in_window >= MAX_RECOVERIES_PER_WINDOW {
            return None;
        }
        let (media_id, other_source) = if !chain.retried_same {
            chain.retried_same = true;
            (session.media_id, false)
        } else if let Some((source, key, variant)) = &session.live_identity {
            if chain.switches >= MAX_SOURCE_SWITCHES {
                return None;
            }
            if !chain.tried_live.contains(variant) {
                chain.tried_live.push(variant.clone());
            }
            let next = ChannelRepository::new(&session.pool)
                .resolve_live(*source, key, None, &chain.tried_live)
                .await
                .ok()?;
            chain.switches += 1;
            (next.channel_id, true)
        } else {
            let (entity, variant) = session.variant.as_ref()?;
            if chain.switches >= MAX_SOURCE_SWITCHES {
                return None;
            }
            if !chain.tried_variants.contains(variant) {
                chain.tried_variants.push(variant.clone());
            }
            let next = CanonicalRepository::new(&session.pool)
                .resolve_playback_with(
                    entity,
                    None,
                    session.preferred_lang.as_deref(),
                    &chain.tried_variants,
                )
                .await
                .ok()
                .flatten()?;
            chain.switches += 1;
            (next.media_id, true)
        };
        chain.recoveries_in_window += 1;
        Some(PlaybackRecover {
            token: session.token.clone(),
            media_kind: session.media_kind.clone(),
            media_id,
            attempt: chain.recoveries_in_window,
            other_source,
            reason: RecoverReason::Failure,
        })
    }

    /// Look up what follows and emit `playback:ended` off the actor, so a
    /// provider fetch for the series can't hold up the queue.
    fn announce_end(&self, session: &ActiveSession) {
        let Some(sink) = self.sink.get().cloned() else {
            return;
        };
        let (token, pool, kind, id) = (
            session.token.clone(),
            session.pool.clone(),
            session.media_kind.clone(),
            session.media_id,
        );
        tauri::async_runtime::spawn(async move {
            let next_episode = if kind == "episode" {
                next_episode(&pool, id).await
            } else {
                None
            };
            let autoplay = ProfileSettingsRepository::new(&pool)
                .get()
                .await
                .map(|s| s.autoplay_next)
                .unwrap_or(true);
            sink(PlaybackEvent::Ended(Box::new(PlaybackEnded {
                token,
                next_episode,
                autoplay,
            })));
        });
    }

    async fn record(session: &ActiveSession, action: &PolicyAction) {
        let PolicyAction::Record {
            position_sec,
            duration_sec,
            completed,
        } = *action
        else {
            return;
        };
        if let Err(e) = WatchHistoryRepository::new(&session.pool)
            .record_progress(
                &session.media_kind,
                session.media_id,
                position_sec,
                duration_sec,
                completed,
                session.caption_mode.as_deref(),
            )
            .await
        {
            tracing::warn!(error = %e, "could not persist playback progress");
        }
    }
}

/// The episode after `episode_row` in its series, by (season, episode).
pub(crate) async fn next_episode(pool: &SqlitePool, episode_row: i64) -> Option<SeriesEpisode> {
    let current = SeriesRepository::new(pool)
        .get_episode(episode_row)
        .await
        .ok()
        .flatten()?;
    let mut siblings =
        crate::commands::fetch_series_episodes(pool, current.source_id, &current.series_title)
            .await
            .ok()?;
    siblings.sort_by_key(|e| (e.season, e.episode));
    let at = siblings.iter().position(|e| e.id == episode_row)?;
    siblings.into_iter().nth(at + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use MpvPlaybackPhase::*;

    const D: Option<f64> = Some(2400.0);

    fn records(actions: &[PolicyAction]) -> Vec<(f64, bool)> {
        actions
            .iter()
            .filter_map(|a| match a {
                PolicyAction::Record {
                    position_sec,
                    completed,
                    ..
                } => Some((*position_sec, *completed)),
                _ => None,
            })
            .collect()
    }

    fn seeks(actions: &[PolicyAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, PolicyAction::Seek(_)))
            .count()
    }

    #[test]
    fn nothing_is_decided_until_the_engine_knows_the_media() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        assert!(p.observe(Opening, 0.0, None, 0).is_empty());
        assert!(p.observe(Buffering, 0.0, None, 100).is_empty());
        // The engine says "Playing" while still loading: no duration yet.
        assert!(p.observe(Playing, 0.0, None, 200).is_empty());
        assert_eq!(p.finish(300), None, "never ready: nothing to write");
        assert_eq!(
            p.observe(Playing, 0.0, D, 400),
            vec![PolicyAction::Seek(600.0)]
        );
    }

    #[test]
    fn captions_wait_for_resume_acknowledgement_or_bounded_failure() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        assert_eq!(p.caption_position(), None);
        p.observe(Playing, 0.0, D, 0);
        assert_eq!(p.caption_position(), None);
        p.observe(Playing, 0.5, D, 500);
        assert_eq!(p.caption_position(), None);
        p.observe(Playing, 600.1, D, 600);
        assert_eq!(p.caption_position(), Some(600.1));
        let mut failed = SessionPolicy::new(false, Some((600.0, false)));
        for now in [0, 2000, 4000] {
            failed.observe(Playing, 1.0, D, now);
            assert_eq!(failed.caption_position(), None);
        }
        failed.observe(Playing, 6.0, D, 6000);
        assert_eq!(failed.caption_position(), Some(6.0));
        let mut unknown_duration = SessionPolicy::new(false, None);
        unknown_duration.observe(Playing, 2.0, None, 0);
        assert_eq!(unknown_duration.caption_position(), Some(2.0));
    }

    #[test]
    fn a_moment_overrides_resume_and_retries_only_its_own_position() {
        for completed in [false, true] {
            let mut p = SessionPolicy::new(false, Some((600.0, completed))).with_moment(Some(42.0));
            assert!(
                p.observe(Playing, 0.0, None, 0).is_empty(),
                "wait for duration"
            );
            assert_eq!(
                p.observe(Playing, 0.0, D, 100),
                vec![PolicyAction::Seek(41.0)]
            );
            assert_eq!(
                p.observe(Playing, 0.0, D, 2_200),
                vec![PolicyAction::Seek(41.0)]
            );
            assert_eq!(
                records(&p.observe(Playing, 41.2, D, 2_500)),
                vec![(41.2, false)]
            );
            assert_eq!(seeks(&p.observe(Playing, 43.0, D, 3_000)), 0);
        }
        let mut beginning = SessionPolicy::new(false, Some((600.0, false))).with_moment(Some(0.0));
        assert_eq!(
            beginning.observe(Playing, 0.0, D, 0),
            vec![PolicyAction::Seek(0.0)]
        );
        let mut live = SessionPolicy::new(true, None).with_moment(Some(42.0));
        assert_eq!(seeks(&live.observe(Playing, 0.0, D, 0)), 0);
    }

    #[test]
    fn resumes_then_writes_every_five_seconds_while_playing() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        assert_eq!(
            p.observe(Playing, 0.0, D, 1_000),
            vec![PolicyAction::Seek(600.0)]
        );
        // Until the engine reports the seek, the start is never written.
        assert!(p.observe(Playing, 0.4, D, 1_500).is_empty());
        assert_eq!(p.finish(1_600), None);
        assert_eq!(
            records(&p.observe(Playing, 600.2, D, 1_900)),
            vec![(600.2, false)]
        );
        assert!(p.observe(Playing, 603.0, D, 4_000).is_empty());
        assert_eq!(
            records(&p.observe(Playing, 606.0, D, 7_000)),
            vec![(606.0, false)]
        );
        assert_eq!(
            seeks(&p.observe(Playing, 607.0, D, 7_100)),
            0,
            "resumes once"
        );
    }

    #[test]
    fn a_rejected_resume_seek_is_retried_and_never_overwrites_the_resume_point() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        assert_eq!(seeks(&p.observe(Playing, 0.0, D, 0)), 1);
        assert!(p.observe(Playing, 1.0, D, 1_000).is_empty());
        assert_eq!(seeks(&p.observe(Playing, 2.0, D, 2_100)), 1, "retry");
        assert_eq!(seeks(&p.observe(Playing, 4.0, D, 4_200)), 1, "last retry");
        assert_eq!(p.finish(4_300), None, "still the stored position");
        // Three rejected seeks: playback continues from where it really is.
        let given_up = p.observe(Playing, 6.5, D, 6_300);
        assert_eq!(seeks(&given_up), 0);
        assert_eq!(records(&given_up), vec![(6.5, false)]);
    }

    #[test]
    fn a_finished_or_near_start_position_is_not_resumed() {
        for resume in [Some((2390.0, true)), Some((3.0, false)), None] {
            let mut p = SessionPolicy::new(false, resume);
            assert_eq!(seeks(&p.observe(Playing, 0.0, D, 0)), 0);
        }
    }

    #[test]
    fn pausing_writes_immediately_even_from_buffering_and_finish_writes_the_last_position() {
        let mut p = SessionPolicy::new(false, None);
        p.observe(Playing, 10.0, D, 0);
        assert_eq!(
            records(&p.observe(Paused, 12.0, D, 1_000)),
            vec![(12.0, false)]
        );
        assert!(p.observe(Paused, 12.0, D, 9_000).is_empty());
        p.observe(Buffering, 20.0, D, 9_100);
        assert_eq!(
            records(&p.observe(Paused, 21.0, D, 9_200)),
            vec![(21.0, false)]
        );
        p.observe(Playing, 30.0, D, 9_500);
        assert_eq!(
            p.finish(10_000),
            Some(PolicyAction::Record {
                position_sec: 30.0,
                duration_sec: D,
                completed: false
            })
        );
    }

    #[test]
    fn completion_is_decided_by_the_threshold_and_by_the_end() {
        let mut p = SessionPolicy::new(false, None);
        p.observe(Playing, 0.0, Some(1000.0), 0);
        assert_eq!(
            records(&p.observe(Paused, 951.0, Some(1000.0), 100)),
            vec![(951.0, true)]
        );
        let end = p.observe(Ended, 1000.0, Some(1000.0), 200);
        assert_eq!(records(&end), vec![(1000.0, true)]);
        assert!(end.contains(&PolicyAction::Ended));
        assert!(
            p.observe(Ended, 1000.0, Some(1000.0), 300).is_empty(),
            "once"
        );
        assert!(
            p.observe(Playing, 500.0, Some(1000.0), 400).is_empty(),
            "late snapshot"
        );
        assert_eq!(p.finish(400), None, "already written at the end");
    }

    #[test]
    fn live_is_never_resumed_or_completed() {
        let mut p = SessionPolicy::new(true, Some((600.0, false)));
        let first = p.observe(Playing, 0.0, None, 0);
        assert_eq!(records(&first), vec![(0.0, false)]);
        assert_eq!(seeks(&first), 0);
        let end = p.observe(Ended, 50.0, None, 100);
        assert_eq!(records(&end), vec![(50.0, false)]);
    }

    #[test]
    fn an_error_asks_for_recovery_once_and_decides_nothing_else() {
        let mut p = SessionPolicy::new(false, None);
        p.observe(Playing, 10.0, Some(100.0), 0);
        assert_eq!(
            p.observe(Error, 0.0, None, 10_000),
            vec![PolicyAction::Failed]
        );
        assert!(p.observe(Error, 0.0, None, 10_100).is_empty(), "once");
        assert!(
            p.observe(Playing, 50.0, Some(100.0), 20_000).is_empty(),
            "no writes after"
        );
        assert_eq!(
            records(&p.finish(10_000).into_iter().collect::<Vec<_>>()),
            vec![(10.0, false)],
            "the last good position is kept"
        );
    }

    #[test]
    fn a_session_that_ended_or_never_loaded_does_not_ask_for_recovery_twice() {
        let mut p = SessionPolicy::new(false, None);
        assert_eq!(
            p.observe(Error, 0.0, None, 0),
            vec![PolicyAction::Failed],
            "open failed"
        );
        let mut done = SessionPolicy::new(false, None);
        done.observe(Playing, 0.0, D, 0);
        done.observe(Ended, 2400.0, D, 100);
        assert!(
            done.observe(Error, 0.0, None, 200).is_empty(),
            "already finished"
        );
    }

    #[test]
    fn a_source_is_healthy_only_after_it_has_really_played() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        p.observe(Playing, 0.0, D, 0); // resume seek
        assert!(!p
            .observe(Playing, 600.0, D, 1_000)
            .contains(&PolicyAction::Healthy));
        assert!(!p
            .observe(Playing, 610.0, D, 20_500)
            .contains(&PolicyAction::Healthy));
        assert!(p
            .observe(Playing, 621.0, D, 21_000)
            .contains(&PolicyAction::Healthy));
        assert!(
            !p.observe(Playing, 630.0, D, 30_000)
                .contains(&PolicyAction::Healthy),
            "once"
        );
        // Playing briefly before failing never counts as healthy.
        let mut brief = SessionPolicy::new(true, None);
        assert!(!brief
            .observe(Playing, 0.0, None, 0)
            .contains(&PolicyAction::Healthy));
        assert_eq!(
            brief.observe(Error, 0.0, None, 1_000),
            vec![PolicyAction::Failed]
        );
    }
}
