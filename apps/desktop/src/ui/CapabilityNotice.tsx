//! A plain note when an on-device capability is off, and what still works:
//! "Meaning-based search is off — exact search still works." Says where it
//! would run (on this Mac), so nothing reads as a cloud feature.

import { memo, type ReactNode } from "react";
import { CpuIcon } from "lucide-react";

export const CapabilityNotice = memo(function CapabilityNotice({
  title,
  detail,
  action,
}: {
  title: string;
  detail: string;
  action?: ReactNode;
}) {
  return (
    <div
      role="note"
      className="flex items-center gap-3 rounded-xl border border-white/[0.08] bg-white/[0.03] px-3.5 py-2.5"
    >
      <CpuIcon className="h-4 w-4 shrink-0 text-white/50" strokeWidth={1.5} aria-hidden />
      <p className="min-w-0 flex-1 text-sm text-white/75">
        <span className="font-medium text-white/90">{title}</span> — {detail}
        <span className="ml-2 rounded-md bg-white/[0.06] px-1.5 py-px text-xs text-white/55">
          On this Mac
        </span>
      </p>
      {action}
    </div>
  );
});
