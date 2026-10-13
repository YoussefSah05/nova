import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Skeleton } from "@/components/ui/skeleton";
import type { CacheStats } from "@/types/generated/CacheStats";
import type { ArtworkCacheStats } from "@/types/generated/ArtworkCacheStats";
import { SettingsButton, SettingsCard, SettingsError, SettingRow } from "../SettingsUI";
import { formatBytes } from "../format";
import { useConfirm } from "../confirm";

export function StorageSection({ profileId }: { profileId: string }) {
  return (
    <div className="flex flex-col gap-6">
      <CacheSection key={profileId} profileId={profileId} />
      <CacheSection />
    </div>
  );
}
function CacheSection({ profileId }: { profileId?: string }) {
  const confirm = useConfirm();
  const client = useQueryClient();
  const transcript = profileId !== undefined;
  const queryKey = [
    "settings-storage",
    transcript ? "transcripts" : "artwork",
    profileId ?? "shared",
  ];
  const stats = useQuery({
    queryKey,
    staleTime: 0,
    queryFn: async () => {
      if (transcript) {
        const result = await invoke<CacheStats>("get_transcription_cache_stats", { profileId });
        // Backend bytes cover the shared transcript directory, not this profile.
        return { count: result.transcripts, bytes: null };
      }
      const result = await invoke<ArtworkCacheStats>("get_artwork_cache_stats");
      return { count: result.files, bytes: result.bytes };
    },
  });
  const clear = useMutation({
    mutationFn: () =>
      invoke(
        transcript ? "clear_transcription_cache" : "clear_artwork_cache",
        transcript ? { profileId } : {}
      ),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey });
      if (transcript)
        await client.invalidateQueries({
          predicate: (q) =>
            q.queryKey[0] === "browse" &&
            q.queryKey[1] === profileId &&
            q.queryKey[3] === "search" &&
            q.queryKey[4] === "moments",
        });
    },
  });
  const title = transcript ? "Cached transcripts" : "Cached artwork";
  return (
    <SettingsCard title={transcript ? "This profile" : "Shared by all profiles"}>
      {stats.isError ? (
        <div className="py-4">
          <SettingsError
            message={`Couldn’t check ${title.toLowerCase()}.`}
            retry={() => void stats.refetch()}
          />
        </div>
      ) : stats.isPending ? (
        <Skeleton className="my-4 h-20" />
      ) : (
        <SettingRow
          title={title}
          description={`${stats.data.count.toLocaleString()} ${transcript ? "transcripts" : "images"}${stats.data.bytes !== null ? ` · ${formatBytes(stats.data.bytes)}` : ""}. ${transcript ? "Generated again when captions are requested." : "Downloaded again when shown."}`}
        >
          <SettingsButton
            variant="danger"
            disabled={clear.isPending || stats.data.count === 0}
            onClick={() => {
              void confirm(
                transcript
                  ? {
                      title: "Clear cached transcripts?",
                      description:
                        "This profile’s saved captions are removed. They are generated again when needed.",
                      confirmLabel: "Clear transcripts",
                      destructive: true,
                    }
                  : {
                      title: "Clear cached artwork?",
                      description:
                        "Artwork is removed for all profiles on this Mac and downloaded again when shown.",
                      confirmLabel: "Clear artwork",
                      destructive: true,
                    }
              ).then((yes) => yes && clear.mutate());
            }}
          >
            {clear.isPending ? "Clearing…" : transcript ? "Clear transcripts" : "Clear artwork"}
          </SettingsButton>
        </SettingRow>
      )}
      {clear.isError && (
        <div className="pb-4">
          <SettingsError message={`Couldn’t clear ${title.toLowerCase()}. Try again.`} />
        </div>
      )}
      {clear.isSuccess && (
        <p role="status" className="pb-4 text-xs text-muted-foreground">
          Cache cleared.
        </p>
      )}
    </SettingsCard>
  );
}
