// Generates the static mockup pages. Output is plain HTML; viewing needs no build step.
import { writeFileSync } from "node:fs";
import { join } from "node:path";
// Optional: regenerates the HTML pages after an edit. Run: node docs/mockups/tools/gen.mjs

import { fileURLToPath } from "node:url";
const OUT = fileURLToPath(new URL("..", import.meta.url));

// ---------- icons (one stroke family, 24 grid) ----------
const P = {
  laptop: '<rect x="4" y="5" width="16" height="10.5" rx="1.5"/><path d="M2 18.5h20"/>',
  desktop: '<rect x="3" y="4" width="18" height="12" rx="1.5"/><path d="M12 16v4M8.5 20h7"/>',
  phone: '<rect x="7" y="2.5" width="10" height="19" rx="2.2"/><path d="M11 18.5h2"/>',
  tablet: '<rect x="4" y="2.5" width="16" height="19" rx="2.2"/><path d="M11 18.5h2"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  chev: '<path d="M9 6l6 6-6 6"/>',
  chevdown: '<path d="M6 9l6 6 6-6"/>',
  back: '<path d="M15 5l-7 7 7 7"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  shield: '<path d="M12 3l7 3v5.5c0 4.2-3 7.7-7 9.5-4-1.8-7-5.3-7-9.5V6z"/><path d="M9 12l2 2 4-4"/>',
  wifi: '<path d="M2.5 9a14 14 0 0119 0M5.5 12.3a9.5 9.5 0 0113 0M8.6 15.5a5 5 0 016.8 0"/><circle cx="12" cy="19" r="0.6"/>',
  power: '<path d="M12 3v8"/><path d="M6.3 6.8a8 8 0 1011.4 0"/>',
  bell: '<path d="M6 16V11a6 6 0 0112 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 004 0"/>',
  clip: '<rect x="5.5" y="4.5" width="13" height="16.5" rx="2"/><path d="M9 4.5V3.5h6v1"/><path d="M9 11h6M9 15h4"/>',
  inarrow: '<path d="M17 7L7 17M7 9v8h8"/>',
  outarrow: '<path d="M7 17L17 7M9 7h8v8"/>',
  alert: '<path d="M12 4l9 16H3z"/><path d="M12 10v4.5"/><circle cx="12" cy="17.3" r="0.5"/>',
  qr: '<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><path d="M14 14h2v2h-2zM18 18h2v2h-2zM14 18h2M18 14h2"/>',
  camera: '<path d="M4 8h3l1.5-2h7L17 8h3v11H4z"/><circle cx="12" cy="13" r="3.5"/>',
  keypad: '<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 10h.01M11 10h.01M15 10h.01M7 14h10"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.5 2.7 2.5 14.3 0 17M12 3.5c-2.5 2.7-2.5 14.3 0 17"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
  pause: '<path d="M9 6v12M15 6v12"/>',
  share: '<path d="M12 3v12M8 7l4-4 4 4"/><path d="M6 11H5v10h14V11h-1"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 00-1-1H5a1 1 0 00-1 1v10a1 1 0 001 1h3"/>',
  refresh: '<path d="M20 11a8 8 0 10-2.3 5.7"/><path d="M20 5v6h-6"/>',
  battery: '<rect x="3" y="7" width="16" height="10" rx="2"/><path d="M21 10.5v3"/><path d="M6 10v4"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M2.8 12h2.4M18.8 12h2.4M5.5 5.5l1.7 1.7M16.8 16.8l1.7 1.7M5.5 18.5l1.7-1.7M16.8 7.2l1.7-1.7"/>',
  more: '<circle cx="6" cy="12" r="1.3" fill="currentColor"/><circle cx="12" cy="12" r="1.3" fill="currentColor"/><circle cx="18" cy="12" r="1.3" fill="currentColor"/>',
  pencil: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
  trash: '<path d="M5 7h14M10 7V4.5h4V7M7 7l1 13h8l1-13"/>',
  action: '<rect x="8" y="2.5" width="8" height="19" rx="2.4"/><path d="M8 7.5H6.5"/>',
  tap: '<path d="M9 11V5.5a1.5 1.5 0 013 0V11"/><path d="M12 10a1.5 1.5 0 013 0v1M15 10.5a1.5 1.5 0 013 0V15a6 6 0 01-6 6h-.5a5 5 0 01-4.2-2.3L5 15a1.4 1.4 0 012.2-1.7L9 15"/>',
  grid: '<rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/>',
  sliders: '<path d="M5 6h9M18 6h1M5 12h3M12 12h7M5 18h11M20 18h-1"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>',
  help: '<circle cx="12" cy="12" r="8.5"/><path d="M9.6 9.5a2.5 2.5 0 014.8 1c0 1.7-2.4 2-2.4 3.5"/><circle cx="12" cy="17" r="0.5"/>',
  lock: '<rect x="5" y="10.5" width="14" height="10" rx="2"/><path d="M8 10.5V8a4 4 0 018 0v2.5"/>',
  image: '<rect x="3.5" y="5" width="17" height="14" rx="2"/><circle cx="9" cy="10" r="1.6"/><path d="M20.5 16l-5-5-8 8"/>',
};
const ic = (n, cls = "") => `<svg class="ic ${cls}" viewBox="0 0 24 24" aria-hidden="true">${P[n]}</svg>`;

// Placeholder app mark: a ticket with a perforation, on navy.
const appmark = (cls = "appmark") => `<svg class="${cls}" viewBox="0 0 64 64" aria-hidden="true"><rect width="64" height="64" rx="15" fill="#22346b"/><path d="M14 22a3 3 0 013-3h30a3 3 0 013 3v5a5 5 0 000 10v5a3 3 0 01-3 3H17a3 3 0 01-3-3v-5a5 5 0 000-10z" fill="#f7e6a0"/><path d="M38 21v22" stroke="#ad2b21" stroke-width="2" stroke-dasharray="2.5 2.5"/><rect x="20" y="28" width="12" height="2.6" rx="1.3" fill="#ad2b21"/><rect x="20" y="33.4" width="8" height="2.6" rx="1.3" fill="#ad2b21"/></svg>`;

