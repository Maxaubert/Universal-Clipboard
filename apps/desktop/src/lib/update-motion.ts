// Update motion (Max's pick 2026-11-14 from docs/design/desktop/motion/update.html):
// U1 "One shape" on the update screen, U3 "Home card" on Home. Timings, easings and sizes are the
// mockup's; its PRESS moment is 0 here. Reduced motion: fades only (the mockup's reduced
// variant), the ring still fills because it shows progress.
import { anim, draw, IN, IO, OUT, reduced, shake, track } from './motion';

const q = (root: Element, s: string) => root.querySelector<HTMLElement>(s);

/** One part arrives: fade, plus a rise or a grow unless reduced (the mockup's `life`). */
function arrive(el: Element | null, delay: number, d: number, from: { y?: number; scale?: number } = {}) {
  const r = reduced();
  const tf = r ? 'none' : from.scale ? `scale(${from.scale})` : from.y ? `translateY(${from.y}px)` : 'none';
  anim(el, [{ opacity: 0, transform: tf }, { opacity: 1, transform: 'none' }], { duration: d, delay, easing: OUT });
}

/** U1 offer: the window opened by itself. Art, text, then the buttons. */
export function offerIn(root: Element) {
  arrive(q(root, '.u-art'), 300, 700, { scale: 0.92 });
  arrive(q(root, '.u-txt'), 420, 700, { y: 12 });
  arrive(q(root, '.u-go'), 560, 600);
  arrive(q(root, '.u-later'), 560, 600);
}

/** U1 offer or failure leaves on Update now / Try again: the button presses and hands over to
 * the black shape at 120 ms; the rest fades. */
export function offerOut(root: Element, go: Element | null): number {
  const r = reduced();
  if (go && !r) track(go, [[0, { transform: 'scale(1)' }, IO], [110, { transform: 'scale(0.97)' }, OUT], [390, { transform: 'scale(1)' }]], 'none');
  if (go) {
    if (r) anim(go, [{ opacity: 1 }, { opacity: 0 }], { duration: 260, easing: 'ease', fill: 'forwards' });
    else track(go, [[0, { opacity: 1 }], [120, { opacity: 1 }], [120, { opacity: 0 }], [121, { opacity: 0 }]], 'forwards');
    anim(go.querySelector('.lbl'), [{ opacity: 1 }, { opacity: 0 }], { duration: 120, easing: IN, fill: 'forwards' });
  }
  anim(q(root, '.u-art'), [{ opacity: 1 }, { opacity: 0 }], { duration: 300, easing: IN, fill: 'forwards' });
  anim(q(root, '.u-txt'), [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: r ? 'none' : 'translateY(-8px)' }], {
    duration: 280,
    easing: IN,
    fill: 'forwards',
  });
  anim(q(root, '.u-later'), [{ opacity: 1 }, { opacity: 0 }], { duration: 250, easing: IN, fill: 'forwards' });
  return 450;
}

/** U1 updating arrives (the shape lands on the dial at `end`): track, arrow, text, ring. */
export function updatingIn(root: Element, end = 900) {
  const r = reduced();
  const core = q(root, '.core');
  if (r) anim(core, [{ opacity: 0 }, { opacity: 1 }], { duration: 360, delay: 120, easing: 'ease' });
  else anim(core, [{ opacity: 0 }, { opacity: 0 }], { duration: end, fill: 'none' });
  arrive(q(root, '.trk'), 700, 400);
  arrive(q(root, '.arw'), 760, 420, { y: -6 });
  arrive(q(root, '.u-txt'), 520, 600, { y: 10 });
  ring(root, 1000);
}

/** The ring fills while the helper works. There is no real progress to show: it fills most of
 * the way in 2.2 s, as in the mockup, and waits there (its resting look in Update.svelte)
 * until the app closes. */
export function ring(root: Element, start: number) {
  const prog = q(root, '.prog');
  track(prog, [[0, { strokeDashoffset: 1 }], [start, { strokeDashoffset: 1 }, IO], [start + 2200, { strokeDashoffset: 0.2 }]], 'none');
}

/** A layer leaves with a short fade (updating, before the end state). */
export function fadeOut(root: Element): number {
  anim(root, [{ opacity: 1 }, { opacity: 0 }], { duration: 300, easing: IN, fill: 'forwards' });
  return 300;
}

/** U1 end: after the restart (or in place), the dial grows in and the check draws, or the red
 * circle with the back arrow draws and gives one damped shake (E1). */
export function endIn(root: Element, t0 = 0) {
  arrive(q(root, '.u-art'), t0 + 250, 600, { scale: 0.72 });
  arrive(q(root, '.u-txt'), t0 + 400, 560, { y: 10 });
  arrive(q(root, '.u-acts'), t0 + 520, 560, { y: 10 });
  draw(q(root, '.tick'), t0 + 600, 460, OUT);
  draw(q(root, '.undo'), t0 + 600, 620, OUT);
  later(t0 + 1300, () => shake(q(root, '.shk')));
}

/** U3: the card opens from its top edge and pushes the tiles down. Reduced: it fades in. */
export function cardOpens(card: HTMLElement, tiles: HTMLElement | null, t: number) {
  if (reduced()) {
    anim(card, [{ opacity: 0 }, { opacity: 1 }], { duration: 700, delay: t, easing: 'ease' });
  } else {
    const full = 'inset(0px 0px 0% 0px round 12px)';
    const shut = 'inset(0px 0px 100% 0px round 12px)';
    anim(card, [{ opacity: 0, clipPath: shut }, { opacity: 1, clipPath: full }], { duration: 700, delay: t, easing: IO });
    const dy = card.offsetHeight + 16;
    anim(tiles, [{ transform: `translateY(-${dy}px)` }, { transform: 'translateY(0px)' }], { duration: 700, delay: t, easing: IO });
  }
  if (card.dataset.state === 'failed') {
    draw(card.querySelector('.undo'), t + 550, 620, OUT);
    later(t + 1050, () => shake(card));
  }
}

/** U3: Later closes the card; the tiles move up into its place. Reduced: it fades, then the
 * space closes at once. */
export function cardCloses(card: HTMLElement): number {
  const h = card.offsetHeight;
  if (reduced()) {
    anim(card, [{ opacity: 1 }, { opacity: 0 }], { duration: 250, easing: 'ease', fill: 'forwards' });
    return 250;
  }
  anim(card, [{ opacity: 1, height: `${h}px`, marginBottom: '0px' }, { opacity: 0, height: '0px', marginBottom: '-16px' }], {
    duration: 450,
    easing: IO,
    fill: 'forwards',
  });
  return 450;
}

function later(ms: number, fn: () => void) {
  setTimeout(fn, ms);
}
