// Screen changes (motion picks 2026-11-14, docs/design/desktop/motion).
//
// S1 "One shape" (setup.html): in setup, the black button that was pressed never leaves. A black
// shape grows out of it into the next screen's black part (the Show a code button, the dark
// code box), and on pairing it shrinks to the node between the two devices and turns green
// (the Connected moment). Timings, easings and art come from the S1 timeline: the mockup's
// t1..t4 are 0 here, the moment the route changes.
// H3 "You are here" (home.html): Home builds in from the top, This PC's dark bar docks last.
// U1 "One shape" (update.html): Update now on Home's card grows into the update screen's dial;
// the update screen's own entrances and phase changes are in src/lib/update-motion.ts.
// Everything else (back, settings, device, help): a short cross-fade, S1's reduced variant.
// Reduced motion: every change is that cross-fade, with the Connected check fading in.
//
// App.svelte renders each screen in a `.layer` (absolute, fills the body) inside a {#key}, so
// the old screen stays for its exit while the new one enters. `screenOut` is the old layer's
// Svelte out-transition, `screenIn` the new layer's action. Screens mark their parts with
// data-m="text|art|cta|back|os|rows|hdr|tiles|files|here" and the S1 target with data-shape.
import { anim, canAnimate, draw, IN, IO, OUT, press, reduced, rise, track } from './motion';
import { store, type View } from './store.svelte';
import { endIn, offerIn, updatingIn } from './update-motion';

const SETUP = new Set<View>(['welcome', 'perms', 'add', 'show', 'enter', 'connected']);
const INK = '#1D1C1A';
const GREEN = '#1E7A46';

type Kind = 'none' | 'fade' | 'shape' | 'rise' | 'paired' | 'home' | 'boot';
interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
  r: number;
}
interface Plan {
  kind: Kind;
  to: View;
  src: Rect | null;
}

let plan: Plan | null = null;

function rectIn(el: Element, body: Element): Rect {
  const b = body.getBoundingClientRect();
  const r = el.getBoundingClientRect();
  const cr = getComputedStyle(el).borderTopLeftRadius;
  const rad = cr.endsWith('%') ? (r.width * parseFloat(cr)) / 100 : parseFloat(cr) || 0;
  return { x: r.left - b.left, y: r.top - b.top, w: r.width, h: r.height, r: rad };
}

/** clip-path that shows only `r` inside the body (S1's shape). */
function clip(body: HTMLElement, r: Rect, rad = r.r): string {
  const f = (n: number) => Math.round(n * 10) / 10;
  const BW = body.offsetWidth;
  const BH = body.offsetHeight;
  return `inset(${f(r.y)}px ${f(BW - r.x - r.w)}px ${f(BH - r.y - r.h)}px ${f(r.x)}px round ${f(rad)}px)`;
}

const isBlack = (el: Element | null): el is HTMLElement =>
  !!el && el instanceof HTMLElement && getComputedStyle(el).backgroundColor === 'rgb(29, 28, 26)';

function decide(from: View, to: View, oldNode: Element): Plan {
  const nav = store.nav;
  if (nav.kind === 'none' || !canAnimate()) return { kind: 'none', to, src: null };
  if (reduced()) return { kind: 'fade', to, src: null };
  if (nav.kind === 'forward' && to === 'connected' && (from === 'show' || from === 'enter')) {
    const src = nav.from && oldNode.contains(nav.from) ? nav.from : oldNode.querySelector('[data-shape]');
    return { kind: 'paired', to, src: src && oldNode.parentElement ? rectIn(src, oldNode.parentElement) : null };
  }
  if (nav.kind === 'forward' && to === 'home' && from === 'connected') return { kind: 'home', to, src: null };
  // U1: Update now on Home's card grows into the dial.
  if (nav.kind === 'forward' && to === 'update' && isBlack(nav.from) && oldNode.contains(nav.from) && oldNode.parentElement) {
    return { kind: 'shape', to, src: rectIn(nav.from, oldNode.parentElement) };
  }
  if (nav.kind === 'forward' && SETUP.has(from) && SETUP.has(to) && to !== 'welcome') {
    const f = nav.from && oldNode.contains(nav.from) ? nav.from : null;
    if (isBlack(f) && oldNode.parentElement) return { kind: 'shape', to, src: rectIn(f, oldNode.parentElement) };
    return { kind: 'rise', to, src: null };
  }
  return { kind: 'fade', to, src: null };
}

