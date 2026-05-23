import type { NowAndNext } from "@/types";

interface NowAndNextChipProps {
  data: NowAndNext | undefined;
}

/// Inline strip rendered under each Live channel name showing the current
/// programme and a thin progress bar of its elapsed share. Falls back to
/// nothing when no EPG data is available for this channel.
export function NowAndNextChip({ data }: NowAndNextChipProps) {
  if (!data?.now) return null;
  const { now, next } = data;
  const nowEpoch = Date.now() / 1000;
  const pct =
    now.stopUtc > now.startUtc
      ? Math.max(0, Math.min(100, ((nowEpoch - now.startUtc) / (now.stopUtc - now.startUtc)) * 100))
      : 0;

  return (
    <div className="mt-0.5 flex max-w-full flex-col gap-0.5">
      <p className="truncate text-[11px] text-white/50">
        <span className="text-white/75">Now:</span>{" "}
        <span className="text-white/90">{now.title}</span>
        {next && (
          <>
            <span className="mx-1.5 text-white/20">·</span>
            <span className="text-white/55">
              {formatTime(next.startUtc)} {next.title}
            </span>
          </>
        )}
      </p>
      <div className="h-0.5 w-full overflow-hidden rounded-full bg-white/[0.06]">
        <div className="h-full rounded-full bg-indigo-400/70" style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function formatTime(epoch: number): string {
  const d = new Date(epoch * 1000);
  return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}
