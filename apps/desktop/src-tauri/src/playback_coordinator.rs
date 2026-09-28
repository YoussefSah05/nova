//! Rust-owned playback policy for the native (libmpv) session (bootstrap §24):
//! resume, progress persistence, completion and the next episode.
//!
//! The engine keeps executing commands and rendering; React keeps the
//! presentation. What used to be decided in the React player — seek to the
//! resume point, write progress on a timer, decide "finished", pick the next
//! episode — is decided here, from the engine's own state snapshots, so it
//! can't be lost to a webview reload, an unmount race or a stale callback.
//!
//! [`SessionPolicy`] is the pure decision core (state snapshot in, actions
//! out), tested with synthetic states; [`PlaybackCoordinator`] owns the one
//! active session and executes the actions against storage and the engine.

use std::sync::Arc;

use nova_core::SeriesEpisode;
use nova_mpv_player::{MpvEngine, MpvPlaybackPhase, MpvPlayerState};
use nova_storage::{ProfileSettingsRepository, SeriesRepository, WatchHistoryRepository};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

/// Progress is persisted at most this often while playing.
pub const PROGRESS_INTERVAL_MS: i64 = 5_000;
/// Watched this share of the duration counts as finished.
pub const COMPLETED_THRESHOLD: f64 = 0.95;
/// A stored position this close to the start isn't worth resuming.
const MIN_RESUME_SEC: f64 = 5.0;
/// After the resume seek, positions are not written until the engine has had
/// time to report the new position (so a stale 0:00 can't overwrite it).
const RESUME_SETTLE_MS: i64 = 3_000;

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
}

/// The playback decisions of one loaded media item.
#[derive(Debug)]
pub struct SessionPolicy {
    live: bool,
    resume_at: Option<f64>,
    ready: bool,
    ended: bool,
    playing: bool,
    last_write_ms: Option<i64>,
    settle_until_ms: i64,
    position_sec: f64,
    duration_sec: Option<f64>,
}

