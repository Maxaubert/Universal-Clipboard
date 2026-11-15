/* Universal Clipboard iPhone kit script (2026-11-15).
   Load order at the end of <body>:
     <script src="../desktop/motion/art.js"></script>
     <script src="ios.js"></script>
     <script src="../desktop/motion/kit.js"></script>
     <script> page script: Kit.register(...) </script>
   It runs before kit.js snapshots the stages, so the chrome it adds is part of every replay.

   What it does:
   - Adds the status bar (9:41), the Dynamic Island and the home indicator to every .screen.
     <div class="screen" data-sbar="light"> makes the status bar white (over the camera).
     <div class="screen" data-chrome="none"> adds nothing.
   - Fills <i data-ic="gear"></i> with the icon of that name (IOS.ic below), and
     <i data-logo="apple"></i> with a device logo (from art.js).
   - Dark Mode: ?dark=1, or the Dark button it adds to the page tools.
   IOS.ic: SF Symbols look-alikes (24 x 24, 2 px strokes, round caps). */
(function () {
  'use strict';
  const S = (d, extra) => '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"' + (extra || '') + '>' + d + '</svg>';
  const F = (d) => '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">' + d + '</svg>';
  const ic = {
    gear: S('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>'),
    pause: F('<rect x="6" y="4.5" width="4" height="15" rx="1.2"/><rect x="14" y="4.5" width="4" height="15" rx="1.2"/>'),
    play: F('<path d="M7.5 5.2v13.6a1 1 0 0 0 1.5.86l11-6.8a1 1 0 0 0 0-1.72l-11-6.8a1 1 0 0 0-1.5.86z"/>'),
    plus: S('<path d="M12 5v14M5 12h14"/>', ' stroke-width="2.4"'),
    chev: '<svg class="chev" viewBox="0 0 8 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M1.5 1.5L6.5 7l-5 5.5"/></svg>',
    back: S('<path d="M15 5l-7 7 7 7"/>', ' stroke-width="2.4"'),
    x: S('<path d="M6 6l12 12M18 6L6 18"/>', ' stroke-width="2.4"'),
    check: S('<path d="M4.5 12.5l5 5L19.5 7"/>', ' stroke-width="2.6"'),
    up: S('<path d="M12 19V5M6 11l6-6 6 6"/>', ' stroke-width="2.4"'),
    down: S('<path d="M12 5v14M6 13l6 6 6-6"/>', ' stroke-width="2.4"'),
    scan: S('<path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2"/><path d="M8 8h3v3H8zM13 13h3v3h-3zM13 8h3M8 16h3"/>'),
    keypad: S('<rect x="2.5" y="5" width="19" height="14" rx="3"/><path d="M6.5 9.5h.01M10 9.5h.01M14 9.5h.01M17.5 9.5h.01M6.5 13h.01M10 13h.01M14 13h.01M17.5 13h.01M8.5 16h7"/>'),
    camera: S('<path d="M3 8.5A2.5 2.5 0 0 1 5.5 6h2l1.5-2h6l1.5 2h2A2.5 2.5 0 0 1 21 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/><circle cx="12" cy="13" r="3.5"/>'),
    wifi: S('<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.3a9.5 9.5 0 0 1 13 0M8.6 15.5a5 5 0 0 1 6.8 0"/><circle cx="12" cy="19" r="1" fill="currentColor"/>'),
    network: S('<circle cx="12" cy="5" r="2.5"/><circle cx="5" cy="18" r="2.5"/><circle cx="19" cy="18" r="2.5"/><path d="M11 7.3L6.2 15.8M13 7.3l4.8 8.5M7.5 18h9"/>'),
    share: S('<path d="M12 3v12M7.5 7.5L12 3l4.5 4.5"/><path d="M8 10.5H6.5A1.5 1.5 0 0 0 5 12v7.5A1.5 1.5 0 0 0 6.5 21h11a1.5 1.5 0 0 0 1.5-1.5V12a1.5 1.5 0 0 0-1.5-1.5H16"/>'),
    paste: S('<rect x="5.5" y="4.5" width="13" height="16" rx="2"/><path d="M9 4.5V3.8a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v.7"/><path d="M9 10h6M9 13.5h6M9 17h3.5"/>'),
    info: S('<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.6v.01"/>'),
    warn: S('<path d="M10.3 4.2L2.7 17.5A2 2 0 0 0 4.4 20.5h15.2a2 2 0 0 0 1.7-3L13.7 4.2a2 2 0 0 0-3.4 0z"/><path d="M12 9.5v4.5M12 17.2v.01"/>'),
    shield: S('<path d="M12 2.8l8 3.2v6c0 4.6-3.3 8.3-8 9.6-4.7-1.3-8-5-8-9.6V6z"/>'),
    lock: S('<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>'),
    trash: S('<path d="M4 7h16M10 11v6M14 11v6M5.5 7l1 12.5A1.5 1.5 0 0 0 8 21h8a1.5 1.5 0 0 0 1.5-1.5L18.5 7M9 7V4.5A1.5 1.5 0 0 1 10.5 3h3A1.5 1.5 0 0 1 15 4.5V7"/>'),
    pencil: S('<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>'),
    photo: S('<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><circle cx="8.5" cy="9.5" r="1.8"/><path d="M21 15.5l-5-5-9 9"/>'),
    text: S('<path d="M5 6h14M5 10.5h14M5 15h9"/>'),
    ellipsis: F('<circle cx="5.5" cy="12" r="1.9"/><circle cx="12" cy="12" r="1.9"/><circle cx="18.5" cy="12" r="1.9"/>'),
    arrowsync: S('<path d="M20 11a8 8 0 0 0-14.3-4.9L4 8M4 4v4h4M4 13a8 8 0 0 0 14.3 4.9L20 16M20 20v-4h-4"/>'),
    iphone: S('<rect x="6.5" y="2.5" width="11" height="19" rx="2.6"/><path d="M10.5 18.5h3"/>'),
    laptop: S('<rect x="4.5" y="5" width="15" height="10" rx="1.5"/><path d="M2.5 18.5h19"/>'),
    appstore: S('<circle cx="12" cy="12" r="9"/><path d="M9.5 7.5L14.5 16M14.5 7.5L9.5 16M7.5 13.5h9"/>'),
  };
  /* The app icon "Paper Layers" (PR #31, packaging/assets/icon/source/paper-layers.svg). It fills its box;
     the box gives the rounded corners. */
  ic.app = '<svg class="appicon" viewBox="0 0 1024 1024" fill="none" aria-hidden="true"><defs><linearGradient id="ucTile" x2=".74" y2="1"><stop stop-color="#34373f"/><stop offset="1" stop-color="#101116"/></linearGradient></defs><rect width="1024" height="1024" fill="url(#ucTile)"/><path d="M281 408L512 303L743 408L512 513Z M281 516L512 621L743 516M281 620L512 725L743 620" stroke="#fff" stroke-width="42" stroke-linecap="round" stroke-linejoin="round"/><rect x="437" y="278" width="150" height="66" rx="26" fill="#9b9da6"/></svg>';

  const sbar = (light) =>
    '<div class="sbar' + (light ? ' light' : '') + '" aria-hidden="true"><span>9:41</span><span class="sic">' +
    '<svg width="19" height="12" viewBox="0 0 19 12" fill="currentColor"><rect x="0" y="7.5" width="3.2" height="4.5" rx="1"/><rect x="5" y="5" width="3.2" height="7" rx="1"/><rect x="10" y="2.5" width="3.2" height="9.5" rx="1"/><rect x="15" y="0" width="3.2" height="12" rx="1"/></svg>' +
    '<svg width="17" height="12" viewBox="0 0 17 12" fill="currentColor"><path d="M8.5 2.3c2.3 0 4.4.9 6 2.4l1.2-1.2A10.2 10.2 0 0 0 8.5.6 10.2 10.2 0 0 0 1.3 3.5l1.2 1.2a8.5 8.5 0 0 1 6-2.4zm0 3.4c1.4 0 2.6.5 3.6 1.4l1.2-1.2a6.8 6.8 0 0 0-9.6 0l1.2 1.2c1-.9 2.2-1.4 3.6-1.4zm0 3.4c.5 0 1 .2 1.3.5L8.5 11 7.2 9.6c.3-.3.8-.5 1.3-.5z"/></svg>' +
    '<svg width="27" height="13" viewBox="0 0 27 13" fill="none"><rect x="0.5" y="0.5" width="23" height="12" rx="3.8" stroke="currentColor" opacity="0.4"/><rect x="2" y="2" width="20" height="9" rx="2.5" fill="currentColor"/><path d="M25 4.5v4c.8-.3 1.3-1.1 1.3-2s-.5-1.7-1.3-2z" fill="currentColor" opacity="0.45"/></svg>' +
    '</span></div>';

  function decorate(root) {
    root.querySelectorAll('.screen').forEach((s) => {
      if (s.dataset.chrome === 'none' || s.dataset.done) return;
      s.dataset.done = '1';
      const light = s.dataset.sbar === 'light';
      s.insertAdjacentHTML('beforeend', sbar(light) + '<div class="island"></div><div class="homebar' + (light ? ' light' : '') + '"></div>');
    });
    root.querySelectorAll('i[data-ic]').forEach((el) => { if (!el.innerHTML) el.innerHTML = ic[el.dataset.ic] || ''; el.style.display = 'contents'; });
    root.querySelectorAll('i[data-logo]').forEach((el) => {
      if (el.innerHTML) return;
      const k = el.dataset.logo;
      if (k === 'iphone' || k === 'ipad') { el.innerHTML = window.UC_ART ? window.UC_ART.logo('apple', 'currentColor', Number(el.dataset.size) || 20) : ''; }
      else el.innerHTML = window.UC_ART ? window.UC_ART.logo(k, 'currentColor', Number(el.dataset.size) || 20) : '';
      el.style.display = 'contents';
    });
  }

  const params = new URLSearchParams(location.search);
  const root = document.documentElement;
  let dark = params.get('dark') === '1';
  root.classList.toggle('dark', dark);
  decorate(document);

  function addDarkButton() {
    const tools = document.querySelector('.gal-tools');
    if (!tools || tools.querySelector('[data-ios=dark]')) return;
    const b = document.createElement('button');
    b.type = 'button'; b.dataset.ios = 'dark';
    const sync = () => { b.setAttribute('aria-pressed', String(dark)); b.textContent = 'Dark Mode'; };
    b.addEventListener('click', () => { dark = !dark; root.classList.toggle('dark', dark); sync(); });
    sync();
    tools.appendChild(b);
    const sz = document.createElement('button');
    sz.type = 'button'; sz.dataset.ios = 'size'; sz.setAttribute('aria-pressed', 'false'); sz.textContent = 'Actual size';
    sz.addEventListener('click', () => {
      const g = document.querySelector('.gal-grid'); const on = !g.classList.contains('actual');
      g.classList.toggle('actual', on); sz.setAttribute('aria-pressed', String(on));
    });
    tools.appendChild(sz);
  }
  /* Max 2026-11-15: make it plain that the options on a page are alternatives, not steps.
     A "Pick one" banner above the grid, an "Option X, n of N" tag on each, and "or" between them. */
  function markChoices() {
    const grid = document.querySelector('.gal-grid');
    if (!grid || grid.querySelector('.pick')) return;
    const opts = Array.from(grid.querySelectorAll('figure.opt'));
    if (opts.length < 2) return;
    const ids = opts.map((o) => { const b = o.querySelector('.opt-label b'); return b ? b.textContent.trim() : o.id.toUpperCase(); });
    const words = ['', 'one', 'two', 'three', 'four', 'five', 'six'];
    const list = ids.slice(0, -1).map((i) => '<b>' + i + '</b>').join(', ') + ' or <b>' + ids[ids.length - 1] + '</b>';
    const ban = document.createElement('div');
    ban.className = 'pick';
    ban.innerHTML = '<span class="pick-t">Pick one</span><span class="pick-s">These are ' + (words[opts.length] || opts.length) +
      ' different versions of the same screens. Each plays the whole story. Choose ' + list + '.</span>';
    grid.insertBefore(ban, grid.firstChild);
    opts.forEach((o, i) => {
      const tag = document.createElement('div');
      tag.className = 'opt-tag';
      tag.innerHTML = '<b>' + ids[i] + '</b><span>Option ' + (i + 1) + ' of ' + opts.length + '</span>';
      o.insertBefore(tag, o.firstChild);
      if (i > 0) o.classList.add('opt-or');
    });
  }

  // kit.js fills .gal-tools on DOMContentLoaded; add ours right after.
  const later = () => setTimeout(() => { addDarkButton(); markChoices(); }, 0);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', later); else later();

  window.IOS = { ic, decorate };
})();
