/* Universal Clipboard motion kit: replay, auto-loop, reduced motion, pause, seek.
   Load at the end of <body>:  <script src="kit.js"></script>  then your page script.

   Each option is <figure class="opt" id="a1" data-loop="5000">. data-loop is the whole cycle in ms,
   animation plus rest (default 5000). Two ways to animate, mix them freely:

   1. CSS. Write rules under `.opt.is-playing ...`. Kit removes the class, restores the stage, and
      adds it again on every replay, so CSS keyframes restart.
   2. JS (WAAPI). Kit.register('#a1', (ctx) => { ... }). Runs on every replay with a fresh DOM.
        ctx.opt, ctx.stage            the figure and its .stage
        ctx.$(sel), ctx.$$(sel)       query inside the stage
        ctx.reduced                   true when reduced motion is on (html.reduce)
        ctx.anim(target, keyframes, options)
                                      el.animate with fill 'both' by default. target: element or selector
                                      (a selector animates every match). Returns the Animation(s).
        ctx.after(ms, fn)             run fn at ms (DOM swaps, text changes). Cancelled on replay.
                                      Animations made inside fn with ctx.anim seek correctly.
        ctx.signal                    AbortSignal, aborted on the next replay.
      Return a function to clean up, if you need one.
   Kit restores the stage's original HTML before each replay, so DOM changes never leak into the
   next loop. Keep the resting HTML in its final, readable state: content must show without script.

   Page controls (built into .gal-tools): Replay all, Reduced motion, Pause all.
   URL switches for headless checks: ?reduce=1  ?t=1200 (freeze every option at 1200 ms)
   Console / Playwright: Kit.seek(ms), Kit.replay(el), Kit.replayAll(), Kit.pause(), Kit.resume(),
   Kit.setReduced(bool). */
