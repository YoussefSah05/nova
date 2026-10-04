# Player companion and Nova motion — frontend contract

The player direction was approved on 2026-10-02. This work precedes the remaining
backend milestones. It adds the companion frontend, shared motion and a rotating
Home showcase; it does not implement intelligence engines.

- Reuse PlayerModal's playback coordinator, native video rectangle, captions,
  transcript IPC, track controls, episode browser and live guide.
- Provide Now / Find / Explore / Saved in one companion alongside the video.
  Real transcript lines and playback actions work today. Summaries, semantic
  moments, generated chapters, questions and translation are explicitly pending.
  Never ship the proposal's fictional answers or media as production data.
- Saved moments are temporary player-session notes, clearly labelled. No schema,
  migration, new IPC command or model job is introduced.
- Home rotates a bounded set of real films and series from existing browse caches,
  preserving the current resume action. Pause on hover/focus, hidden documents,
  inactive/offscreen Home, overlays and reduced motion. Include manual controls.
- Motion uses the existing toolkit and opacity/transform only. Avoid animation of
  virtualized rows, continuous image zoom and per-frame React state. Native video
  bounds change once when the panel opens, never on each animation frame.
- Validate carousel lifecycle/accessibility, companion capability states and
  transcript isolation with focused tests, then frontend typecheck/lint/build.
  Native first-frame latency, actual ProMotion frame rate and provider playback
  require an on-device probe; do not infer them from frontend tests.

The intelligence backend, live ASR, translation, durable bookmarks and subsequent
architecture milestones remain separate work.
