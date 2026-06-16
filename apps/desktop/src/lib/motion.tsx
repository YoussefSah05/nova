/* eslint-disable react-refresh/only-export-components --
   This is a shared motion toolkit: it intentionally exports variants/presets
   alongside the <Reveal>/<Stagger> wrappers. Fast-refresh of these helpers isn't
   relevant. */
//! Shared motion vocabulary (Phase: UI identity pass).
//!
//! A tiny, opinionated layer over `motion` so screens share one feel and respect
//! `prefers-reduced-motion` centrally. Use the variants with `motion.*` elements,
//! the `pressable` preset on interactive chrome, and the `<Reveal>` / `<Stagger>`
//! wrappers for entrance choreography. Keep motion to mount + interaction — never
//! per-scroll-frame (it would fight the browse virtualization / content-visibility).

import { type ReactNode } from "react";
import { motion, AnimatePresence, useReducedMotion, type Variants } from "motion/react";

export { motion, AnimatePresence, useReducedMotion };

/// Matches the CSS `--ease-enter` token.
export const EASE_ENTER = [0.16, 1, 0.3, 1] as const;
/// Springy preset for hover/press micro-interactions.
export const SPRING = { type: "spring", stiffness: 420, damping: 30 } as const;

export const fadeUp: Variants = {
  hidden: { opacity: 0, y: 12 },
  show: { opacity: 1, y: 0, transition: { duration: 0.28, ease: EASE_ENTER } },
};

export const scaleIn: Variants = {
  hidden: { opacity: 0, scale: 0.97 },
  show: { opacity: 1, scale: 1, transition: { duration: 0.2, ease: EASE_ENTER } },
};

export const staggerContainer: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.05, delayChildren: 0.03 } },
};

/// Per-child entrance for use inside a `staggerContainer`.
export const staggerItem: Variants = fadeUp;

/// Hover/press micro-interaction for buttons & cards. Spread onto a `motion.*`.
export const pressable = {
  whileHover: { scale: 1.03 },
  whileTap: { scale: 0.97 },
  transition: SPRING,
} as const;

/// Fade-and-rise a block on mount. Degrades to a plain wrapper under reduced motion.
export function Reveal({
  children,
  className,
  delay = 0,
}: {
  children: ReactNode;
  className?: string;
  delay?: number;
}) {
  const reduce = useReducedMotion();
  if (reduce) return <div className={className}>{children}</div>;
  return (
    <motion.div
      className={className}
      variants={fadeUp}
      initial="hidden"
      animate="show"
      transition={{ delay }}
    >
      {children}
    </motion.div>
  );
}

/// Container that staggers the entrance of its `<StaggerItem>` children.
export function Stagger({ children, className }: { children: ReactNode; className?: string }) {
  const reduce = useReducedMotion();
  if (reduce) return <div className={className}>{children}</div>;
  return (
    <motion.div className={className} variants={staggerContainer} initial="hidden" animate="show">
      {children}
    </motion.div>
  );
}

export function StaggerItem({ children, className }: { children: ReactNode; className?: string }) {
  const reduce = useReducedMotion();
  if (reduce) return <div className={className}>{children}</div>;
  return (
    <motion.div className={className} variants={staggerItem}>
      {children}
    </motion.div>
  );
}
