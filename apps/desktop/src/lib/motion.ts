// Motion helpers (picked 2026-11-14 from docs/design/desktop/motion: S1, E1, H3, A3, A4).
// Web Animations only, no library. Every helper does nothing where the browser cannot animate
// (jsdom in unit tests), and callers ask `reduced()` for the calm variant of the mockups.

export const OUT = 'cubic-bezier(0.16, 1, 0.3, 1)'; // arrivals
export const IO = 'cubic-bezier(0.65, 0, 0.35, 1)'; // moves between two places
export const IN = 'cubic-bezier(0.4, 0, 1, 1)'; // exits

/** The browser can run Web Animations (false in jsdom). */
export function canAnimate(): boolean {
  return typeof Element !== 'undefined' && typeof Element.prototype.animate === 'function';
}

/** The user asked the OS for less motion. */
export function reduced(): boolean {
  try {
    return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}

type Props = Record<string, string | number>;

/** el.animate with a guard. Default fill is 'backwards': the first frame holds during the delay,
 * and the element's own style shows after the end. */
export function anim(el: Element | null | undefined, frames: Keyframe[], o: KeyframeAnimationOptions): Animation | null {
  if (!el || !canAnimate()) return null;
  return el.animate(frames, { fill: 'backwards', ...o });
}

/** One track over absolute times, like the mockups' timelines: frames are [ms, props, easing of
 * the segment that starts there]. Fill 'both' while it runs; cancel() it when done if needed. */
export function track(el: Element | null | undefined, frames: [number, Props, string?][], fill: FillMode = 'both'): Animation | null {
  if (!el || !canAnimate() || frames.length === 0) return null;
  const f = frames.slice().sort((a, b) => a[0] - b[0]);
  if (f[0][0] > 0) f.unshift([0, f[0][1]]);
  const total = Math.max(1, f[f.length - 1][0]);
  const kf = f.map(([t, p, e]) => ({ ...p, offset: Math.min(1, t / total), easing: e ?? 'linear' }));
  return el.animate(kf, { duration: total, easing: 'linear', fill });
}

/** Entrance: rise from below (or fade only when reduced), like the mockups' `rise`. */
export function rise(el: Element | null | undefined, o: { y?: number; x?: number; scale?: number; d?: number; delay?: number; easing?: string } = {}): void {
  if (!el) return;
  if (reduced()) {
    anim(el, [{ opacity: 0 }, { opacity: 1 }], { duration: 320, delay: Math.min(o.delay ?? 0, 200), easing: 'ease-out' });
    return;
  }
  const tf = `translate(${o.x ?? 0}px, ${o.y ?? 8}px)${o.scale ? ` scale(${o.scale})` : ''}`;
  anim(el, [{ opacity: 0, transform: tf }, { opacity: 1, transform: 'none' }], { duration: o.d ?? 520, delay: o.delay ?? 0, easing: o.easing ?? OUT });
}

/** A short press on a button (S1). */
export function press(el: Element | null | undefined): void {
  if (!el || reduced()) return;
  track(el, [[0, { transform: 'scale(1)' }, IO], [110, { transform: 'scale(0.97)' }, OUT], [390, { transform: 'scale(1)' }]], 'none');
}

/** One damped shake, 7 px at most (E1). Nothing when reduced: the red outline says it. */
export function shake(el: Element | null | undefined): void {
  if (!el || reduced()) return;
  const x = (v: number) => ({ transform: `translateX(${v}px)` });
  track(el, [[0, x(0), IO], [100, x(-7), IO], [210, x(6), IO], [320, x(-4), IO], [430, x(2), IO], [540, x(0)]], 'none');
}

/** Draw an SVG stroke (pathLength 1, dasharray 1 2): hidden until `delay`, drawn over `d`. */
export function draw(el: Element | null | undefined, delay: number, d: number, easing = IO): void {
  anim(el, [{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }], { duration: d, delay, easing });
}

interface Roll {
  /** How far the text travels (default 110%, a whole line). */
  dist?: string;
  /** Duration of the arrival; the exit takes 55% of it. */
  d?: number;
  delay?: number;
  /** Pop in place (scale 0.6 to 1) instead of rolling (A3's icon). */
  pop?: boolean;
}
const away = (p: Roll, sign: number) => (p.pop ? 'scale(0.6)' : `translateY(${sign < 0 ? '-' : ''}${p.dist ?? '110%'})`);

/** Svelte transition: text that rolls up out of a clipped line (A3, A4, H3). Like the
 * mockups' `cycle`: the old text leaves first, the new one starts a little later. */
export function rollOut(node: Element, p: Roll = {}) {
  if (!canAnimate()) return { duration: 0 };
  const r = reduced();
  const d = Math.round((p.d ?? 520) * 0.55);
  const delay = p.delay ?? 0;
  node.setAttribute('aria-hidden', 'true');
  anim(node, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: r ? 'none' : away(p, -1) }], { duration: d, delay, easing: IN, fill: 'forwards' });
  return { duration: delay + d };
}
export function rollIn(node: Element, p: Roll = {}) {
  if (!canAnimate()) return { duration: 0 };
  const r = reduced();
  const d = p.d ?? 520;
  const delay = (p.delay ?? 0) + Math.round(d * (r ? 0.6 : 0.3));
  anim(node, [{ opacity: 0, transform: r ? 'none' : away(p, 1) }, { opacity: 1, transform: 'none' }], { duration: d, delay, easing: OUT });
  return { duration: delay + d };
}

/** Svelte transition: a message that rises in (E1). */
export function msgIn(node: Element, p: { delay?: number } = {}) {
  if (!canAnimate()) return { duration: 0 };
  const r = reduced();
  anim(node, [{ opacity: 0, transform: r ? 'none' : 'translateY(6px)' }, { opacity: 1, transform: 'none' }], { duration: 420, delay: p.delay ?? 0, easing: OUT });
  return { duration: (p.delay ?? 0) + 420 };
}

/** Wait for every running animation in `root` to finish (tests and the screen flow). */
export async function settled(root: Document | Element = document): Promise<void> {
  if (!canAnimate()) return;
  const list = 'getAnimations' in root ? (root as Document).getAnimations() : [];
  await Promise.all(list.map((a) => a.finished.catch(() => undefined)));
}