impl SessionPolicy {
    /// `resume` is the stored `(position, completed)`, if any.
    pub fn new(live: bool, resume: Option<(f64, bool)>) -> Self {
        let resume_at = resume
            .filter(|(position, completed)| !live && !completed && *position > MIN_RESUME_SEC)
            .map(|(position, _)| position);
        Self {
            live,
            resume_at,
            ready: false,
            ended: false,
            playing: false,
            last_write_ms: None,
            settle_until_ms: 0,
            position_sec: 0.0,
            duration_sec: None,
        }
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

    /// Decide on one engine snapshot of this session.
    pub fn observe(
        &mut self,
        phase: MpvPlaybackPhase,
        position_sec: f64,
        duration_sec: Option<f64>,
        now_ms: i64,
    ) -> Vec<PolicyAction> {
        let mut actions = Vec::new();
        if self.ended || matches!(phase, MpvPlaybackPhase::Error) {
            return actions;
        }
        let was_playing = self.playing;
        self.playing = matches!(phase, MpvPlaybackPhase::Playing);
        let loaded = matches!(
            phase,
            MpvPlaybackPhase::Playing | MpvPlaybackPhase::Paused | MpvPlaybackPhase::Ended
        );
        if !self.ready {
            if !loaded {
                return actions; // still opening: positions are meaningless
            }
            self.ready = true;
            if let Some(at) = self.resume_at.take() {
                actions.push(PolicyAction::Seek(at));
                self.settle_until_ms = now_ms + RESUME_SETTLE_MS;
                // The stored position stands until the engine reports the seek.
                self.position_sec = at;
            }
        }
        let settling = now_ms < self.settle_until_ms;
        if position_sec.is_finite() && !settling {
            self.position_sec = position_sec;
        }
        if let Some(d) = duration_sec.filter(|d| d.is_finite() && *d > 0.0) {
            self.duration_sec = Some(d);
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
            _ if settling => {}
            MpvPlaybackPhase::Paused if was_playing => actions.push(self.record(now_ms)),
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
    /// never loaded, already ended, or is still settling after the resume seek
    /// (the stored position is the right one then).
    pub fn finish(&mut self, now_ms: i64) -> Option<PolicyAction> {
        if !self.ready || self.ended || now_ms < self.settle_until_ms {
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
    /// The profile's "autoplay next episode" setting.
    pub autoplay: bool,
}

struct ActiveSession {
    token: String,
    pool: Arc<SqlitePool>,
    media_kind: String,
    media_id: i64,
    caption_mode: Option<String>,
    autoplay: bool,
    policy: SessionPolicy,
}

/// Owns the one native playback session's policy.
pub struct PlaybackCoordinator {
    engine: Arc<MpvEngine>,
    active: Mutex<Option<ActiveSession>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

impl PlaybackCoordinator {
    pub fn new(engine: Arc<MpvEngine>) -> Self {
        Self {
            engine,
            active: Mutex::new(None),
        }
    }

    /// Start the policy of a freshly opened engine token. The previous
    /// session must have been finished ([`Self::finish`]) first.
    pub async fn begin(
        &self,
        token: String,
        pool: Arc<SqlitePool>,
        media_kind: String,
        media_id: i64,
    ) {
        let live = media_kind == "live";
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
        let autoplay = ProfileSettingsRepository::new(&pool)
            .get()
            .await
            .map(|s| s.autoplay_next)
            .unwrap_or(true);
        *self.active.lock().await = Some(ActiveSession {
            token,
            pool,
            media_kind,
            media_id,
            caption_mode: None,
            autoplay,
            policy: SessionPolicy::new(live, resume),
        });
    }

    /// Final progress write of the active session (closing, or replacing it
    /// with other media), then forget it. `token` limits this to that
    /// session; `None` finishes whatever is active.
    pub async fn finish(&self, token: Option<&str>) {
        let mut active = self.active.lock().await;
        if token.is_some_and(|t| active.as_ref().is_some_and(|s| s.token != t)) {
            return; // a stale token: the live session stays
        }
        let Some(mut session) = active.take() else {
            return;
        };
        drop(active);
        if let Some(action) = session.policy.finish(now_ms()) {
            Self::record(&session, &action).await;
        }
    }

    /// The user's caption selection, persisted with the session's progress.
    pub async fn set_caption_mode(&self, token: &str, mode: Option<String>) {
        if let Some(session) = self.active.lock().await.as_mut() {
            if session.token == token {
                session.caption_mode = mode;
            }
        }
    }

    /// Apply the policy to one engine snapshot. Snapshots of any other token
    /// (a replaced or closed session) are ignored.
    pub async fn observe(&self, app: &AppHandle, state: &MpvPlayerState) {
        self.observe_with(state, |payload| {
            if let Err(e) = app.emit("playback:ended", payload) {
                tracing::debug!(error = %e, "failed to emit playback:ended");
            }
        })
        .await;
    }

    /// [`Self::observe`] with the `playback:ended` sink supplied (tests).
    pub async fn observe_with(&self, state: &MpvPlayerState, emit: impl FnOnce(PlaybackEnded)) {
        let mut emit = Some(emit);
        let mut active = self.active.lock().await;
        let Some(session) = active.as_mut() else {
            return;
        };
        if state.token.as_deref() != Some(session.token.as_str()) {
            return;
        }
        let actions = session.policy.observe(
            state.phase.clone(),
            state.position_sec,
            state.duration_sec,
            now_ms(),
        );
        for action in actions {
            match action {
                PolicyAction::Seek(at) => {
                    if let Err(e) = self.engine.seek_absolute(&session.token, at) {
                        tracing::debug!(error = %e, "resume seek failed");
                    }
                }
                PolicyAction::Record { .. } => Self::record(session, &action).await,
                PolicyAction::Ended => {
                    let next_episode = if session.media_kind == "episode" {
                        next_episode(&session.pool, session.media_id).await
                    } else {
                        None
                    };
                    let payload = PlaybackEnded {
                        token: session.token.clone(),
                        next_episode,
                        autoplay: session.autoplay,
                    };
                    if let Some(emit) = emit.take() {
                        emit(payload);
                    }
                }
            }
        }
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

    #[test]
    fn nothing_is_decided_while_opening() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        assert!(p.observe(Opening, 0.0, None, 0).is_empty());
        assert!(p.observe(Buffering, 0.0, Some(2400.0), 100).is_empty());
        assert_eq!(p.finish(200), None, "never loaded: nothing to write");
    }

    #[test]
    fn resumes_once_then_writes_every_five_seconds_while_playing() {
        let mut p = SessionPolicy::new(false, Some((600.0, false)));
        let first = p.observe(Playing, 0.0, Some(2400.0), 1_000);
        assert_eq!(first, vec![PolicyAction::Seek(600.0)]);
        // Settling after the seek: a stale 0:00 is never written.
        assert!(p.observe(Playing, 0.0, Some(2400.0), 2_000).is_empty());
        assert_eq!(p.finish(2_500), None);
        let settled = p.observe(Playing, 601.0, Some(2400.0), 4_100);
        assert_eq!(records(&settled), vec![(601.0, false)]);
        assert!(p.observe(Playing, 603.0, Some(2400.0), 6_000).is_empty());
        assert_eq!(
            records(&p.observe(Playing, 606.0, Some(2400.0), 9_200)),
            vec![(606.0, false)]
        );
        assert!(
            !p.observe(Playing, 607.0, Some(2400.0), 9_300)
                .iter()
                .any(|a| matches!(a, PolicyAction::Seek(_))),
            "resumes once"
        );
    }

    #[test]
    fn a_finished_or_near_start_position_is_not_resumed() {
        for resume in [Some((2390.0, true)), Some((3.0, false)), None] {
            let mut p = SessionPolicy::new(false, resume);
            let actions = p.observe(Playing, 0.0, Some(2400.0), 0);
            assert!(!actions.iter().any(|a| matches!(a, PolicyAction::Seek(_))));
        }
    }

    #[test]
    fn pausing_writes_immediately_and_finish_writes_the_last_position() {
        let mut p = SessionPolicy::new(false, None);
        p.observe(Playing, 10.0, Some(2400.0), 0);
        assert_eq!(
            records(&p.observe(Paused, 12.0, Some(2400.0), 1_000)),
            vec![(12.0, false)]
        );
        assert!(p.observe(Paused, 12.0, Some(2400.0), 9_000).is_empty());
        p.observe(Playing, 30.0, Some(2400.0), 9_500);
        assert_eq!(
            p.finish(10_000),
            Some(PolicyAction::Record {
                position_sec: 30.0,
                duration_sec: Some(2400.0),
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
        assert_eq!(p.finish(400), None, "already written at the end");
    }

    #[test]
    fn live_is_never_resumed_or_completed() {
        let mut p = SessionPolicy::new(true, Some((600.0, false)));
        let first = p.observe(Playing, 0.0, None, 0);
        assert_eq!(records(&first), vec![(0.0, false)]);
        assert!(!first.iter().any(|a| matches!(a, PolicyAction::Seek(_))));
        let end = p.observe(Ended, 50.0, None, 100);
        assert_eq!(records(&end), vec![(50.0, false)]);
    }

    #[test]
    fn errors_decide_nothing() {
        let mut p = SessionPolicy::new(false, None);
        p.observe(Playing, 10.0, Some(100.0), 0);
        assert!(p.observe(Error, 0.0, None, 10_000).is_empty());
        assert_eq!(
            records(&p.finish(10_000).into_iter().collect::<Vec<_>>()),
            vec![(10.0, false)],
            "the last good position is kept"
        );
    }
}