const groups = (node: Element) => Array.from(node.querySelectorAll<HTMLElement>('[data-m]'));

/** Svelte out-transition of a screen layer. */
export function screenOut(node: Element) {
  // The view this layer was made for (set once by screenIn; props already say the new one).
  const from = ((node as HTMLElement).dataset.layer ?? '') as View;
  const to = store.view;
  const pl = decide(from, to, node);
  plan = pl;
  if (pl.kind === 'none') return { duration: 0 };
  // The leaving layer is only a picture now: no focus, no screen reader, no test ids.
  (node as HTMLElement).inert = true;
  node.setAttribute('aria-hidden', 'true');
  for (const el of Array.from(node.querySelectorAll('[data-testid]'))) el.removeAttribute('data-testid');
  if (pl.kind === 'fade') {
    anim(node, [{ opacity: 1 }, { opacity: 0 }], { duration: 200, easing: 'ease', fill: 'forwards' });
    return { duration: 400 };
  }
  // S1: the pressed button (or the code box) is handed to the shape; the rest leaves.
  const src = pl.kind === 'shape' || pl.kind === 'paired' ? sourceIn(node) : null;
  if (pl.to === 'update' && src) {
    // U1 from Home's card: the button presses and hands over; Home fades as one.
    press(src);
    anim(src.querySelector('.lbl'), [{ opacity: 1 }, { opacity: 0 }], { duration: 120, easing: IN, fill: 'forwards' });
    anim(node, [{ opacity: 1 }, { opacity: 0 }], { duration: 300, delay: 120, easing: IN, fill: 'forwards' });
    return { duration: 900 };
  }
  const handOff = pl.kind === 'paired' ? 200 : 150;
  if (src) {
    if (src.tagName === 'BUTTON') press(src);
    anim(src.querySelector('.lbl'), [{ opacity: 1 }, { opacity: 0 }], { duration: 150, easing: IN, fill: 'forwards' });
    track(src, [[0, { opacity: 1 }], [handOff, { opacity: 1 }], [handOff, { opacity: 0 }], [handOff + 1, { opacity: 0 }]], 'forwards');
  }
  for (const g of groups(node)) {
    if (src && g.contains(src)) {
      for (const c of Array.from(g.children)) {
        if (!c.contains(src)) anim(c, [{ opacity: 1 }, { opacity: 0 }], { duration: 250, easing: IN, fill: 'forwards' });
      }
      continue;
    }
    const lift = g.dataset.m === 'text' || g.dataset.m === 'back';
    anim(g, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: lift ? 'translateY(-8px)' : 'none' }], {
      duration: 300,
      easing: IN,
      fill: 'forwards',
    });
  }
  if (groups(node).length === 0) anim(node, [{ opacity: 1 }, { opacity: 0 }], { duration: 300, easing: IN, fill: 'forwards' });
  return { duration: pl.kind === 'paired' ? 1000 : 900 };
}

function sourceIn(node: Element): HTMLElement | null {
  const f = store.nav.from;
  if (f && node.contains(f)) return f as HTMLElement;
  return node.querySelector<HTMLElement>('[data-shape]');
}

/** Action on a new screen layer: plays its entrance and moves focus to its heading. */
export function screenIn(node: HTMLElement, view: View) {
  node.dataset.layer = view;
  const nav = store.nav;
  const pl: Plan = plan && plan.to === view ? plan : { kind: nav.kind === 'boot' ? 'boot' : 'none', to: view, src: null };
  plan = null;
  store.nav = { kind: 'none', from: null };
  if (pl.kind !== 'none' && pl.kind !== 'boot') focusHeading(node);
  if (!canAnimate()) return;
  enter(node, view, pl);
}

function focusHeading(node: HTMLElement) {
  setTimeout(() => {
    if (!node.isConnected || node.contains(document.activeElement)) return;
    const h = node.querySelector<HTMLElement>('h1');
    if (!h) return;
    h.tabIndex = -1;
    h.focus({ preventScroll: true });
  }, 0);
}

function q(node: Element, m: string): HTMLElement | null {
  return node.querySelector<HTMLElement>(`[data-m="${m}"]`);
}

