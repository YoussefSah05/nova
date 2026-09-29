//! A section's title block: display-type name, a one-line summary, and
//! optional controls on the right.

import type { ReactNode } from "react";

export function SectionHeader({
  title,
  summary,
  children,
}: {
  title: string;
  summary?: string | null;
  children?: ReactNode;
}) {
  return (
    <header className="flex items-end gap-5">
      <div className="flex min-w-0 flex-col gap-1.5">
        <h1 className="font-display text-[36px] font-medium leading-tight tracking-[-0.015em]">
          {title}
        </h1>
        {summary && <p className="text-sm text-white/60">{summary}</p>}
      </div>
      <div className="flex-1" />
      {children}
    </header>
  );
}
