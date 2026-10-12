//! Whether the user is scrolling anything, for work that should wait.
//!
//! One passive capture listener (installed on first use) records the last
//! scroll time. `whenScrollIdle` resolves once nothing has scrolled for
//! `IDLE_MS`, so optional main-thread work (logo sampling, warm-ups) never
//! competes with frames while content moves.

const IDLE_MS = 200;
let lastScroll = -Infinity;
let installed = false;

function install() {
  if (installed || typeof document === "undefined") return;
  installed = true;
  document.addEventListener(
    "scroll",
    () => {
      lastScroll = performance.now();
    },
    { capture: true, passive: true }
  );
}

export function isScrolling(now: number = performance.now()): boolean {
  install();
  return now - lastScroll < IDLE_MS;
}

export function whenScrollIdle(): Promise<void> {
  install();
  return new Promise((resolve) => {
    const check = () => {
      const wait = IDLE_MS - (performance.now() - lastScroll);
      if (wait <= 0) resolve();
      else setTimeout(check, wait);
    };
    check();
  });
}
