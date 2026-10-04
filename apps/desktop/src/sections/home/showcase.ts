import type { ContinueWatchingItem } from "@/types";
import type { BrowseItem } from "@/features/browse/queries";

export type ShowcaseSlide =
  | { key: string; kind: "resume"; title: string; item: ContinueWatchingItem }
  | { key: string; kind: "title"; title: string; item: BrowseItem };

/** A bounded, alternating selection from the library, with resume first. */
export function showcaseSlides(
  resume: ContinueWatchingItem | null,
  movies: BrowseItem[],
  series: BrowseItem[]
): ShowcaseSlide[] {
  const slides: ShowcaseSlide[] = resume
    ? [
        {
          key: `resume-${resume.mediaKind}-${resume.mediaId}`,
          kind: "resume",
          title: resume.title,
          item: resume,
        },
      ]
    : [];
  const seen = new Set<string>();
  for (let i = 0; i < Math.max(movies.length, series.length) && slides.length < 5; i++) {
    for (const item of [movies[i], series[i]]) {
      if (!item || slides.length >= 5 || seen.has(item.key)) continue;
      seen.add(item.key);
      if (resume?.mediaKind === "vod" && resume.entityId && item.entityId === resume.entityId)
        continue;
      if (
        resume?.mediaKind === "episode" &&
        item.series &&
        ((resume.entityId && item.series.entityId === resume.entityId) ||
          (resume.sourceId === item.series.sourceId &&
            resume.seriesTitle === item.series.seriesTitle))
      )
        continue;
      slides.push({ key: item.key, kind: "title", title: item.title, item });
    }
  }
  return slides;
}