function enter(node: HTMLElement, view: View, pl: Plan) {
  const r = reduced();
  switch (pl.kind) {
    case 'none':
      return;
    case 'boot':
      if (view === 'welcome') return welcomeIn(node, r);
      if (view === 'home') return homeIn(node);
      if (view === 'update') return updateBoot(node);
      return;
    case 'fade':
      anim(node, [{ opacity: 0 }, { opacity: 1 }], { duration: 320, delay: 180, easing: 'ease' });
      if (view === 'connected') anim(node.querySelector('.l-check'), [{ opacity: 0 }, { opacity: 1 }], { duration: 400, delay: 400, easing: 'ease' });
      return;
    case 'home':
      return homeIn(node);
    case 'rise':
      groups(node).forEach((g, i) => rise(g, { y: 8, d: 520, delay: 300 + i * 60 }));
      return;
    case 'shape':
      return shapeIn(node, view, pl.src);
    case 'paired':
      return pairedIn(node, pl.src);
  }
}

// S1 Welcome, first open: text rises, the devices rise one by one, the clip settles, arcs draw.
function welcomeIn(node: HTMLElement, r: boolean) {
  const t = q(node, 'text');
  const parts = [t?.querySelector('h1'), t?.querySelector('p'), q(node, 'cta')];
  if (r) {
    [...parts, q(node, 'art')].forEach((el, i) => anim(el, [{ opacity: 0 }, { opacity: 1 }], { duration: 500, delay: 80 + i * 140, easing: 'ease' }));
    return;
  }
  parts.forEach((el, i) => rise(el, { y: 14, d: 760, delay: 80 + i * 120 }));
  ['.w-laptop', '.w-phone', '.w-droid'].forEach((s, i) => rise(node.querySelector(s), { y: 22, d: 900, delay: 380 + i * 130 }));
  anim(node.querySelector('.w-clip'), [{ opacity: 0, transform: 'translateY(8px) scale(0.9)' }, { opacity: 1, transform: 'none' }], {
    duration: 600,
    delay: 900,
    easing: OUT,
  });
  anim(node.querySelector('.w-arcs'), [{ opacity: 0 }, { opacity: 1 }], { duration: 700, delay: 1050, easing: OUT });
}

// H3: the header, then the tiles one by one, then This PC's bar docks from below. The file
// buttons above the bar come up with it, a little before.
function homeIn(node: HTMLElement) {
  rise(q(node, 'hdr'), { y: 6 });
  node.querySelectorAll('[data-m="tiles"] > *').forEach((t, i) => rise(t, { y: 10, delay: 120 + i * 60 }));
  rise(q(node, 'files'), { y: 34, d: 760, delay: 380 });
  rise(q(node, 'here'), { y: 34, d: 760, delay: 420 });
}

/** The black shape: from `src` (at `start`) to `dst` (at `end`), then it hands over. */
function shape(node: HTMLElement, src: Rect, dst: Rect, start: number, end: number): Animation | null {
  const body = node.parentElement;
  const blob = body?.querySelector<HTMLElement>(':scope > .blob');
  if (!body || !blob) return null;
  const a = clip(body, src);
  const b = clip(body, dst);
  track(blob, [[0, { clipPath: a, opacity: 0 }], [start, { clipPath: a, opacity: 0 }], [start, { clipPath: a, opacity: 1 }, IO], [end, { clipPath: b, opacity: 1 }], [end, { clipPath: b, opacity: 0 }]], 'none');
  return null;
}

// U1: the window opened by itself on the update screen (the agent's `--route update`).
function updateBoot(node: HTMLElement) {
  const ph = node.querySelector<HTMLElement>('[data-phase]')?.dataset.phase;
  if (ph === 'offer') offerIn(node);
  else if (ph === 'ok' || ph === 'fail') endIn(node);
  else if (ph === 'updating') updatingIn(node, 0);
}

/** The black shape inside one screen (U1: Update now grows into the dial): from `from` to `to`. */
export function shapeBetween(from: Element, to: Element, start: number, end: number): void {
  const body = to.closest<HTMLElement>('.body');
  const blob = body?.querySelector<HTMLElement>(':scope > .blob');
  if (!body || !blob || !canAnimate() || reduced()) return;
  const a = clip(body, rectIn(from, body));
  const b = clip(body, rectIn(to, body));
  track(blob, [[0, { clipPath: a, opacity: 0 }], [start, { clipPath: a, opacity: 0 }], [start, { clipPath: a, opacity: 1 }, IO], [end, { clipPath: b, opacity: 1 }], [end, { clipPath: b, opacity: 0 }]], 'none');
}

