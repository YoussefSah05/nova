//! How far through a show the profile is: `14 of 26 watched` over one tick
//! per episode (a bar past sixty), with TMDB's status beside it.

import type { SeriesSummary } from "@/types";
import { progressTicks, statusLabel, watchedLine } from "../shows";

export function ShowProgress({ series }: { series: SeriesSummary }) {
  const line = watchedLine(series);
  const status = statusLabel(series);
  if (!line && !status) return null;
  const ticks = progressTicks(series);
  const share = series.episodeCount > 0 ? (series.watchedCount ?? 0) / series.episodeCount : 0;
  return (
    <div className="flex max-w-xl flex-col gap-2">
      <p className="flex gap-3 text-sm text-white/70">
        {status && (
          <span className={status === "Returning" ? "text-primary" : undefined}>{status}</span>
        )}
        {line && <span>{line}</span>}
      </p>
      {line &&
        (ticks ? (
          <span aria-hidden className="flex gap-[3px]">
            {ticks.map((done, i) => (
              <span
                key={i}
                className={
                  done
                    ? "h-1 flex-1 rounded-sm bg-primary"
                    : "h-1 flex-1 rounded-sm bg-white/[0.14]"
                }
              />
            ))}
          </span>
        ) : (
          <span aria-hidden className="h-1 overflow-hidden rounded-sm bg-white/[0.14]">
            <span
              className="block h-full bg-primary"
              style={{ width: `${Math.min(100, share * 100)}%` }}
            />
          </span>
        ))}
    </div>
  );
}