// Illustrative QR: finder patterns plus seeded modules. Not a real code.
function qr(seed = 7, cls = "qr") {
  const n = 25; let s = seed; const rnd = () => ((s = (s * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  let r = "";
  const finder = (x, y) => `<rect x="${x}" y="${y}" width="7" height="7" fill="#17233f"/><rect x="${x + 1}" y="${y + 1}" width="5" height="5" fill="#fff"/><rect x="${x + 2}" y="${y + 2}" width="3" height="3" fill="#17233f"/>`;
  const inF = (x, y) => (x < 8 && y < 8) || (x > n - 9 && y < 8) || (x < 8 && y > n - 9);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    if (inF(x, y)) continue;
    const timing = (y === 6 || x === 6) ? (x + y) % 2 === 0 : rnd() > 0.52;
    if (timing) r += `<rect x="${x}" y="${y}" width="1" height="1"/>`;
  }
  return `<svg class="${cls}" viewBox="0 0 ${n} ${n}" shape-rendering="crispEdges" aria-label="QR code"><g fill="#17233f">${r}</g>${finder(0, 0)}${finder(n - 7, 0)}${finder(0, n - 7)}</svg>`;
}

// ---------- frames ----------
const wctl = `<span class="wc"><i><svg viewBox="0 0 10 10"><path d="M0 5h10"/></svg></i><i><svg viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9"/></svg></i><i><svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10"/></svg></i></span>`;
function win(os, body, { cls = "", flush = false } = {}) {
  let bar;
  if (os === "win") bar = `<div class="titlebar">${appmark()}<span>Universal Clipboard</span>${wctl}</div>`;
  else if (os === "mac") bar = `<div class="titlebar"><span class="lights"><i></i><i></i><i></i></span>Universal Clipboard</div>`;
  else bar = `<div class="titlebar">Universal Clipboard<span class="close"><svg viewBox="0 0 10 10"><path d="M1 1l8 8M9 1L1 9"/></svg></span></div>`;
  return `<div class="win os-${os} ${cls}">${bar}<div class="content${flush ? " flush" : ""}">${body}</div></div>`;
}
const shot = (title, note, inner) => `<figure class="shot">${inner}<figcaption><b>${title}</b>${note}</figcaption></figure>`;

const sigbars = `<svg viewBox="0 0 18 12"><rect x="0" y="8" width="3" height="4" rx="1"/><rect x="5" y="5.5" width="3" height="6.5" rx="1"/><rect x="10" y="3" width="3" height="9" rx="1"/><rect x="15" y="0" width="3" height="12" rx="1"/></svg>`;
const batt = `<svg viewBox="0 0 26 12"><rect x="0.5" y="0.5" width="22" height="11" rx="3" fill="none" stroke="currentColor" opacity=".5"/><rect x="2" y="2" width="17" height="8" rx="1.6"/><rect x="23.5" y="4" width="2" height="4" rx="1"/></svg>`;
const wifiS = `<svg viewBox="0 0 16 12"><path d="M8 12l-2.6-3.2a4 4 0 015.2 0zM8 0a13 13 0 018 2.8l-1.7 2.1a10.3 10.3 0 00-12.6 0L0 2.8A13 13 0 018 0z"/></svg>`;
function ios(body, { overlay = "", bg = "" } = {}) {
  return `<div class="device ios"><div class="screen" ${bg ? `style="background:${bg}"` : ""}><div class="statusbar"><span>9:41</span><span class="island"></span><span class="sys">${sigbars}${wifiS}${batt}</span></div>${body}${overlay}<span class="homebar"></span></div></div>`;
}
function android(body, { overlay = "", bg = "" } = {}) {
  return `<div class="device android"><div class="screen" ${bg ? `style="background:${bg}"` : ""}><div class="statusbar"><span>9:41</span><span class="punch"></span><span class="sys">${wifiS}${sigbars}${batt}</span></div>${body}${overlay}<span class="homebar"></span></div></div>`;
}
function ipad(body) {
  return `<div class="device ipad"><div class="screen"><div class="statusbar"><span>9:41  Fri 13 Nov</span><span class="sys">${wifiS}${batt}</span></div>${body}<span class="homebar"></span></div></div>`;
}

function page(file, title, intro, shots, extraHead = "") {
  const html = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title} · Universal Clipboard mockups</title>
<link rel="stylesheet" href="mockups.css">${extraHead}
</head>
<body>
<header class="board-head">
  <h1>${title}</h1>
  <p>${intro}</p>
</header>
<nav class="board-nav"><a class="back" href="index.html">All screens</a><span>Mockup only. Names, times and codes are examples.</span></nav>
<main class="board">
${shots.join("\n")}
</main>
</body>
</html>
`;
  writeFileSync(join(OUT, file), html);
}

// ---------- shared pieces ----------
const devIc = { pc: "desktop", mac: "desktop", laptop: "laptop", phone: "phone", tablet: "tablet" };
function devices(rows, openIdx = -1) {
  return `<ul class="devices">${rows.map((r, i) => `<li class="${r.cls || ""}"><span class="dic">${ic(r.icon)}</span><span><span class="dn">${r.name}${r.me ? '<span class="me">This device</span>' : ""}</span><span class="ds"><i class="mark ${r.mark}"></i>${r.state}</span></span>${r.me ? "<span></span>" : `<span class="more ${i === openIdx ? "open" : ""}" aria-label="More for ${r.name}">${ic("more")}</span>`}</li>`).join("")}</ul>`;
}
const DEV_MAC = [
  { icon: "desktop", name: "Max's Mac mini", me: true, mark: "on", state: "On" },
  { icon: "desktop", name: "Max's PC", mark: "on", state: "Connected" },
  { icon: "laptop", name: "Kitchen laptop", mark: "net", state: "Connected through the internet" },
  { icon: "phone", name: "Max's iPhone", mark: "away", state: "Asleep or away" },
];

function codeTicket({ code = "418 207 953", long = false, left = "9:42", stub = true, void_ = false, stamp = "" } = {}) {
  return `<div class="ticket ${void_ ? "void" : ""}"><div class="main"><div class="label">${long ? "Long code" : "Code"}</div><div class="code ${long ? "long" : ""}">${code}</div><div class="meta">${void_ ? "This code has ended." : `${ic("clock")}<span>Works for <span class="num">${left}</span> more</span>`}</div></div>${stub ? `<div class="stub">${qr(long ? 11 : 7)}<span class="qrcap">Scan with a phone</span></div>` : ""}${stamp ? `<span class="stamp" style="right:18px;top:16px">${stamp}</span>` : ""}</div>`;
}

// =====================================================================
// 1. Welcome
// =====================================================================
const welcome = (os) => win(os, `
  <div class="steps"><i class="on"></i><i></i><i></i></div>
  <div class="grow" style="display:flex;flex-direction:column;justify-content:center;gap:0">
    <svg viewBox="0 0 360 150" style="width:100%;height:auto;margin-bottom:22px" aria-hidden="true">
      <rect x="12" y="22" width="120" height="78" rx="8" fill="#fff" stroke="#17233f" stroke-width="2"/>
      <path d="M2 112h140" stroke="#17233f" stroke-width="2" stroke-linecap="round"/>
      <rect x="252" y="14" width="64" height="116" rx="12" fill="#fff" stroke="#17233f" stroke-width="2"/>
      <path d="M278 120h12" stroke="#17233f" stroke-width="2" stroke-linecap="round"/>
      <rect x="30" y="40" width="58" height="6" rx="3" fill="#cfeadb"/><rect x="30" y="52" width="80" height="6" rx="3" fill="#cfeadb"/><rect x="30" y="64" width="44" height="6" rx="3" fill="#17233f"/>
      <rect x="264" y="54" width="40" height="6" rx="3" fill="#17233f"/><rect x="264" y="66" width="30" height="6" rx="3" fill="#cfeadb"/>
      <g transform="translate(150 50)">
        <path d="M0 6a4 4 0 014-4h82a4 4 0 014 4v8a7 7 0 000 14v8a4 4 0 01-4 4H4a4 4 0 01-4-4v-8a7 7 0 000-14z" fill="#f7e6a0"/>
        <path d="M60 4v34" stroke="#ad2b21" stroke-width="1.5" stroke-dasharray="3 3"/>
        <rect x="12" y="14" width="34" height="5" rx="2.5" fill="#ad2b21"/><rect x="12" y="23" width="22" height="5" rx="2.5" fill="#ad2b21"/>
      </g>
    </svg>
    <h2 class="title" style="font-size:26px">Copy on one device, paste on another.</h2>
    <p class="lede">Set it up once. After that it works on its own, out of sight.</p>
  </div>
  <div class="btn-row split"><a href="#" class="small">Privacy policy</a><button class="btn primary">Continue</button></div>
`);

// =====================================================================
// 2. Permissions
// =====================================================================
const doneMark = `<span class="done"><span class="tick">${ic("check")}</span>Done</span>`;
function permWin(os, rows, extra = "") {
  return win(os, `
  <div class="steps"><i class="on"></i><i class="on"></i><i></i></div>
  <h2 class="title">A few permissions</h2>
  <p class="lede">Nothing is asked until you press a button.</p>
  <ul class="perm">${rows.map((r) => `<li class="${r.cls || ""}"><span class="pic">${ic(r.icon)}</span><span><span class="pt">${r.t}${r.opt ? ' <span class="opt">(optional)</span>' : ""}</span><span class="pd" style="display:block">${r.d}</span></span>${r.right}</li>`).join("")}</ul>
  ${extra}
  <div class="grow"></div>
  <div class="btn-row split"><button class="btn quiet">Back</button><button class="btn primary">Continue</button></div>`);
}
const permWinRows = [
  { icon: "shield", t: "Let your devices find this PC", d: "Windows asks once. Your other devices can then connect here.", right: doneMark },
  { icon: "power", t: "Start with Windows", d: "Keeps working after a restart.", right: '<button class="btn sm">Turn on</button>' },
];
const permWinRefused = [
  { icon: "shield", cls: "refused", t: "Let your devices find this PC", d: '<span class="warnline">You said no. Sync still works in most homes, but slower to start.</span>', right: '<button class="btn sm">Try again</button>' },
  { icon: "power", t: "Start with Windows", d: "Keeps working after a restart.", right: doneMark },
];
const permMacRows = [
  { icon: "wifi", t: "Find devices on your network", d: "macOS asks once. Needed to reach your other devices.", right: doneMark },
  { icon: "clip", t: "Read what you copy", d: "Opens System Settings. Pick Allow next to Universal Clipboard.", right: '<button class="btn sm primary">Open</button>' },
  { icon: "power", t: "Start at login", d: "Keeps working after a restart.", right: doneMark },
  { icon: "bell", t: "Tell me when sync stops", opt: true, d: "The only way the app can warn you. At most once a day.", right: '<button class="btn sm">Allow</button>' },
];
const permLinuxRows = [
  { icon: "power", t: "Start at login", d: "Keeps working after a restart.", right: '<button class="btn sm primary">Turn on</button>' },
];
const linuxNote = `<div class="notice mint" style="margin-top:12px">${ic("check")}<span>This desktop lets the app use the clipboard in the background. Nothing else to allow.</span></div>`;
const linuxBlocked = win("linux", `
  <div class="steps"><i class="on"></i><i class="on"></i><i></i></div>
  <h2 class="title">This desktop is not supported</h2>
  <p class="lede">This desktop blocks background clipboard access, so Universal Clipboard cannot work here.</p>
  <div class="notice sky">${ic("help")}<span>Supported desktops, and what you can do, are on the help page.</span></div>
  <div class="grow"></div>
  <div class="btn-row split"><button class="btn quiet">Quit</button><button class="btn primary">Open help page</button></div>`);

// =====================================================================
// 3. Add a device
// =====================================================================
const addWin = (os) => win(os, `
  <div class="steps"><i class="on"></i><i class="on"></i><i class="on"></i></div>
  <h2 class="title">Add a device</h2>
  <p class="lede">Pair two devices with a code. You do this once per device.</p>
  <div class="choices">
    <button class="choice"><span class="cic butter">${ic("qr")}</span><span><b>This is my first device</b><span>Show a code for your other device.</span></span><span class="chev">${ic("chev")}</span></button>
    <button class="choice"><span class="cic sky">${ic("keypad")}</span><span><b>I have a code</b><span>Type the code from your other device.</span></span><span class="chev">${ic("chev")}</span></button>
  </div>
  <div class="grow"></div>
  <div class="btn-row split"><button class="btn quiet">Back</button><span class="small">You can add more devices later.</span></div>`);

// =====================================================================
// 4. Show a code
// =====================================================================
const showShort = (os) => win(os, `
  <button class="btn quiet" style="align-self:flex-start;margin:-6px 0 6px -8px">${ic("back")}Back</button>
  <h2 class="title">Type this code on your other device</h2>
  <p class="lede">Or scan the QR code with a phone or tablet.</p>
  ${codeTicket()}
  <div class="waiting" style="margin-top:18px"><span class="spin"></span>Waiting for your other device</div>
  <div class="grow"></div>
  <a href="#" style="font-size:13.5px">The other device is not on this Wi-Fi, or it uses a VPN</a>`);
const showLong = (os) => win(os, `
  <button class="btn quiet" style="align-self:flex-start;margin:-6px 0 6px -8px">${ic("back")}Back</button>
  <h2 class="title">Use the long code</h2>
  <p class="lede">The long code works from anywhere, also through a VPN. Type it on your other device.</p>
  ${codeTicket({ code: "K7QF2-M9XTD-4HRWA-P3ZNC", long: true, left: "9:15", stub: false })}
  <div class="notice sky" style="margin-top:14px">${ic("globe")}<span>Phones and tablets can still scan the QR code. It works the same way.</span></div>
  <div class="waiting" style="margin-top:16px"><span class="spin"></span>Waiting for your other device</div>
  <div class="grow"></div>
  <a href="#" style="font-size:13.5px">Show the short code again</a>`);
const showExpired = (os) => win(os, `
  <h2 class="title">This code has ended</h2>
  <p class="lede">Codes work for 10 minutes. Make a new one and type it on your other device.</p>
  <div style="position:relative">${codeTicket({ void_: true, stub: false, stamp: "ENDED" })}</div>
  <div class="grow"></div>
  <div class="btn-row"><button class="btn primary block">Make a new code</button></div>`);
const showTried = (os) => win(os, `
  <h2 class="title">Type this code on your other device</h2>
  <p class="lede">Or scan the QR code with a phone or tablet.</p>
  ${codeTicket({ left: "6:03" })}
  <div class="notice salmon" style="margin-top:16px">${ic("alert")}<span><b>Someone else tried this code.</b> If that was not you, make a new code.</span></div>
  <div class="grow"></div>
  <div class="btn-row"><button class="btn">Make a new code</button></div>`);

// =====================================================================
// 5. Enter a code
// =====================================================================
const enterBase = (os, input, below, btn = '<button class="btn primary block">Connect</button>') => win(os, `
  <button class="btn quiet" style="align-self:flex-start;margin:-6px 0 6px -8px">${ic("back")}Back</button>
  <h2 class="title">Type the code from your other device</h2>
  <p class="lede">On your other device, open Universal Clipboard and press Add a device.</p>
  <div class="field"><label for="c">Code</label>${input}</div>
  ${below}
  <div class="grow"></div>
  ${btn}`);
const enterEmpty = (os) => enterBase(os, `<div class="codeinput focus"><span>418 207</span><span class="caret"></span><span class="ph">&nbsp;</span></div>`, `<div class="hint">9 digits. Or the long code with letters, if you were given one.</div>`);
const enterLooking = (os) => enterBase(os, `<div class="codeinput"><span>418 207 953</span></div>`, `<div class="waiting" style="margin-top:14px"><span class="spin"></span>Looking for the other device on this network</div>`, '<button class="btn primary block" disabled>Connect</button>');
const enterWrong = (os) => enterBase(os, `<div class="codeinput bad"><span>418 207 935</span></div>`, `<div class="err">${ic("alert")}<span>That code is not right. Check it and try again.</span></div>`);
const enterLocked = (os) => enterBase(os, `<div class="codeinput"><span class="ph">418 207 953</span></div>`, `<div class="notice salmon" style="margin-top:14px">${ic("alert")}<span>Too many tries. <b>Make a new code on the other device</b>, then type it here.</span></div>`, '<button class="btn primary block" disabled>Connect</button>');
const enterNotFound = (os) => enterBase(os, `<div class="codeinput"><span>418 207 953</span></div>`, `<div class="notice sky" style="margin-top:14px">${ic("wifi")}<span>Cannot find the other device on this network. On the other device, press <b>The other device is not on this Wi-Fi</b> and type the long code here.</span></div>`, '<button class="btn primary block">Try again</button>');
const enterJoin = (os) => win(os, `
  <h2 class="title">Type the code from your other device</h2>
  <p class="lede">On your other device, open Universal Clipboard and press Add a device.</p>
  <div class="field"><label>Code</label><div class="codeinput"><span>418 207 953</span></div></div>
  <div class="grow"></div>
  <button class="btn primary block">Connect</button>
  <div class="scrim"><div class="dialog"><h3>Leave your other devices and join these?</h3><p>This PC is paired with 2 other devices. It will stop syncing with them and join Max's Mac mini instead.</p><div class="btn-row"><button class="btn">Cancel</button><button class="btn primary">Join</button></div></div></div>`);

// =====================================================================
// 6. Success
// =====================================================================
const halves = (a, aic, b, bic, code = "418 207 953") => `<div class="halves"><div class="half l"><div class="dn">${ic(aic)}<span>${a}</span></div><div class="code">${code}</div><small>Kept here</small></div><span class="joined-mark">${ic("check")}</span><div class="half r"><div class="dn">${ic(bic)}<span>${b}</span></div><div class="code">${code}</div><small>Matched</small></div></div>`;
const success = (os, me, meic, other, otheric) => win(os, `
  <div class="steps"><i class="on"></i><i class="on"></i><i class="on"></i></div>
  <div class="grow" style="display:flex;flex-direction:column;justify-content:center">
    ${halves(me, meic, other, otheric)}
    <h2 class="title" style="margin-top:28px;font-size:24px">Connected to ${other}</h2>
    <p class="lede" style="margin-bottom:0">Done. You can close this window. Copy something to try it.</p>
  </div>
  <div class="btn-row split"><button class="btn quiet">Add another device</button><button class="btn primary">Close window</button></div>`);

// =====================================================================
// 7. Settings window
// =====================================================================
function settingsBody({ band, list = DEV_MAC, openIdx = -1, paused = false, extra = "", activity }) {
  return `${band}
  <div class="section"><div class="section-h"><span>Your devices</span></div>${devices(list, openIdx)}</div>
  <div class="section" style="display:flex;gap:10px;padding-top:10px">
    <button class="btn primary" style="flex:1">${ic("plus")}Add a device</button>
  </div>
  <div class="section stack" style="padding-top:8px">
    <div class="row-line"><span><b>Pause sync</b><span class="sub" style="display:block">${paused ? "Paused until 15:40" : "Stop for a while, or until you turn it on."}</span></span><span class="switch ${paused ? "on" : ""}"></span></div>
    <div class="disclosure"><span>Settings</span>${ic("chevdown")}</div>
  </div>
  <div class="section" style="padding-top:8px"><div class="section-h"><span>Last activity</span></div>${activity || `<div class="activity"><span class="aic">${ic("inarrow")}</span><span>Text, from Max's PC <time>· 2 minutes ago</time></span></div>`}</div>
  ${extra}`;
}
const bandWorking = `<div class="band mint"><span class="state"><i class="mark on"></i>Working</span><span class="proof">3 other devices. Last copy came in 2 minutes ago.</span></div>`;
const bandPaused = `<div class="band butter"><span class="state"><i class="mark paused"></i>Paused</span><span class="proof">Until 15:40. Your copies stay on this device.</span><button class="btn sm act">Resume</button></div>`;
const bandAlone = `<div class="band sky"><span class="state"><i class="mark away"></i>No other device is on right now</span><span class="proof">It syncs again when one of them wakes up.</span></div>`;
const bandAttn = (s, fix) => `<div class="band salmon"><span class="state" style="font-size:13px;font-weight:650;color:var(--salmon-ink)"><i class="mark blocked"></i>Needs attention</span><span class="sentence" style="grid-column:1">${s}</span><button class="btn sm primary act">${fix}</button></div>`;

const settingsMain = (os, list = DEV_MAC) => win(os, settingsBody({ band: bandWorking, list }), { flush: true, cls: "scroll-hint" });

const DEV_WIN = [
  { icon: "desktop", name: "Max's PC", me: true, mark: "on", state: "On" },
  { icon: "desktop", name: "Max's Mac mini", mark: "on", state: "Connected" },
  { icon: "laptop", name: "Kitchen laptop", mark: "net", state: "Connected through the internet" },
  { icon: "phone", name: "Max's iPhone", mark: "away", state: "Asleep or away" },
];
const DEV_LINUX = [
  { icon: "laptop", name: "Kitchen laptop", me: true, mark: "on", state: "On" },
  { icon: "desktop", name: "Max's PC", mark: "net", state: "Connected through the internet" },
  { icon: "desktop", name: "Max's Mac mini", mark: "net", state: "Connected through the internet" },
  { icon: "phone", name: "Pixel 9", mark: "on", state: "Connected" },
];

const settingsExpanded = (os) => win(os, `
  <div style="padding:14px 24px 8px;display:flex;align-items:center;gap:8px;color:var(--ink-2);font-size:12.5px">${ic("chevdown")}Scrolled down</div>
  <div class="section stack" style="padding-top:0">
    <div class="disclosure" style="border-radius:10px 10px 0 0;box-shadow:none;border-bottom:1px solid var(--line-soft)"><span>Settings</span>${ic("chevdown")}</div>
  </div>
  <div class="section" style="padding-top:0;margin-top:-4px"><div class="group" style="border-radius:0 0 10px 10px">
    <div><span>Start at login</span><span class="switch on"></span></div>
    <div><span>Use the internet when the local network is blocked<span class="sub" style="display:block">Content stays encrypted. Our relay cannot read it.</span></span><span class="switch on"></span></div>
    <div><span>Sync images</span><span class="switch on"></span></div>
    <div><span>Keep formatting<span class="sub" style="display:block">Off means plain text only.</span></span><span class="switch on"></span></div>
    <div><span>Also pass on what Apple's Universal Clipboard brings</span><span class="switch"></span></div>
    <div><span>This device's name</span><span class="muted">Max's Mac mini ${ic("pencil", "")}</span></div>
  </div></div>
  <div class="section" style="padding-top:14px"><div class="section-h"><span>Help</span></div><div class="group">
    <div><span>Copy a diagnostic report<span class="sub" style="display:block">No clipboard content, no keys.</span></span><button class="btn sm">Copy</button></div>
    <div><span>Privacy policy</span>${ic("chev")}</div>
    <div><span class="danger-text">Remove this device and forget everything</span></div>
    <div><span class="sub">Version 1.0.0</span><span class="sub">Up to date</span></div>
  </div></div>`, { flush: true, cls: "scroll-hint" });

// =====================================================================
// 8. Pause and remove
// =====================================================================
const pauseMenu = (os) => win(os, settingsBody({ band: bandWorking }) + `<div class="pop" style="right:28px;top:378px"><div class="hl">${ic("clock")}Pause for 1 hour</div><div>${ic("pause")}Until I turn it on</div></div>`, { flush: true, cls: "scroll-hint" });
const pausedState = (os) => win(os, settingsBody({ band: bandPaused, paused: true, activity: `<div class="activity"><span class="aic">${ic("pause")}</span><span>Paused by you <time>· 14:40</time></span></div>` }), { flush: true, cls: "scroll-hint" });
const rowMenu = (os) => win(os, settingsBody({ band: bandWorking, openIdx: 3 }) + `<div class="pop" style="right:30px;top:300px"><div>${ic("pencil")}Rename</div><div class="red">${ic("trash")}Remove</div></div>`, { flush: true, cls: "scroll-hint" });
const removeConfirm = (os) => win(os, settingsBody({ band: bandWorking }) + `<div class="scrim"><div class="dialog"><h3>Remove Max's iPhone?</h3><p>It stops syncing right away, on all your devices. You can add it again later with a new code.</p><div class="btn-row"><button class="btn">Cancel</button><button class="btn danger">Remove</button></div></div></div>`, { flush: true, cls: "scroll-hint" });
const removedRow = (os) => {
  const list = DEV_MAC.slice(0, 3).concat([{ icon: "phone", name: "Max's iPhone", mark: "away", state: "Removed just now", cls: "gone" }]);
  return win(os, settingsBody({ band: `<div class="band mint"><span class="state"><i class="mark on"></i>Working</span><span class="proof">2 other devices. Last copy came in 2 minutes ago.</span></div>`, list }) + `<span class="stamp" style="right:44px;top:276px;font-size:20px;border-width:2px">REMOVED</span>`, { flush: true, cls: "scroll-hint" });
};
const forgetConfirm = (os) => win(os, settingsExpanded(os).replace(/^.*?<div class="content flush">/s, "").replace(/<\/div><\/div>$/s, "") + `<div class="scrim"><div class="dialog"><h3>Remove this device and forget everything?</h3><p>Your other devices stop syncing with this one. The app deletes its keys and settings and stops starting at login. You can set it up again later.</p><div class="btn-row"><button class="btn">Cancel</button><button class="btn danger">Remove and forget</button></div></div></div>`, { flush: true, cls: "scroll-hint" });

// =====================================================================
// 9. Attention
// =====================================================================
const DEV_BLOCKED = [
  { icon: "desktop", name: "Max's PC", me: true, mark: "on", state: "On" },
  { icon: "desktop", name: "Max's Mac mini", mark: "blocked", state: "Blocked by a VPN or firewall" },
  { icon: "laptop", name: "Kitchen laptop", mark: "blocked", state: "Blocked by a VPN or firewall" },
  { icon: "phone", name: "Max's iPhone", mark: "away", state: "Asleep or away" },
];
const attnNoPath = (os) => win(os, `${bandAttn("Your devices cannot reach each other on this network.", "Use the internet path")}
  <div class="section" style="padding-top:10px"><a href="#" style="font-size:13px">Why? A VPN, a guest Wi-Fi or a firewall can cause this</a></div>
  <div class="section"><div class="section-h"><span>Your devices</span></div>${devices(DEV_BLOCKED)}</div>
  <div class="section" style="padding-top:12px"><button class="btn primary" style="width:100%">${ic("plus")}Add a device</button></div>
  <div class="section" style="padding-top:8px"><div class="section-h"><span>Last activity</span></div><div class="activity"><span class="aic">${ic("outarrow")}</span><span>Text, to your devices <time>· 1 hour ago</time></span></div></div>`, { flush: true, cls: "scroll-hint" });
const attnSimple = (os, sentence, fix, list = DEV_MAC, activity) => win(os, settingsBody({ band: bandAttn(sentence, fix), list, activity }), { flush: true, cls: "scroll-hint" });
const helpVpn = (os) => win(os, `
  <button class="btn quiet" style="align-self:flex-start;margin:-6px 0 6px -8px">${ic("back")}Back</button>
  <h2 class="title">Your VPN blocks the local network</h2>
  <p class="lede">Many VPN apps hide your other devices, even when they sit on the same Wi-Fi.</p>
  <div class="help"><ol>
    <li>Open your VPN app's settings.</li>
    <li>Look for <b>Allow LAN access</b> or <b>Local network sharing</b>, and turn it on.</li>
    <li>Come back here. Sync starts again on its own.</li>
  </ol></div>
  <div class="notice sky" style="margin-top:18px">${ic("globe")}<span>Or use the internet path. Your copies stay encrypted, and the relay cannot read them.</span></div>
  <div class="grow"></div>
  <button class="btn primary block">Use the internet path</button>`);
const removedHere = (os) => win(os, `
  <div class="grow" style="display:flex;flex-direction:column;justify-content:center">
    <div style="position:relative">${halves("Max's PC", "desktop", "Max's Mac mini", "desktop")}<span class="stamp" style="left:50%;top:-18px;margin-left:-70px">REMOVED</span></div>
    <h2 class="title" style="margin-top:28px">Another device removed this one</h2>
    <p class="lede" style="margin-bottom:0">This PC no longer syncs. Set it up again if this was a mistake.</p>
  </div>
  <div class="btn-row split"><button class="btn quiet">Pair my devices again</button><button class="btn primary">Set up again</button></div>`);
const toastWin = `<div class="toast"><div class="th">${appmark()}Universal Clipboard</div><b>Sync stopped on this PC</b>Open the app to fix it. It has not worked for a day.<div><button class="btn sm primary">Open</button></div></div>`;

// =====================================================================
// Mobile: iPhone
// =====================================================================
const iWelcome = ios(`<div class="mbody">
  <div style="flex:1;display:flex;flex-direction:column;justify-content:center">
    ${appmark("appmark-lg")}
    <h1 class="largetitle" style="margin-top:22px">Copy on one device, paste on another.</h1>
    <p class="mlede">Phones need one tap. Apple and Google do not allow more.</p>
    <div class="notice sky" style="font-size:15px">${ic("desktop")}<span>You need at least one computer with Universal Clipboard on it.</span></div>
  </div>
  <button class="btn primary block">Continue</button>
  <a href="#" style="text-align:center;margin-top:14px;font-size:15px">Privacy policy</a></div>`);
const iPerms = ios(`<div class="mbody">
  <div class="navrow">${ic("back")}<span></span></div>
  <h1 class="largetitle">Permissions</h1>
  <p class="mlede">iOS asks only when you tap.</p>
  <div class="inset">
    <div><span class="lic" style="background:var(--mint);color:var(--mint-ink)">${ic("wifi")}</span><span class="lt"><b>Local Network</b><span>Finds your computers on this Wi-Fi.</span></span>${doneMark}</div>
    <div><span class="lic" style="background:var(--sky);color:var(--sky-ink)">${ic("camera")}</span><span class="lt"><b>Camera</b><span>Asked only when you scan a code.</span></span><span class="small">Later</span></div>
  </div>
  <div style="flex:1"></div>
  <button class="btn primary block">Continue</button></div>`);
const iPair = ios(`<div class="mbody">
  <div class="navrow">${ic("back")}<span></span></div>
  <h1 class="largetitle">Add this iPhone</h1>
  <p class="mlede">On your computer, open Universal Clipboard and press Add a device.</p>
  <div class="ticket" style="--notch:var(--ground);margin:8px 0 24px"><div class="main"><div class="label">The code looks like this</div><div class="code" style="font-size:38px">418 207 953</div><div class="meta">Shown on your computer</div></div><div class="stub">${qr(3).replace('class="qr"', 'class="qr" style="width:72px;height:72px"')}</div></div>
  <div style="flex:1"></div>
  <button class="btn primary block">${ic("camera")}Scan QR code</button>
  <button class="btn block" style="margin-top:10px;background:transparent;box-shadow:none;color:var(--primary)">Type a code instead</button></div>`);
const iScan = ios(`<div class="viewfinder"><div class="frame" style="display:grid;place-items:center"><i></i><i></i><i></i><i></i>${qr(9,"qrghost")}</div><div style="position:absolute;top:24px;left:0;right:0;text-align:center;font-weight:600">Point at the code on your computer</div><div style="position:absolute;bottom:60px;left:20px;right:20px"><button class="btn block" style="background:rgba(255,255,255,.16);color:#fff;box-shadow:none">Type a code instead</button></div></div>`, { bg: "#1b2230" }).replace('class="statusbar"', 'class="statusbar" style="color:#fff;background:#1b2230"');
const iType = ios(`<div class="mbody" style="padding-bottom:0">
  <div class="navrow"><span>Cancel</span><b style="color:var(--ink)">Type a code</b><span style="color:var(--ink-3)">Connect</span></div>
  <p class="mlede" style="margin-top:14px">9 digits, or the long code with letters.</p>
  <div class="codeinput focus" style="height:68px;font-size:40px"><span>418 207</span><span class="caret"></span></div>
  <div class="err" style="display:none"></div>
  </div>
  <div style="background:#d5d9e0;height:290px;display:grid;grid-template-columns:repeat(3,1fr);gap:7px;padding:8px 7px 44px">${[1,2,3,4,5,6,7,8,9,"",0,"⌫"].map(k=>`<div style="background:${k===""?"transparent":"#fff"};border-radius:7px;display:grid;place-items:center;font-size:24px">${k==="⌫"?ic("back"):k}</div>`).join("")}</div>`);
const iDone = ios(`<div class="mbody">
  <div style="flex:1;display:flex;flex-direction:column;justify-content:center">
    ${halves("This iPhone", "phone", "Max's PC", "desktop").replace('class="halves"', 'class="halves" style="transform:scale(.98)"')}
    <h1 class="largetitle" style="margin-top:28px;font-size:30px">Connected to Max's PC</h1>
    <p class="mlede">Next, make sending and getting one tap.</p>
  </div>
  <button class="btn primary block">Continue</button></div>`);
const iOneTap = ios(`<div class="mbody" style="padding-bottom:30px">
  <div class="navrow"><span></span><span>Skip</span></div>
  <h1 class="largetitle" style="font-size:30px">Make it one tap</h1>
  <p class="mlede" style="margin-bottom:12px">Add two Shortcuts. Apple's Shortcuts app does the copy and paste part.</p>
  <div class="inset">
    <div><span class="lic" style="background:var(--primary);color:#fff">${ic("outarrow")}</span><span class="lt"><b>Send clipboard</b><span>Sends what you copied.</span></span><button class="btn sm" style="height:34px;border-radius:17px">Add</button></div>
    <div><span class="lic" style="background:var(--primary);color:#fff">${ic("inarrow")}</span><span class="lt"><b>Get from my devices</b><span>Copies the newest clip here.</span></span>${doneMark}</div>
  </div>
  <div class="ghead">Then put them where your thumb is</div>
  <div class="inset">
    <div><span class="lic" style="background:var(--butter);color:var(--butter-ink)">${ic("action")}</span><span class="lt"><b>Action Button</b><span>Settings, Action Button, Shortcut.</span></span>${ic("chev")}</div>
    <div><span class="lic" style="background:var(--butter);color:var(--butter-ink)">${ic("tap")}</span><span class="lt"><b>Back Tap</b><span>Tap the back of the iPhone twice.</span></span>${ic("chev")}</div>
    <div><span class="lic" style="background:var(--butter);color:var(--butter-ink)">${ic("sliders")}</span><span class="lt"><b>Control Center</b></span>${ic("chev")}</div>
    <div><span class="lic" style="background:var(--butter);color:var(--butter-ink)">${ic("grid")}</span><span class="lt"><b>Home Screen</b></span>${ic("chev")}</div>
  </div>
  <p class="small" style="margin:12px 4px 0;font-size:13px">In any app, the Share button also has <b>Send to my devices</b>.</p></div>`);
const iPasteAllow = ios(`<div class="mbody">
  <div class="navrow">${ic("back")}<span>Skip</span></div>
  <h1 class="largetitle" style="font-size:30px">Skip the paste question</h1>
  <p class="mlede">Optional. iOS asks every time the app reads what you copied. You can turn that off.</p>
  <div class="inset">
    <div><span class="lt"><b>1. Open Settings</b></span></div>
    <div><span class="lt"><b>2. Tap Universal Clipboard</b></span></div>
    <div><span class="lt"><b>3. Paste from Other Apps: Allow</b></span></div>
  </div>
  <div style="flex:1"></div>
  <button class="btn primary block">Open Settings</button></div>`);

const iHome = (overlay = "", band = null) => ios(`<div class="mbody">
  <div class="navrow"><span></span>${ic("gear")}</div>
  <h1 class="largetitle">Universal Clipboard</h1>
  ${band || `<div class="mband mint"><span class="state"><i class="mark on"></i>Copied to this iPhone</span><div class="proof">Text, from Max's PC, 1 minute ago. You can paste it now.</div></div>`}
  <div style="margin-top:22px"><span class="paste">${ic("clip")}Paste</span><p class="small" style="font-size:14px;margin:8px 4px 0">Sends what you copied on this iPhone to your devices.</p></div>
  <div class="ghead">Your devices</div>
  <div class="inset">
    <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's PC</b><span><i class="mark on"></i> Connected</span></span></div>
    <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's Mac mini</b><span><i class="mark on"></i> Connected</span></span></div>
    <div><span class="lic" style="color:var(--ink-2)">${ic("tablet")}</span><span class="lt"><b>Max's iPad</b><span><i class="mark away"></i> Asleep or away</span></span></div>
  </div>
  <button class="btn block" style="margin-top:14px;background:transparent;box-shadow:none;color:var(--primary)">${ic("plus")}Add a device</button>
  </div>`, { overlay });
const iSent = iHome(`<div class="banner">${appmark()}<span><b>Sent to your devices</b>Text, to 3 devices.</span></div>`, `<div class="mband mint"><span class="state"><i class="mark on"></i>Sent</span><div class="proof">Text, from this iPhone, just now.</div></div>`);
const iNoComputer = ios(`<div class="mbody">
  <div class="navrow"><span></span>${ic("gear")}</div>
  <h1 class="largetitle">Universal Clipboard</h1>
  <div class="mband sky"><span class="state" style="font-size:20px"><i class="mark away"></i>No computer is on right now</span><div class="proof">Phones can only sync while one of your computers is on.</div></div>
  <button class="btn primary block" style="margin-top:18px">${ic("refresh")}Retry</button>
  <div class="ghead">Your devices</div>
  <div class="inset">
    <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's PC</b><span><i class="mark away"></i> Asleep or away</span></span></div>
    <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's Mac mini</b><span><i class="mark away"></i> Asleep or away</span></span></div>
  </div></div>`);
const iShare = ios(`<div class="mbody"><div class="navrow">${ic("back")}<span>Notes</span></div><div class="textblock" style="margin-top:12px">Meeting moved to Thursday at 10. The address is on the invite.</div></div>`,
  { overlay: `<div class="dim"></div><div class="sheet"><div class="grab"></div><div style="display:flex;gap:12px;align-items:center;padding:0 4px 14px;border-bottom:1px solid var(--line-soft)"><span style="width:44px;height:44px;border-radius:10px;background:#f6e7a0;flex:none"></span><span style="font-size:15px"><b style="display:block">Meeting moved</b><span class="small">Note</span></span></div>
  <div class="shareapps" style="margin-top:14px"><div class="ours"><i>${appmark().replace('class="appmark"', 'style="width:60px;height:60px"')}</i>Send to my devices</div><div><i></i>Messages</div><div><i></i>Mail</div><div><i></i>Notes</div></div>
  <div class="inset" style="background:#f2f3f6"><div>${ic("copy")}<span class="lt"><b>Copy</b></span></div><div>${ic("outarrow")}<span class="lt"><b>Send clipboard</b><span>Shortcut</span></span></div></div></div>` });

// iPad
const ipadHome = ipad(`<div style="display:grid;grid-template-columns:300px 1fr;flex:1;min-height:0">
  <div style="background:var(--surface);padding:18px 18px;border-right:1px solid var(--line-soft)">
    <h1 class="largetitle" style="font-size:28px">Universal Clipboard</h1>
    <div class="mband mint" style="margin-top:8px;padding:14px"><span class="state" style="font-size:18px"><i class="mark on"></i>Copied to this iPad</span><div class="proof" style="font-size:14px">Image, from Max's Mac mini, just now.</div></div>
    <div style="margin-top:16px"><span class="paste">${ic("clip")}Paste</span><p class="small" style="font-size:13px;margin:6px 2px 0">Sends what you copied on this iPad.</p></div>
  </div>
  <div style="padding:18px 26px;overflow:hidden">
    <div class="ghead" style="margin-top:6px">Make it one tap on iPad</div>
    <div class="inset" style="background:var(--surface)">
      <div><span class="lic" style="background:var(--butter);color:var(--butter-ink)">${ic("sliders")}</span><span class="lt"><b>Control Center</b><span>Add the Shortcuts control, then pick Send clipboard.</span></span>${ic("chev")}</div>
      <div><span class="lic" style="background:var(--butter);color:var(--butter-ink)">${ic("grid")}</span><span class="lt"><b>Home Screen</b><span>Put both Shortcuts on the Home Screen.</span></span>${ic("chev")}</div>
    </div>
    <div class="ghead">Your devices</div>
    <div class="inset">
      <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's Mac mini</b><span><i class="mark on"></i> Connected</span></span></div>
      <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's PC</b><span><i class="mark on"></i> Connected</span></span></div>
      <div><span class="lic" style="color:var(--ink-2)">${ic("phone")}</span><span class="lt"><b>Max's iPhone</b><span><i class="mark away"></i> Asleep or away</span></span></div>
    </div>
  </div></div>`);

// =====================================================================
// Mobile: Android
// =====================================================================
const aWelcome = android(`<div class="mbody">
  <div style="flex:1;display:flex;flex-direction:column;justify-content:center">
    ${appmark("appmark-lg")}
    <h1 class="mdtitle" style="margin-top:22px;font-size:32px">Copy on one device, paste on another.</h1>
    <p class="mlede">Phones need one tap. Apple and Google do not allow more.</p>
  </div>
  <button class="btn primary block">Continue</button></div>`);
const aPerms = android(`<div class="mbody">
  <div class="topbar">${ic("back")}<span>Permissions</span></div>
  <p class="mlede">Android asks only when you tap.</p>
  <div class="inset">
    <div><span class="lic" style="background:var(--mint);color:var(--mint-ink)">${ic("bell")}</span><span class="lt"><b>Notifications</b><span>Android needs a small notice while the app runs.</span></span>${doneMark}</div>
    <div><span class="lic" style="background:var(--sky);color:var(--sky-ink)">${ic("wifi")}</span><span class="lt"><b>Nearby devices</b><span>Finds your computers on this Wi-Fi.</span></span><button class="btn sm primary" style="height:36px;border-radius:18px">Allow</button></div>
  </div>
  <p class="small" style="font-size:14px;margin:14px 6px 0">No camera permission needed. Google's code scanner reads QR codes.</p>
  <div style="flex:1"></div>
  <button class="btn primary block">Continue</button></div>`);
const aPermRefused = android(`<div class="mbody">
  <div class="topbar">${ic("back")}<span>Permissions</span></div>
  <p class="mlede">Android asks only when you tap.</p>
  <div class="inset">
    <div><span class="lic" style="background:var(--mint);color:var(--mint-ink)">${ic("bell")}</span><span class="lt"><b>Notifications</b><span>Android needs a small notice while the app runs.</span></span>${doneMark}</div>
    <div><span class="lic" style="background:var(--salmon);color:var(--salmon-ink)">${ic("wifi")}</span><span class="lt"><b>Nearby devices</b><span style="color:var(--salmon-ink)">Turned off. Sync uses the internet only.</span></span><button class="btn sm" style="height:36px;border-radius:18px">How to fix</button></div>
  </div>
  <div style="flex:1"></div>
  <button class="btn primary block">Continue</button></div>`);
const aPair = android(`<div class="mbody">
  <div class="topbar">${ic("back")}<span>Add this phone</span></div>
  <p class="mlede">On your computer, open Universal Clipboard and press Add a device.</p>
  <div class="ticket" style="margin:8px 0 24px"><div class="main"><div class="label">The code looks like this</div><div class="code" style="font-size:38px">418 207 953</div><div class="meta">Shown on your computer</div></div><div class="stub">${qr(5).replace('class="qr"', 'class="qr" style="width:72px;height:72px"')}</div></div>
  <div style="flex:1"></div>
  <button class="btn primary block">${ic("qr")}Scan QR code</button>
  <button class="btn block" style="margin-top:10px;background:transparent;box-shadow:none;color:var(--primary)">Type a code instead</button></div>`);
const aDone = android(`<div class="mbody">
  <div style="flex:1;display:flex;flex-direction:column;justify-content:center">
    ${halves("This phone", "phone", "Kitchen laptop", "laptop")}
    <h1 class="mdtitle" style="margin-top:28px">Connected to Kitchen laptop</h1>
    <p class="mlede">Copies from your computers now arrive here on their own.</p>
  </div>
  <button class="btn primary block">Continue</button></div>`);
const aOneTap = android(`<div class="mbody">
  <div class="topbar"><span style="flex:1">Make it one tap</span><span style="font-size:15px;color:var(--primary)">Skip</span></div>
  <p class="mlede">Two ways to send from this phone.</p>
  <div class="inset">
    <div><span class="lic" style="background:var(--sky);color:var(--sky-ink)">${ic("grid")}</span><span class="lt"><b>Quick Settings tile</b><span>Swipe down, tap Send clipboard.</span></span><button class="btn sm primary" style="height:36px;border-radius:18px">Add tile</button></div>
    <div><span class="lic" style="background:var(--sky);color:var(--sky-ink)">${ic("copy")}</span><span class="lt"><b>In the text menu</b><span>Select text, then tap Send to my devices instead of Copy.</span></span></div>
  </div>
  <div style="margin-top:22px;display:flex;justify-content:center"><span class="seltool"><span>Cut</span><span>Copy</span><span class="ours">Send to my devices</span></span></div>
  <div style="flex:1"></div>
  <button class="btn primary block">Continue</button></div>`);
const aBattery = android(`<div class="mbody">
  <div class="topbar">${ic("back")}<span>One last thing</span></div>
  <div class="mband sky" style="margin-top:8px"><span class="state" style="font-size:20px">${ic("battery")}Battery saving</span><div class="proof">Keep battery saving off for this app if the phone misses clips.</div></div>
  <div style="flex:1"></div>
  <button class="btn primary block">Open battery settings</button>
  <button class="btn block" style="margin-top:10px;background:transparent;box-shadow:none;color:var(--primary)">Done</button></div>`);
const aHome = (overlay = "") => android(`<div class="mbody">
  <div class="topbar"><span style="flex:1">Universal Clipboard</span>${ic("gear")}</div>
  <div class="mband mint"><span class="state"><i class="mark on"></i>Working</span><div class="proof">Copies from your computers arrive on their own. Last one: text, from Max's PC, 2 minutes ago.</div></div>
  <div class="ghead">Your devices</div>
  <div class="inset">
    <div><span class="lic" style="color:var(--ink-2)">${ic("desktop")}</span><span class="lt"><b>Max's PC</b><span><i class="mark on"></i> Connected</span></span>${ic("more")}</div>
    <div><span class="lic" style="color:var(--ink-2)">${ic("laptop")}</span><span class="lt"><b>Kitchen laptop</b><span><i class="mark net"></i> Connected through the internet</span></span>${ic("more")}</div>
    <div><span class="lic" style="color:var(--ink-2)">${ic("tablet")}</span><span class="lt"><b>Max's iPad</b><span><i class="mark away"></i> Asleep or away</span></span>${ic("more")}</div>
  </div>
  <button class="btn block" style="margin-top:14px;background:transparent;box-shadow:none;color:var(--primary)">${ic("plus")}Add a device</button>
  </div><span class="fab">${ic("outarrow")}Send clipboard</span>`, { overlay });
const aSent = aHome(`<div class="snack"><span>Sent to 3 devices</span><span></span></div>`);
const aShade = android(`<div class="shade">
  <div style="display:flex;justify-content:space-between;font-size:14px;opacity:.8;padding:4px 6px"><span>Fri 13 Nov</span><span>${ic("gear")}</span></div>
  <div class="tiles">
    <div class="tile">${ic("wifi")}<span><b>Home Wi-Fi</b><span>Connected</span></span></div>
    <div class="tile on">${ic("outarrow")}<span><b>Send clipboard</b><span>Universal Clipboard</span></span></div>
    <div class="tile">${ic("bell")}<span><b>Do not disturb</b><span>Off</span></span></div>
    <div class="tile">${ic("battery")}<span><b>Battery saver</b><span>Off</span></span></div>
  </div>
  <div class="notif"><div class="nh">${appmark().replace('class="appmark"', 'style="width:16px;height:16px"')}Universal Clipboard</div><b>Syncing with 3 devices</b><span style="opacity:.8">Android needs this notice while the app runs.</span></div>
  <div class="notif"><div class="nh">${appmark().replace('class="appmark"', 'style="width:16px;height:16px"')}Universal Clipboard · now</div><b>Tap to copy</b><span style="opacity:.8">Text, from Max's PC</span><div class="nact"><span>Copy</span><span>Dismiss</span></div></div>
  </div>`, { bg: "#1f2533" }).replace('class="statusbar"', 'class="statusbar" style="color:#eef1f7;background:#1f2533"');
const aSelect = android(`<div class="mbody">
  <div class="topbar">${ic("back")}<span>Messages</span></div>
  <div class="textblock" style="margin-top:12px">Your booking code is <mark>QF7 2M9 XT</mark>. Show it at the desk when you arrive.</div>
  <div style="margin-top:12px;display:flex;justify-content:center"><span class="seltool"><span>Copy</span><span class="ours">Send to my devices</span><span>Share</span><span>${ic("more")}</span></span></div>
  </div>`);

// =====================================================================
// Pages
// =====================================================================
page("01-welcome.html", "First run: Welcome", "Screen 1 of 3 on a computer. One sentence, one button. The same screen on each OS, with that OS's window frame.", [
  shot("Windows 11", "Window controls on the right, Segoe UI.", welcome("win")),
  shot("macOS", "Traffic lights on the left, SF.", welcome("mac")),
  shot("Linux (GNOME)", "Header bar with one round close button.", welcome("linux")),
]);
page("02-permissions.html", "First run: Permissions", "Screen 2 of 3. One row per permission. The OS prompt appears only after the user presses the row's button. A row turns into a green Done when it is finished.", [
  shot("Windows: one row done", "The firewall row is done (one UAC prompt). Start with Windows needs no prompt.", permWin("win", permWinRows)),
  shot("Windows: firewall refused", "The row says what still works and offers to try again.", permWin("win", permWinRefused)),
  shot("macOS: four rows", "Local Network, paste access (opens System Settings), login item, and optional notifications.", permWin("mac", permMacRows)),
  shot("Linux: supported desktop", "Only Start at login. Clipboard access needs no permission here.", permWin("linux", permLinuxRows, linuxNote)),
  shot("Linux: unsupported desktop", "The app says so plainly and stops.", linuxBlocked),
]);
page("03-add-device.html", "First run: Add a device", "Screen 3 of 3. Two choices on one screen. The same screen opens from Add a device in the settings window.", [
  shot("Windows", "", addWin("win")),
  shot("macOS", "", addWin("mac")),
]);
page("04-show-code.html", "Show a code", "The code is a ticket: one number, two matching halves. The QR code sits on the tear-off stub. Codes end after 10 minutes and are stamped, never silently removed.", [
  shot("Short code and QR code", "Same network only. Phones scan the stub.", showShort("mac")),
  shot("Long code", "After pressing the link at the bottom. Works anywhere, also through a VPN.", showLong("mac")),
  shot("Someone else tried the code", "A failed try the user did not make.", showTried("win")),
  shot("Code ended", "After 10 minutes, or after 5 wrong tries on the other side.", showExpired("linux")),
]);
page("05-enter-code.html", "Enter a code", "One field takes both the short code and the long code. Each problem has one sentence and one way out.", [
  shot("Typing", "Digits group themselves in threes as you type.", enterEmpty("win")),
  shot("Looking", "Searching the local network, up to 20 seconds.", enterLooking("win")),
  shot("Wrong code", "", enterWrong("win")),
  shot("Too many tries", "After 5 tries the code stops working.", enterLocked("mac")),
  shot("Other device not found", "Short code, nothing found in 20 seconds.", enterNotFound("mac")),
  shot("Already in another group", "Groups never merge. The user must choose.", enterJoin("linux")),
]);
page("06-success.html", "Connected", "Both devices show the matched halves of the same ticket. Then the window can close and the app disappears.", [
  shot("Linux laptop, joined a Mac", "", success("linux", "Kitchen laptop", "laptop", "Max's Mac mini", "desktop")),
  shot("Windows PC, joined a Mac", "", success("win", "Max's PC", "desktop", "Max's Mac mini", "desktop")),
]);
page("07-settings.html", "Settings window", "What opens when the user starts the app again. About 420 x 560. The coloured band says the state and the fact that proves it. Device marks use one fill grammar: solid is connected, half is through the internet, an outline is asleep, stripes are blocked.", [
  shot("macOS: working", "The usual state. Nothing to do.", settingsMain("mac")),
  shot("Windows: working", "The same window on Windows.", settingsMain("win", DEV_WIN)),
  shot("Linux: working", "", settingsMain("linux", DEV_LINUX)),
  shot("Settings and Help, expanded", "Scrolled down. Never shows clipboard content.", settingsExpanded("mac")),
  shot("No other device is on", "Not an error. Calm sky band.", win("win", settingsBody({ band: bandAlone, list: DEV_WIN.map((d, i) => i ? { ...d, mark: "away", state: "Asleep or away" } : d) }), { flush: true, cls: "scroll-hint" })),
]);
page("08-pause-remove.html", "Pause, rename and remove", "Pause has two choices. Removing a device asks once, then stamps the row so nothing disappears without a trace.", [
  shot("Pause menu", "Turning on Pause sync opens two choices.", pauseMenu("mac")),
  shot("Paused", "Butter band with a Resume button.", pausedState("mac")),
  shot("Device menu", "Rename and Remove.", rowMenu("win")),
  shot("Remove a device", "", removeConfirm("win")),
  shot("Removed", "The row is stamped, then fades on the next start.", removedRow("mac")),
  shot("Remove this device and forget everything", "The clean way to leave, on every OS.", forgetConfirm("linux")),
]);
page("09-attention.html", "Needs attention", "No tray icon, so problems show in the settings window: one sentence and one Fix button. The OS notifies only when the user must act, at most once a day.", [
  shot("No path between devices", "LAN blocked and the internet path is off.", attnNoPath("win")),
  shot("Help: VPN blocks the network", "Opened from the Why link.", helpVpn("win")),
  shot("macOS blocks the network", "Local Network was refused.", attnSimple("mac", "macOS blocks this app from your network.", "Open Settings")),
  shot("macOS paste access", "Paste access set to Ask or Deny.", attnSimple("mac", "macOS asks before this app can read what you copy.", "Open Settings")),
  shot("Wrong clock", "", attnSimple("linux", "The date and time on this device are wrong, so its copies are ignored.", "Fix time", DEV_LINUX)),
  shot("Removed by another device", "", removedHere("win")),
  shot("Background service will not start", "After two failed starts.", attnSimple("win", "The background service cannot start.", "Copy report", DEV_WIN)),
  shot("Clip too large", "Shown in Last activity only. Not an error.", win("mac", settingsBody({ band: bandWorking, activity: `<div class="activity"><span class="aic">${ic("image")}</span><span>Image was too large to sync (limit 25 MB) <time>· 5 minutes ago</time></span></div>` }), { flush: true, cls: "scroll-hint" })),
  shot("OS notification (Windows)", "Only when the user must act. At most one a day.", toastWin),
]);
page("10-iphone-setup.html", "iPhone: setup", "Native iOS layout: large titles, inset grouped lists, SF. The ticket appears only to show what the code looks like.", [
  shot("Welcome", "States the one-tap rule up front.", iWelcome),
  shot("Permissions", "Camera is asked only when scanning.", iPerms),
  shot("Add this iPhone", "Scan first, type as the fallback.", iPair),
  shot("Scanning", "", iScan),
  shot("Type a code", "", iType),
  shot("Connected", "", iDone),
  shot("Make it one tap", "Two Shortcuts, then where to put them. All of it can be skipped.", iOneTap),
  shot("Skip the paste question", "Optional last page.", iPasteAllow),
]);
page("11-iphone-ipad-daily.html", "iPhone and iPad: daily use", "Opening the app gets the newest clip and copies it here. The Paste button sends what this device copied. The Share sheet and Shortcuts do the same without opening the app.", [
  shot("Open the app: clip received", "The app fetched and copied the newest clip on open.", iHome()),
  shot("After pressing Paste", "A banner confirms. Never shows the content.", iSent),
  shot("Share sheet", "Send to my devices from any app.", iShare),
  shot("No computer is on", "Never a silent empty result.", iNoComputer),
  shot("iPad", "Sidebar layout. One-tap options only list what iPad has: Control Center and Home Screen.", ipadHome),
]);
page("12-android-setup.html", "Android: setup", "Material 3 layout: top app bar, pill buttons, Roboto. Same ticket, same words.", [
  shot("Welcome", "", aWelcome),
  shot("Permissions", "Notifications and Nearby devices. No camera.", aPerms),
  shot("Nearby devices refused", "Says what still works and how to fix it.", aPermRefused),
  shot("Add this phone", "", aPair),
  shot("Connected", "", aDone),
  shot("Make it one tap", "Quick Settings tile and the text menu entry.", aOneTap),
  shot("Battery note", "Opens the right settings page.", aBattery),
]);
page("13-android-daily.html", "Android: daily use", "Computer to phone is automatic while the phone is awake. Phone to computer is one tap: the Quick Settings tile, the text menu, the Share sheet, or the button in the app.", [
  shot("App open", "The extended button is the one main action.", aHome()),
  shot("After Send clipboard", "Snackbar confirms.", aSent),
  shot("Notification shade", "The tile, the required notice, and the Tap to copy fallback.", aShade),
  shot("Text selection menu", "Send to my devices sits beside Copy.", aSelect),
]);

// Index
const pages = [
  ["Computers", [
    ["01-welcome.html", "Welcome", "Screen 1 of 3, on Windows, macOS and Linux."],
    ["02-permissions.html", "Permissions", "Per OS, with refused and unsupported states."],
    ["03-add-device.html", "Add a device", "First device, or I have a code."],
    ["04-show-code.html", "Show a code", "Short code, QR, long code, ended, someone tried."],
    ["05-enter-code.html", "Enter a code", "Typing, looking, wrong, locked, not found, other group."],
    ["06-success.html", "Connected", "Matched ticket halves."],
    ["07-settings.html", "Settings window", "Working on each OS, expanded settings, nobody on."],
    ["08-pause-remove.html", "Pause, rename, remove", "Menus, dialogs and the stamped row."],
    ["09-attention.html", "Needs attention", "Every problem state, help page, OS notification."],
  ]],
  ["Phones and tablets", [
    ["10-iphone-setup.html", "iPhone setup", "Welcome to Make it one tap."],
    ["11-iphone-ipad-daily.html", "iPhone and iPad daily use", "Open, Paste, Share sheet, no computer on, iPad."],
    ["12-android-setup.html", "Android setup", "Permissions, pairing, tile, battery."],
    ["13-android-daily.html", "Android daily use", "App, snackbar, shade, text menu."],
  ]],
];
writeFileSync(join(OUT, "index.html"), `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Universal Clipboard mockups</title>
<link rel="stylesheet" href="mockups.css">
</head>
<body>
<header class="board-head">
  ${appmark("appmark-lg")}
  <div>
    <h1>Universal Clipboard: screen mockups</h1>
    <p>Static mockups for approval (milestone M8a). Made 2026-11-13 from section 2 of the spec. Open any page; nothing needs a server or a build.</p>
  </div>
</header>
<section class="board-head" style="padding-top:18px;display:block">
  <p style="max-width:80ch"><b>Visual idea: the cloakroom ticket.</b> Pairing works like a cloakroom: one number, two matching halves. The code is shown as a butter-yellow ticket with red numerals and a perforated stub that holds the QR code. After pairing, both devices show the matched halves. Everything else stays calm and plain: a pale ground, navy text and buttons, and one coloured band that says the state. Each OS keeps its own window frame, fonts and controls.</p>
  <div class="swatches">
    <span class="swatch"><i style="background:#f2f4f1"></i>Ground</span>
    <span class="swatch"><i style="background:#22346b"></i>Ink navy</span>
    <span class="swatch"><i style="background:#f7e6a0"></i>Ticket stock</span>
    <span class="swatch"><i style="background:#ad2b21"></i>Ticket red (codes only)</span>
    <span class="swatch"><i style="background:#cfeadb"></i>Working</span>
    <span class="swatch"><i style="background:#f7c9b9"></i>Needs attention</span>
    <span class="swatch"><i style="background:#d3e3f4"></i>Nobody on, info</span>
  </div>
  <p class="small" style="margin-top:10px">The app icon is a placeholder. Codes, names and times are examples. The QR codes are drawn patterns, not real codes.</p>
</section>
${pages.map(([h, list]) => `<h2 class="index-h">${h}</h2><div class="index-grid">${list.map(([f, t, d]) => `<a class="index-card" href="${f}"><b>${t}</b><span>${d}</span><em>${f}</em></a>`).join("")}</div>`).join("\n")}
</body>
</html>
`);
console.log("ok");