// Per target screen: when the shape lands, and when each part arrives [delay, duration, rise].
const SHAPE_IN: Record<string, { end: number; parts: Record<string, [number, number, number]> }> = {
  add: { end: 850, parts: { art: [350, 620, 12], text: [650, 620, 12], cta: [600, 500, 0] } },
  perms: { end: 850, parts: { back: [420, 520, 8], text: [480, 520, 8], rows: [540, 520, 8], cta: [600, 500, 0] } },
  show: { end: 950, parts: { back: [420, 520, 8], text: [480, 520, 8], os: [540, 520, 8], cta: [600, 520, 8] } },
  // U1: the parts arrive in updatingIn (src/lib/update-motion.ts).
  update: { end: 900, parts: {} },
};

function shapeIn(node: HTMLElement, view: View, src: Rect | null) {
  const spec = SHAPE_IN[view];
  const target = node.querySelector<HTMLElement>('[data-shape]');
  if (!spec || !target || !src) {
    groups(node).forEach((g, i) => rise(g, { y: 8, d: 520, delay: 300 + i * 60 }));
    return;
  }
  const end = spec.end;
  shape(node, src, rectIn(target, node.parentElement!), 150, end);
  if (view === 'update') return updatingIn(node, end);
  // The target hides until the shape lands on it, then its label fades in.
  anim(target, [{ opacity: 0 }, { opacity: 0 }], { duration: end, fill: 'none' });
  anim(target.querySelector('.lbl'), [{ opacity: 0 }, { opacity: 1 }], { duration: 300, delay: end, easing: OUT });
  for (const g of groups(node)) {
    const [delay, d, y] = spec.parts[g.dataset.m ?? ''] ?? [420, 520, 8];
    if (g.contains(target)) {
      for (const c of Array.from(g.children)) if (!c.contains(target)) anim(c, [{ opacity: 0 }, { opacity: 1 }], { duration: d, delay, easing: OUT });
      continue;
    }
    if (y) rise(g, { y, d, delay });
    else anim(g, [{ opacity: 0 }, { opacity: 1 }], { duration: d, delay, easing: OUT });
  }
  if (view === 'show') {
    anim(node.querySelector('.m-qr'), [{ opacity: 0, transform: 'scale(0.96)' }, { opacity: 1, transform: 'none' }], { duration: 520, delay: end, easing: OUT });
    anim(node.querySelector('.m-code'), [{ opacity: 0 }, { opacity: 1 }], { duration: 450, delay: end + 130, easing: OUT });
  }
}

// S1 Connected: the shape shrinks to the node between the devices, turns green and opens into
// the check; the green line draws across; the text and buttons rise.
function pairedIn(node: HTMLElement, src: Rect | null) {
  const T4 = 1000;
  const art = q(node, 'art');
  anim(art, [{ opacity: 0 }, { opacity: 1 }], { duration: 600, delay: 650, easing: OUT });
  // The dashed "connecting" line shows with the devices and gives way to the green one.
  track(node.querySelector('.l-dash'), [[0, { opacity: 1 }], [T4, { opacity: 1 }, IN], [T4 + 400, { opacity: 0 }]], 'none');
  draw(node.querySelector('.l-solid'), T4 + 120, 520);
  anim(node.querySelector('.l-check'), [{ opacity: 0 }, { opacity: 1 }], { duration: 300, delay: T4 + 260, easing: OUT });
  draw(node.querySelector('.l-tick'), T4 + 420, 420, OUT);
  rise(q(node, 'text'), { y: 10, d: 560, delay: T4 + 360 });
  rise(q(node, 'cta'), { y: 10, d: 560, delay: T4 + 460 });
  const circle = node.querySelector('.l-check circle');
  const body = node.parentElement;
  const blob = body?.querySelector<HTMLElement>(':scope > .blob');
  if (!src || !circle || !body || !blob) return;
  const dst = rectIn(circle, body);
  const a = clip(body, src);
  const b = clip(body, dst, dst.w / 2);
  track(blob, [[0, { clipPath: a }], [200, { clipPath: a }, IO], [T4, { clipPath: b }], [T4 + 640, { clipPath: b }]], 'none');
  track(blob, [[0, { opacity: 0 }], [200, { opacity: 0 }], [200, { opacity: 1 }], [T4 + 260, { opacity: 1 }, OUT], [T4 + 640, { opacity: 0 }]], 'none');
  track(blob, [[0, { backgroundColor: INK }], [T4, { backgroundColor: INK }, IO], [T4 + 320, { backgroundColor: GREEN }], [T4 + 640, { backgroundColor: GREEN }]], 'none');
}
