import { invoke } from "@tauri-apps/api/core";
import { useMutation, useMutationState, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ModelStatus } from "@/types/generated/ModelStatus";
import type { EnrichModelStatus } from "@/types/generated/EnrichModelStatus";
import type { LocalModelStatus } from "@/types/generated/LocalModelStatus";
import type { ModelRuntimeStatus } from "@/types/generated/ModelRuntimeStatus";

export const modelKeys = {
  all: ["settings-models"] as const,
  whisper: ["settings-models", "whisper"] as const,
  embedding: ["settings-models", "embedding"] as const,
  runtime: ["settings-models", "runtime"] as const,
  action: ["settings-model-action"] as const,
};
export type ModelAction =
  | "download_whisper_model"
  | "download_enrich_models"
  | "remove_whisper_model";
export function useModelStatus() {
  const whisper = useQuery({
    queryKey: modelKeys.whisper,
    queryFn: () => invoke<ModelStatus>("get_model_status"),
    staleTime: 0,
    refetchInterval: 15000,
  });
  const embedding = useQuery({
    queryKey: modelKeys.embedding,
    queryFn: () => invoke<EnrichModelStatus>("get_enrich_model_status"),
    staleTime: 0,
    refetchInterval: 15000,
  });
  const runtime = useQuery({
    queryKey: modelKeys.runtime,
    queryFn: () => invoke<ModelRuntimeStatus>("model_runtime_status"),
    staleTime: 0,
    refetchInterval: 5000,
  });
  return { whisper, embedding, runtime };
}
export function useModelAction() {
  const client = useQueryClient();
  const mutation = useMutation({
    mutationKey: modelKeys.action,
    mutationFn: (command: ModelAction) => invoke<void>(command),
    onSettled: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey: modelKeys.all }),
        client.invalidateQueries({ queryKey: ["capability", "semanticSearch"] }),
      ]);
    },
  });
  // Mutation cache outlives a Settings page. Reopening cannot start a duplicate
  // download and still shows its completion/failure if it finished while closed.
  const states = useMutationState({
    filters: { mutationKey: modelKeys.action },
    select: (m) => ({ status: m.state.status, command: m.state.variables as ModelAction }),
  });
  const pending = states.find((state) => state.status === "pending");
  const latest = states.at(-1);
  return {
    run: mutation.mutate,
    pending: pending?.command,
    failed: !pending && latest?.status === "error",
  };
}

export function useModelControls() {
  const client = useQueryClient();
  const models = useQuery({
    queryKey: [...modelKeys.all, "controls"],
    queryFn: () => invoke<LocalModelStatus[]>("list_model_controls"),
    refetchInterval: 5000,
  });
  const change = useMutation({
    mutationFn: async (action: { modelId: string; enabled?: boolean }) => {
      if (action.enabled === undefined)
        await invoke<boolean>("unload_model", { modelId: action.modelId });
      else await invoke<void>("set_model_enabled", action);
    },
    onSettled: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey: modelKeys.all }),
        client.invalidateQueries({ queryKey: ["capability", "semanticSearch"] }),
      ]);
    },
  });
  return { models, change };
}
