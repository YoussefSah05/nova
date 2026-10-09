import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ExternalLibraryStatus } from "@/types/generated/ExternalLibraryStatus";
import { Skeleton } from "@/components/ui/skeleton";
import { modelKeys } from "../modelData";
import { SettingRow, SettingsButton, SettingsCard, SettingsError } from "../SettingsUI";

const key = ["settings-external-model-libraries"];
export function ExternalModelLibraries() {
  const client = useQueryClient();
  const libraries = useQuery({
    queryKey: key,
    queryFn: () => invoke<ExternalLibraryStatus[]>("list_external_model_libraries"),
    staleTime: 0,
    refetchInterval: 15000,
  });
  const add = useMutation({
    mutationFn: () => invoke<string | null>("add_external_model_library"),
    onSuccess: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey: key }),
        client.invalidateQueries({ queryKey: modelKeys.all }),
        client.invalidateQueries({ queryKey: ["capability", "semanticSearch"] }),
      ]);
    },
  });
  return (
    <SettingsCard title="External model folders">
      <SettingRow
        title="Connect a model folder"
        description="Choose the parent folder containing models/whisper, models/embeddings or models/opus. Models stay in that folder."
      >
        <SettingsButton onClick={() => add.mutate()} disabled={add.isPending}>
          {add.isPending ? "Opening…" : "Choose folder"}
        </SettingsButton>
      </SettingRow>
      {libraries.isPending ? (
        <Skeleton className="my-4 h-16" />
      ) : libraries.isError ? (
        <div className="py-4">
          <SettingsError
            message="Couldn’t check external model folders."
            retry={() => void libraries.refetch()}
          />
        </div>
      ) : libraries.data.length === 0 ? (
        <p className="py-4 text-sm text-muted-foreground">
          No folders connected. Use Choose folder to add one.
        </p>
      ) : (
        libraries.data.map((library) => (
          <SettingRow
            key={library.libraryId}
            title={`${library.displayName} · ${library.libraryId.slice(-8)}`}
            description={`${library.availability === "available" ? "Connected" : "Disconnected — reconnect the drive to check it again"} · ${library.models.length ? library.models.map((model) => `${model.capability}: ${model.availability}`).join(" · ") : "No recognized models"}`}
          />
        ))
      )}
      {add.isError && (
        <div className="py-4">
          <SettingsError message="Couldn’t connect that folder. Try choosing it again." />
        </div>
      )}
      <SettingRow
        title="Folder status"
        description="Disconnected folders stay registered. Nova uses an available internal copy first, then a connected external copy."
      >
        <SettingsButton onClick={() => void libraries.refetch()} disabled={libraries.isFetching}>
          Refresh folders
        </SettingsButton>
      </SettingRow>
    </SettingsCard>
  );
}
