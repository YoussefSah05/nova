//! Installed model files on this Mac, read from the real model registry before
//! any profile opens. Derived capability, so neutral and dashed — never amber,
//! which marks a profile's own things. Planned capabilities say so plainly.

import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { SparklesIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { modelKeys } from "@/features/settings/modelData";
import type { ModelStatus } from "@/types/generated/ModelStatus";
import type { EnrichModelStatus } from "@/types/generated/EnrichModelStatus";

type State = "ready" | "missing" | "unknown" | "planned" | "checking";

const STATE_LABEL: Record<State, string> = {
  // The model is installed; per-profile indexes may still be building.
  ready: "Model installed",
  missing: "Not installed",
  unknown: "Couldn’t check",
  planned: "Coming with intelligence",
  checking: "Checking…",
};

function stateOf(query: { data?: { installed: boolean }; isError: boolean }): State {
  if (query.data) return query.data.installed ? "ready" : "missing";
  // A failed read says nothing about the model; never report it as missing.
  return query.isError ? "unknown" : "checking";
}

export function IntelligenceStatus() {
  // Shares the Settings → Models cache keys; one read is enough on this screen.
  const dialogue = useQuery({
    queryKey: modelKeys.whisper,
    queryFn: () => invoke<ModelStatus>("get_model_status"),
  });
  const search = useQuery({
    queryKey: modelKeys.embedding,
    queryFn: () => invoke<EnrichModelStatus>("get_enrich_model_status"),
  });
  const capabilities: { label: string; state: State }[] = [
    { label: "Dialogue transcription", state: stateOf(dialogue) },
    { label: "Semantic search", state: stateOf(search) },
    { label: "Ask Nova", state: "planned" },
  ];

  return (
    <section aria-label="On-device intelligence" className="flex flex-col gap-3">
      <p className="flex items-center gap-2 text-xs text-white/55">
        <SparklesIcon className="h-3.5 w-3.5" strokeWidth={1.5} aria-hidden />
        On-device intelligence · runs locally on this Mac
      </p>
      <ul className="flex flex-wrap gap-2">
        {capabilities.map(({ label, state }) => (
          <li
            key={label}
            data-state={state}
            className="flex h-8 items-center gap-2 rounded-full border border-dashed border-white/20 px-3 text-xs"
          >
            <span
              aria-hidden
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                state === "ready" ? "bg-white/80" : "border border-white/40"
              )}
            />
            <span className="text-white/80">{label}</span>
            <span className="text-white/45">{STATE_LABEL[state]}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
