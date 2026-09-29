//! A title heading that shows the TMDB title logo when there is one. The
//! heading element stays (its accessible name is the logo's alt text), and a
//! missing or broken logo falls back to the plain title. Inside a button
//! (a card), `span` keeps the markup valid; the button carries the name.

import { Artwork } from "@/components/Artwork";
import { cn } from "@/lib/utils";

export function TitleLogo({
  title,
  logoUrl,
  as: Heading = "h1",
  className,
  logoClassName,
}: {
  title: string;
  logoUrl: string | null | undefined;
  as?: "h1" | "h2" | "span";
  className?: string;
  logoClassName?: string;
}) {
  return (
    <Heading className={cn(className, logoUrl && "leading-none")}>
      <Artwork
        src={logoUrl}
        alt={title}
        decoding="async"
        className={cn(
          "block h-auto max-h-20 w-auto max-w-[min(26rem,85%)] object-contain object-left drop-shadow-[0_2px_12px_rgba(0,0,0,0.6)] md:max-h-28",
          logoClassName
        )}
        fallback={title}
      />
    </Heading>
  );
}
