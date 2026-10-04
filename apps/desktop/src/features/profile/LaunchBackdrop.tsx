//! Static launch backdrop for the profile picker: one warm light from the top
//! left and a floor vignette, matching the boot mark. No animation — the picker
//! is the first frame of the app and should settle, not drift.

export function LaunchBackdrop() {
  return (
    <div aria-hidden className="pointer-events-none fixed inset-0 -z-10">
      <div className="absolute inset-0 bg-[radial-gradient(70%_60%_at_18%_0%,color-mix(in_srgb,var(--color-brand)_14%,transparent),transparent_70%)]" />
      <div className="absolute inset-0 bg-[radial-gradient(90%_70%_at_50%_120%,rgba(0,0,0,0.6),transparent_70%)]" />
    </div>
  );
}
