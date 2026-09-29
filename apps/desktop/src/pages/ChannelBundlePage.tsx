//! A channel brand's bundle (beIN Sports 1…10, a network's regional feeds):
//! its channels on the same guide as the Live section, with the chosen one
//! beside it, instead of a list of tiles.

import type { BrowseItem } from "@/features/browse/queries";
import { LiveGuide } from "@/features/live/LiveGuide";

export function ChannelBundlePage({ item }: { item: BrowseItem & { groupKey: string } }) {
  return (
    <div className="flex h-screen flex-col gap-4 px-6 pb-6 pt-[84px]">
      <header className="flex h-14 shrink-0 items-baseline gap-4">
        <h1 className="ml-2 font-display text-[28px] font-medium tracking-[-0.01em]">
          {item.title}
        </h1>
        {item.memberCount && (
          <span className="text-sm text-white/60">{item.memberCount} channels</span>
        )}
      </header>
      <LiveGuide
        scope={{ kind: "bundle", group: item.groupKey }}
        emptyLabel="These channels are no longer in the playlist."
      />
    </div>
  );
}
