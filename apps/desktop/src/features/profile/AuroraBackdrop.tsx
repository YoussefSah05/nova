//! Animated aurora backdrop for the profile picker. A few large, blurred,
//! slow-drifting blobs in the brand accent over the app's warm body gradient —
//! the picker's "wow" layer, kept GPU-cheap (transform/opacity only; keyframes in
//! globals.css) and static under reduced-motion. Purely decorative.

export function AuroraBackdrop() {
  return (
    <div aria-hidden className="pointer-events-none fixed inset-0 -z-10 overflow-hidden">
      <div
        className="nova-aurora-blob nova-aurora-a"
        style={{
          top: "-12%",
          left: "-8%",
          width: "60vmax",
          height: "60vmax",
          background:
            "radial-gradient(circle, var(--color-brand) 0%, color-mix(in srgb, var(--color-brand) 30%, transparent) 45%, transparent 70%)",
        }}
      />
      <div
        className="nova-aurora-blob nova-aurora-b"
        style={{
          bottom: "-18%",
          right: "-10%",
          width: "55vmax",
          height: "55vmax",
          opacity: 0.35,
          background:
            "radial-gradient(circle, var(--color-brand-hi) 0%, color-mix(in srgb, var(--color-brand-hi) 22%, transparent) 50%, transparent 72%)",
        }}
      />
      {/* Vignette so the blobs read as light, not flat fill. */}
      <div className="absolute inset-0 bg-[radial-gradient(120%_120%_at_50%_50%,transparent_30%,rgba(0,0,0,0.55))]" />
    </div>
  );
}
