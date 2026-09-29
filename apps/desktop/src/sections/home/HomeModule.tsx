//! One Home module: a display-type heading, an optional note and action, and
//! its content. Modules below the fold skip layout and paint until they come
//! near the viewport (`content-visibility: auto`), so a long Home scrolls as
//! cheaply as a short one.

import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function HomeModule({
  id,
  title,
  note,
  action,
  className,
  children,
}: {
  id: string;
  title: ReactNode;
  note?: ReactNode;
  action?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section
      aria-labelledby={id}
      className={cn("nova-cv flex flex-col gap-[18px] px-[88px]", className)}
    >
      <div className="flex items-baseline gap-3.5">
        <h2 id={id} className="font-display text-[28px] font-medium tracking-[-0.01em]">
          {title}
        </h2>
        {note && <span className="text-sm text-white/60">{note}</span>}
        <div className="flex-1" />
        {action}
      </div>
      {children}
    </section>
  );
}
