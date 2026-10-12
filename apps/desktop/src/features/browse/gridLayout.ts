//! Pure layout math for browse grids.

/// A landscape row's height: the 16:9 artwork of a card (gaps excluded), the
/// meta line under it (10 px gap + one 16 px line) and the row gap. Matching
/// the measured height keeps the virtualizer from correcting rows mid-scroll.
export function landscapeRowHeight(width: number, columns: number, gap: number): number {
  const card = (width - gap * (columns - 1)) / columns;
  return Math.round((card * 9) / 16 + 10 + 16 + gap);
}