(function () {
  'use strict';
  const root = document.documentElement;
  const params = new URLSearchParams(location.search);
  const fns = new Map();     // opt -> fn
  const state = new Map();   // opt -> { html, timer, abort, cleanup, base: Map<Animation, ms> }
  let paused = false;
  let seekT = null;          // number while frozen by seek
  const visible = new Set();

  const mq = window.matchMedia ? matchMedia('(prefers-reduced-motion: reduce)') : null;
  let reduced = params.get('reduce') === '1' || (params.get('reduce') !== '0' && !!(mq && mq.matches));
  root.classList.toggle('reduce', reduced);

  const opts = () => Array.from(document.querySelectorAll('.opt'));
  const resolve = (x) => (typeof x === 'string' ? document.querySelector(x) : x);
  const stOf = (opt) => {
    let s = state.get(opt);
    if (!s) {
      const stage = opt.querySelector('.stage');
      s = { html: stage ? stage.innerHTML : '', timer: 0, abort: null, cleanup: null, base: new Map() };
      state.set(opt, s);
    }
    return s;
  };
  const loopMs = (opt) => Math.max(1500, Number(opt.dataset.loop) || 5000);

  function stop(opt) {
    const s = stOf(opt);
    clearTimeout(s.timer); s.timer = 0;
    if (s.abort) s.abort.abort();
    if (typeof s.cleanup === 'function') { try { s.cleanup(); } catch (e) { console.error(e); } }
    s.cleanup = null;
    opt.getAnimations({ subtree: true }).forEach((a) => {
      if (!(window.CSSAnimation && a instanceof CSSAnimation) && !(window.CSSTransition && a instanceof CSSTransition)) a.cancel();
    });
    s.base = new Map();
  }

  function run(opt) {
    const s = stOf(opt);
    stop(opt);
    const stage = opt.querySelector('.stage');
    if (stage) stage.innerHTML = s.html;
    opt.classList.remove('is-playing');
    void opt.offsetWidth; // restart CSS keyframes
    opt.classList.add('is-playing');
    const fn = fns.get(opt);
    if (!fn) return;
    const abort = new AbortController();
    s.abort = abort;
    let base = 0;
    const $ = (sel) => stage.querySelector(sel);
    const $$ = (sel) => Array.from(stage.querySelectorAll(sel));
    const ctx = {
      opt, stage, $, $$, reduced, signal: abort.signal,
      anim(target, keyframes, options) {
        const o = Object.assign({ fill: 'both', easing: 'cubic-bezier(0.16, 1, 0.3, 1)' }, typeof options === 'number' ? { duration: options } : options);
        const els = typeof target === 'string' ? $$(target) : [target].flat().filter(Boolean);
        const list = els.map((el) => {
          const a = el.animate(keyframes, o);
          s.base.set(a, base);
          if (paused || seekT !== null) a.pause();
          return a;
        });
        return typeof target === 'string' || Array.isArray(target) ? list : list[0];
      },
      after(ms, cb) {
        if (seekT !== null) {
          if (ms <= seekT) { const prev = base; base = ms; try { cb(); } finally { base = prev; } }
          return;
        }
        const id = setTimeout(() => { if (!abort.signal.aborted) { const prev = base; base = ms; try { cb(); } finally { base = prev; } } }, ms);
        abort.signal.addEventListener('abort', () => clearTimeout(id));
      },
    };
    try { s.cleanup = fn(ctx) || null; } catch (e) { console.error(e); }
  }

  function schedule(opt) {
    const s = stOf(opt);
    clearTimeout(s.timer);
    if (paused || seekT !== null || !visible.has(opt) || document.hidden) return;
    s.timer = setTimeout(() => { replay(opt); }, loopMs(opt));
  }

  function replay(opt) {
    opt = resolve(opt);
    if (!opt) return;
    if (seekT !== null) { seekT = null; root.classList.remove('frozen'); }
    run(opt);
    if (paused) pauseAnims(opt);
    schedule(opt);
  }
  function replayAll() { opts().forEach(replay); }

  function pauseAnims(opt) { opt.getAnimations({ subtree: true }).forEach((a) => a.pause()); }
  function pause() {
    paused = true; root.classList.add('paused');
    opts().forEach((o) => { clearTimeout(stOf(o).timer); pauseAnims(o); });
    syncTools();
  }
  function resume() {
    paused = false; root.classList.remove('paused');
    if (seekT !== null) { replayAll(); syncTools(); return; }
    opts().forEach((o) => { o.getAnimations({ subtree: true }).forEach((a) => a.play()); schedule(o); });
    syncTools();
  }

  function setReduced(on) {
    reduced = !!on; root.classList.toggle('reduce', reduced);
    syncTools();
    if (seekT !== null) seek(seekT); else replayAll();
  }

  /* Freeze every option at t ms after its start, for screenshots. Kit.replayAll() unfreezes. */
  function seek(t) {
    seekT = Math.max(0, Number(t) || 0);
    root.classList.add('frozen');
    opts().forEach((opt) => {
      run(opt);
      const s = stOf(opt);
      opt.getAnimations({ subtree: true }).forEach((a) => {
        a.pause();
        const b = s.base.has(a) ? s.base.get(a) : 0;
        a.currentTime = Math.max(0, seekT - b);
      });
      clearTimeout(s.timer);
    });
  }

  function register(target, fn) {
    const opt = resolve(target);
    if (!opt) { console.warn('Kit.register: no option', target); return; }
    stOf(opt);
    fns.set(opt, fn);
  }

  /* ---- Page controls ---- */
  const ico = {
    replay: '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 4v5h5"/></svg>',
    pause: '<svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><rect x="6" y="4.5" width="4" height="15" rx="1"/><rect x="14" y="4.5" width="4" height="15" rx="1"/></svg>',
    play: '<svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M7 4.8v14.4a.8.8 0 0 0 1.2.7l11.3-7.2a.8.8 0 0 0 0-1.4L8.2 4.1A.8.8 0 0 0 7 4.8z"/></svg>',
  };
  let tools = null;
  function buildTools() {
    tools = document.querySelector('.gal-tools');
    if (!tools) return;
    tools.innerHTML =
      '<button type="button" data-kit="replay">' + ico.replay + 'Replay all</button>' +
      '<button type="button" data-kit="reduce" aria-pressed="false">Reduced motion</button>' +
      '<button type="button" data-kit="pause" aria-pressed="false"></button>';
    tools.addEventListener('click', (e) => {
      const b = e.target.closest('button[data-kit]');
      if (!b) return;
      if (b.dataset.kit === 'replay') { if (paused) resume(); replayAll(); }
      if (b.dataset.kit === 'reduce') setReduced(!reduced);
      if (b.dataset.kit === 'pause') (paused ? resume : pause)();
    });
    syncTools();
  }
  function syncTools() {
    if (!tools) return;
    tools.querySelector('[data-kit=reduce]').setAttribute('aria-pressed', String(reduced));
    const p = tools.querySelector('[data-kit=pause]');
    p.setAttribute('aria-pressed', String(paused));
    p.innerHTML = paused ? ico.play + 'Play all' : ico.pause + 'Pause all';
  }

  function init() {
    buildTools();
    opts().forEach((opt) => {
      stOf(opt);
      const b = opt.querySelector('.opt-replay');
      if (b) {
        if (!b.innerHTML.trim()) b.innerHTML = ico.replay + 'Replay';
        b.addEventListener('click', () => { if (paused) { paused = false; root.classList.remove('paused'); syncTools(); } replay(opt); });
      }
    });
    if (mq && mq.addEventListener && params.get('reduce') === null) mq.addEventListener('change', (e) => setReduced(e.matches));
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) opts().forEach((o) => clearTimeout(stOf(o).timer));
      else opts().forEach(schedule);
    });

    const start = () => {
      if (params.has('t')) { opts().forEach((o) => visible.add(o)); seek(Number(params.get('t'))); return; }
      if ('IntersectionObserver' in window) {
        const io = new IntersectionObserver((entries) => {
          entries.forEach((en) => {
            const o = en.target;
            if (en.isIntersecting) { if (!visible.has(o)) { visible.add(o); if (seekT === null) replay(o); } }
            else { visible.delete(o); clearTimeout(stOf(o).timer); }
          });
        }, { threshold: 0.25 });
        opts().forEach((o) => io.observe(o));
      } else { opts().forEach((o) => visible.add(o)); replayAll(); }
    };
    // Register calls run in the page script after kit.js, so start on the next frame, after fonts.
    const fonts = document.fonts ? document.fonts.ready : Promise.resolve();
    fonts.then(() => requestAnimationFrame(start));
  }

  window.Kit = { register, replay, replayAll, pause, resume, setReduced, seek, get reduced() { return reduced; } };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
})();
