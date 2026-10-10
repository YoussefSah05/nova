//! The shared detail-page frame for movies and series: a full-bleed backdrop
//! with the artwork's ambient tint, the poster, the title logo, a meta line
//! (years · rating badge · runtime or seasons · score · genre), tagline,
//! overview, the actions, and whatever the page adds below (cast, episodes).

import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";

import { Artwork } from "@/components/Artwork";
import { Button } from "@/components/ui/button";
import { AmbientGlow, useAmbientColor } from "@/lib/ambient";
import { useArtworkSrc } from "@/lib/artwork";
import { motion, useReducedMotion, EASE_ENTER } from "@/lib/motion";
import { TitleLogo } from "../TitleLogo";
import type { MetaLine } from "./meta";

export function TitleHero({
  title,
  posterUrl,
  backdropUrl,
  logoUrl,
  meta,
  audioBanner,
  tagline,
  overview,
  note,
  onBack,
  actions,
  children,
}: {
  title: string;
  posterUrl: string | null | undefined;
  backdropUrl: string | null | undefined;
  logoUrl?: string | null;
  meta: MetaLine;
  audioBanner?: ReactNode;
  tagline?: string | null;
  overview?: string | null;
  /** A short highlighted line, e.g. the next episode's air date. */
  note?: string | null;
  onBack: () => void;
  actions: ReactNode;
  children?: ReactNode;
}) {
  const reduce = useReducedMotion();
  const [backdrop, onBackdropError] = useArtworkSrc(backdropUrl ?? posterUrl);
  const ambient = useAmbientColor(backdrop);

  return (
    <div className="relative min-h-screen bg-neutral-950 text-white">
      <div className="absolute inset-x-0 top-0 h-[60vh] overflow-hidden">
        {backdrop ? (
          <motion.img
            src={backdrop}
            alt=""
            onError={onBackdropError}
            decoding="async"
            className="h-full w-full object-cover object-top"
            initial={reduce ? false : { scale: 1.06, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            transition={{ duration: 0.6, ease: EASE_ENTER }}
          />
        ) : (
          <div className="h-full w-full bg-gradient-to-br from-primary/20 via-neutral-900 to-neutral-950" />
        )}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/60 to-transparent" />
        <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/85 to-transparent" />
      </div>
      <AmbientGlow color={ambient} className="h-[70vh] mix-blend-screen" />

      <motion.div
        className="relative z-10 flex flex-col gap-6 px-[88px] pb-12 pt-[96px]"
        initial={reduce ? false : { y: 16, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        transition={{ duration: 0.4, ease: EASE_ENTER, delay: 0.08 }}
      >
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          className="w-fit gap-2 text-white/70 hover:text-white"
        >
          <ArrowLeft className="h-4 w-4" /> Back
        </Button>

        <div className="mt-[30vh] flex items-end gap-5">
          <Artwork
            src={posterUrl}
            alt=""
            loading="lazy"
            decoding="async"
            className="hidden h-52 w-36 shrink-0 rounded-xl object-cover shadow-2xl sm:block"
          />
          <div className="min-w-0 max-w-2xl">
            <TitleLogo
              title={title}
              logoUrl={logoUrl}
              className="font-display text-3xl font-semibold tracking-tight drop-shadow-lg md:text-4xl"
            />
            <MetaRow meta={meta} />
            {audioBanner && <div className="mt-3">{audioBanner}</div>}
            {tagline && <p className="mt-2 text-sm italic text-white/60">{tagline}</p>}
            {note && <p className="mt-2 text-xs font-medium text-primary">{note}</p>}
            {overview && (
              <p className="mt-3 line-clamp-4 max-w-xl text-sm leading-6 text-white/75">
                {overview}
              </p>
            )}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">{actions}</div>
        {children}
      </motion.div>
    </div>
  );
}

function MetaRow({ meta }: { meta: MetaLine }) {
  if (!meta.certification && meta.parts.length === 0) return null;
  return (
    <div className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-white/70">
      {meta.certification && (
        <span className="rounded border border-white/35 px-1.5 text-xs font-medium leading-5 text-white/85">
          {meta.certification}
        </span>
      )}
      {meta.parts.map((part, i) => (
        <span key={part} className="flex items-center gap-2">
          {(i > 0 || meta.certification) && <span className="text-white/30">·</span>}
          {part}
        </span>
      ))}
    </div>
  );
}
