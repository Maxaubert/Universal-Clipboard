import { parse } from 'node-html-parser';
import fs from 'fs';
const dir = '../../docs/design/desktop/';
const load = (p) => {
  const f = fs.readdirSync(dir).find((x) => x.startsWith(p));
  return parse(fs.readFileSync(dir + f, 'utf8')).querySelector('x-dc').querySelector('div').querySelector('div');
};
const svgs = (c) => c.querySelectorAll('svg');
const out = {};
const L = load;
const l1 = svgs(L('L01'));
out.welcome = l1[l1.length - 1].outerHTML;
out.add = svgs(L('L03')).find((s) => s.getAttribute('width') === '374').outerHTML;
out.connected = svgs(L('L06')).find((s) => s.getAttribute('width') === '374').outerHTML;
out.attention = svgs(L('L11')).find((s) => s.getAttribute('width') === '374').outerHTML;
out.vpn = svgs(L('L12')).find((s) => s.getAttribute('width') === '300').outerHTML;
const tiles = L('L07')
  .querySelectorAll('button')
  .filter((b) => b.getAttribute('style').includes('aspect-ratio'));
const logo = (t) => t.querySelector('svg').innerHTML.trim();
out.logoWindows = logo(tiles[0]);
out.logoApple = logo(tiles[1]);
out.logoLinux = logo(tiles[2]);
out.logoAndroid = logo(tiles[5]);
out.plus = tiles[6].querySelector('svg').innerHTML;
const hdr = L('L07')
  .querySelectorAll('button')
  .filter((b) => b.getAttribute('aria-label'));
out.pause = hdr.find((b) => b.getAttribute('aria-label') === 'Pause').querySelector('svg').innerHTML;
out.gear = hdr.find((b) => b.getAttribute('aria-label') === 'Settings').querySelector('svg').innerHTML;
const p2 = svgs(L('L02')).filter((s) => s.getAttribute('width') === '52');
out.permStart = p2[0].innerHTML;
out.permFirewall = p2[1].innerHTML;
out.checkBig = svgs(L('L02')).find((s) => s.getAttribute('aria-label') === 'Allowed').innerHTML;
const b3 = L('L03')
  .querySelectorAll('button')
  .filter((b) => /code/.test(b.text));
out.iconShow = b3[0].querySelector('svg').innerHTML;
out.iconEnter = b3[1].querySelector('svg').innerHTML;
out.trio = L('L04').querySelector('div[aria-label^="Works"]').innerHTML;
out.pencil = L('L08')
  .querySelectorAll('button')
  .find((b) => b.text.includes('Rename'))
  .querySelector('svg').innerHTML;
const s9 = svgs(L('L09')).filter((s) => s.getAttribute('width') === '30' && s.getAttribute('stroke'));
out.iconPower = s9[0].innerHTML;
out.iconKey = s9[1].innerHTML;
out.lock = svgs(L('L09')).find((s) => s.getAttribute('width') === '14').innerHTML;
let s = '// Generated once from docs/design/desktop/*.dc.html. Static artwork only (no user data).\n';
for (const [k, v] of Object.entries(out)) s += `export const ${k} = ${JSON.stringify(v.replace(/\s+/g, ' ').replace(/> </g, '><'))};\n`;
fs.writeFileSync('src/lib/art.ts', s);
console.log(Object.entries(out).map(([k, v]) => k + ':' + v.length).join(' '));
