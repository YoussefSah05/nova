//! A titled row of cards from a ready list of items (discovery rails: trending
//! in your library, more like this, a collection). Collapses when empty.

import { PosterCard, type PosterCardHandlers } from "./PosterCard";
import type { BrowseItem } from "./queries";

export function ItemRail({
  title,
  items,
  ...handlers
}: { title: string; items: BrowseItem[] } & PosterCardHandlers) {
  if (items.length === 0) return null;
  return (
    <section className="rail-contain flex flex-col gap-2">
      <h3 className="font-display text-[15px] font-semibold tracking-tight text-foreground/90">
        {title}
      </h3>
      <div className="-mx-1 flex snap-x gap-3 overflow-x-auto px-1 pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {items.map((item) => (
          <PosterCard key={item.key} item={item} {...handlers} />
        ))}
      </div>
    </section>
  );
}
