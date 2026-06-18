//! Animated number that counts up to its value. Used for the sidebar content
//! counts + the import metrics so figures feel "alive". Reduced-motion → snaps.

import { useEffect, useRef, useState } from "react";
import { animate, useReducedMotion } from "motion/react";

export function CountUp({
  value,
  format = (n) => n.toLocaleString(),
  className,
}: {
  value: number;
  format?: (n: number) => string;
  className?: string;
}) {
  const reduce = useReducedMotion();
  const [display, setDisplay] = useState(value);
  const prev = useRef(value);

  useEffect(() => {
    if (reduce) {
      setDisplay(value);
      prev.current = value;
      return;
    }
    const from = prev.current;
    prev.current = value;
    const controls = animate(from, value, {
      duration: 0.7,
      ease: [0.16, 1, 0.3, 1],
      onUpdate: (v) => setDisplay(v),
    });
    return () => controls.stop();
  }, [value, reduce]);

  return <span className={className}>{format(Math.round(display))}</span>;
}
