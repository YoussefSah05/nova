//! Window move-handle. An element tagged `data-tauri-drag-region` is treated by
//! the OS as the title bar: drag to move the window, double-click to maximize.
//! Used on screens without their own top bar (the profile picker); the browse
//! header opts in directly via the attribute.

import { cn } from "@/lib/utils";

/// Width (px) reserved at the top-left for the macOS traffic-light buttons when
/// the window uses the `Overlay` title-bar style.
export const TITLEBAR_INSET = 76;

/// A thin, transparent strip pinned to the top of the window that acts as the
/// drag handle. Children (if any) render above it and stay interactive.
export function DragRegion({ className }: { className?: string }) {
  return (
    <div
      data-tauri-drag-region
      className={cn("absolute inset-x-0 top-0 z-50 h-9", className)}
      aria-hidden
    />
  );
}
