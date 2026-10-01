import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Skeleton } from "@/components/ui/skeleton";
import type { ModelDownloadEvent } from "@/types/generated/ModelDownloadEvent";
import { SettingsButton, SettingsCard, SettingsError, SettingRow } from "../SettingsUI";
import { useModelAction, useModelStatus } from "../modelData";
import { formatBytes } from "../format";
import { ExternalModelLibraries } from "./ExternalModelLibraries";

export function ModelsSection() {
  const { whisper, embedding, runtime } = useModelStatus();
  const action = useModelAction();
  const [progress, setProgress] = useState<ModelDownloadEvent | null>(null);
  useEffect(() => {
    setProgress(null);
    if (action.pending !== "download_whisper_model") return;
    let cancelled = false;
    const unlisten = listen<ModelDownloadEvent>("model:download:progress", ({ payload }) => {
      if (!cancelled) setProgress(payload);
    }).catch(() => null);
    return () => {
      cancelled = true;
      void unlisten.then((off) => off?.());
    };
  }, [action.pending]);
  const percent =
    progress?.totalBytes && progress.totalBytes > 0
      ? Math.min(
          100,
          Math.max(0, Math.round((progress.downloadedBytes / progress.totalBytes) * 100))
        )
      : null;
  return (
    <div className="flex flex-col gap-6">
      <p className="text-sm leading-relaxed text-muted-foreground">
        Models are shared by all profiles. Download only what you need; browsing and playback work
        without them.
      </p>
      {action.failed && (
        <SettingsError message="The model action could not finish. It may be in use, or the download may have failed. Try the action again when ready." />
      )}
      <SettingsCard title="Source-language captions">
        {whisper.isError ? (
          <div className="py-4">
            <SettingsError
              message="Couldn’t check the caption model."
              retry={() => void whisper.refetch()}
            />
          </div>
        ) : whisper.isPending ? (
          <Skeleton className="my-4 h-20" />
        ) : (
          <SettingRow
            title="Whisper"
            description={`${whisper.data.modelId} · ${whisper.data.installed ? "Ready for captions" : whisper.data.availability === "missing" ? "Not downloaded" : "Not ready — download to repair"}`}
          >
            {whisper.data.installed ? (
              <SettingsButton
                variant="danger"
                disabled={!!action.pending}
                onClick={() => {
                  if (
                    window.confirm(
                      "Remove the Whisper model from this Mac? Captions will need it downloaded again. Cached transcripts stay saved."
                    )
                  )
                    action.run("remove_whisper_model");
                }}
              >
                {action.pending === "remove_whisper_model" ? "Removing…" : "Remove model"}
              </SettingsButton>
            ) : (
              <SettingsButton
                variant="primary"
                disabled={!!action.pending}
                onClick={() => action.run("download_whisper_model")}
              >
                {action.pending === "download_whisper_model"
                  ? "Downloading…"
                  : "Download captions (~500 MB)"}
              </SettingsButton>
            )}
          </SettingRow>
        )}
        {action.pending === "download_whisper_model" && (
          <div className="space-y-2 py-4" role="status">
            <p className="text-xs text-muted-foreground">
              {progress
                ? `${progress.artifact} · ${formatBytes(progress.downloadedBytes)}${progress.totalBytes ? ` of ${formatBytes(progress.totalBytes)}` : " downloaded"}`
                : "Preparing download…"}
            </p>
            <div
              role="progressbar"
              aria-label="Caption model download"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent ?? undefined}
              className="h-1 overflow-hidden rounded bg-white/10"
            >
              <div className="h-full bg-primary" style={{ width: `${percent ?? 0}%` }} />
            </div>
            <p className="text-xs text-muted-foreground">
              You can leave Settings while the download continues.
            </p>
          </div>
        )}
      </SettingsCard>
      <SettingsCard title="Catalog intelligence">
        {embedding.isError ? (
          <div className="py-4">
            <SettingsError
              message="Couldn’t check the catalog model."
              retry={() => void embedding.refetch()}
            />
          </div>
        ) : embedding.isPending ? (
          <Skeleton className="my-4 h-20" />
        ) : (
          <SettingRow
            title="Embedding model"
            description={`${embedding.data.modelId} · ${embedding.data.installed ? "Ready for catalog enrichment and search" : "Not downloaded or needs repair"}`}
          >
            {embedding.data.installed ? (
              <span className="rounded-full bg-primary/10 px-3 py-1 text-xs text-primary">
                Ready
              </span>
            ) : (
              <SettingsButton
                variant="primary"
                disabled={!!action.pending}
                onClick={() => action.run("download_enrich_models")}
              >
                {action.pending === "download_enrich_models"
                  ? "Downloading…"
                  : "Download catalog (~120 MB)"}
              </SettingsButton>
            )}
          </SettingRow>
        )}
        {action.pending === "download_enrich_models" && (
          <p role="status" className="pb-4 text-xs text-muted-foreground">
            Downloading catalog model files. You can leave Settings while this finishes.
          </p>
        )}
      </SettingsCard>
      <SettingsCard title="Activity on this Mac">
        {runtime.isError ? (
          <div className="py-4">
            <SettingsError
              message="Model activity is unavailable."
              retry={() => void runtime.refetch()}
            />
          </div>
        ) : runtime.isPending ? (
          <Skeleton className="my-4 h-16" />
        ) : (
          <>
            <SettingRow
              title={
                runtime.data.activeJobs || runtime.data.busyModels ? "Working" : "Ready when needed"
              }
              description={`${runtime.data.activeJobs} active jobs · ${runtime.data.queuedJobs} queued jobs · ${runtime.data.loadedModels} loaded models`}
            />
            <SettingRow
              title="Estimated model memory"
              description={`${formatBytes(runtime.data.estimatedResidentBytes)}${runtime.data.unknownResidentModels ? " · some models have no memory estimate" : ""}`}
            />
            {runtime.data.suspendedWork > 0 && (
              <p className="pb-4 text-xs text-muted-foreground">
                Some background work is waiting while you watch.
              </p>
            )}
          </>
        )}
      </SettingsCard>
      <ExternalModelLibraries />
    </div>
  );
}
