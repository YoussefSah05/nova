import { Skeleton } from "@/components/ui/skeleton";
import type { LocalModelStatus } from "@/types/generated/LocalModelStatus";
import { useModelControls } from "../modelData";
import { SettingsButton, SettingsCard, SettingsError, SettingRow } from "../SettingsUI";

const names: Record<string, string> = {
  "large-v3-turbo-q5_0": "Whisper · source captions",
  "multilingual-e5-small": "Catalog enrichment and semantic search",
  "opus-mt-en-fr": "English → French",
  "opus-mt-fr-en": "French → English",
  "opus-mt-en-ar": "English → Arabic",
  "opus-mt-ar-en": "Arabic → English",
};

function modelDescription(model: LocalModelStatus): string {
  const availability: Record<string, string> = {
    available: "Installed",
    unverified: "Unverified · may need repair",
    missing: "Missing files",
    unavailable: "Library unavailable",
    corrupt: "Files need repair",
  };
  const activity: Record<string, string> = {
    idle: "Loads on demand",
    loading: "Loading or changing model",
    prepared: "Prepared · speech weights load during transcription",
    loaded: "Loaded in memory",
    inUse: "In use",
  };
  return [
    availability[model.availability] ?? "Needs repair",
    model.library === "external" ? "External library" : "On this Mac",
    model.enabled ? "Enabled" : "Disabled",
    activity[model.activity] ?? "Activity unknown",
  ].join(" · ");
}

export function ModelControls() {
  const { models, change } = useModelControls();
  return (
    <>
      <SettingsCard title="Model status and controls">
        <p className="py-3 text-xs leading-relaxed text-muted-foreground">
          Enable a model to allow new work. It loads when needed. Disable releases an idle model;
          existing captions and translations stay saved. Turn captions off or wait for active jobs
          before disabling a model in use.
        </p>
        {change.isError && (
          <SettingsError message="Couldn’t change this model. It may be loading or in use. Turn captions off, wait for active work to finish, then try again." />
        )}
        {models.isError ? (
          <SettingsError
            message="Couldn’t check model controls."
            retry={() => void models.refetch()}
          />
        ) : models.isPending ? (
          <Skeleton className="my-4 h-24" />
        ) : (
          models.data.map((model) => {
            const name = names[model.modelId] ?? model.modelId;
            const busy = model.activity === "inUse" || model.activity === "loading";
            return (
              <SettingRow key={model.modelId} title={name} description={modelDescription(model)}>
                <div className="flex gap-2">
                  {(model.activity === "loaded" || model.activity === "prepared") && (
                    <SettingsButton
                      aria-label={`Free memory for ${name}`}
                      disabled={change.isPending}
                      onClick={() => change.mutate({ modelId: model.modelId })}
                    >
                      Free memory
                    </SettingsButton>
                  )}
                  <SettingsButton
                    aria-label={`${model.enabled ? "Disable" : "Enable"} ${name}`}
                    disabled={change.isPending || busy}
                    onClick={() =>
                      change.mutate({ modelId: model.modelId, enabled: !model.enabled })
                    }
                  >
                    {model.enabled ? "Disable" : "Enable"}
                  </SettingsButton>
                </div>
              </SettingRow>
            );
          })
        )}
      </SettingsCard>
      <SettingsCard title="Caption readiness and timing">
        <p className="py-3 text-sm leading-relaxed text-muted-foreground">
          Movies and episodes use Whisper for speech, then a translation model for each language
          direction. French ↔ Arabic requires both directions through English. Add a model library
          below if the translation files are on an external drive. Live AI captions are unavailable.
        </p>
        <p className="pb-3 text-xs leading-relaxed text-muted-foreground">
          A loaded model alone does not mean captions are ready. The player extracts and recognizes
          30-second audio windows before publishing captions. First captions depend on audio
          download, model startup and recognition; no fixed wait has been verified for this player.
          The CC menu shows the current stage, time in that stage and first-batch readiness time.
          Saved captions can appear immediately.
        </p>
        <p className="pb-3 text-xs leading-relaxed text-muted-foreground">
          Measured warm English → French and English → Arabic translation took about 0.3 seconds per
          sentence at the 95th percentile, after source text was ready. Batches, model loading and
          two language hops add time. Other directions still need measurement.
        </p>
        <p className="pb-3 text-xs leading-relaxed text-muted-foreground">
          Speech uses word timestamps, overlapping windows and boundary trimming to match the audio.
          Translations share the original caption’s time span. Recognition can miss names, accents
          or overlapping speakers; translations can lose nuance. Timing accuracy in real playback
          has not yet been measured.
        </p>
      </SettingsCard>
    </>
  );
}
