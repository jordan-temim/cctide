// Generates the cctide icons with zero dependencies (pure Node + zlib).
//
// Concept: "CC" (Claude Code), where each C is a gauge — a C is just an arc, so
// we fill part of it (terracotta) with a radial tick at the fill level, the
// rest as a grey track. Left C ~50% (tick mid-C), right C ~30%. Evokes the two
// C's of Claude Code + the daily/weekly gauge idea.
//
// Two outputs, with DIFFERENT canvases on purpose:
//   _preview-color.png  square (app icon, fed to `tauri icon`)
//   _preview-mono.png   WIDE template — the C's fill the full height so they
//                       stay large in the macOS menu bar (which scales to the
//                       bar height; a square canvas wastes vertical space).
//
// Once approved:
//   cp _preview-color.png app-icon-master.png && cp _preview-mono.png tray-icon.png
//   npm run tauri icon src-tauri/icons/app-icon-master.png

import zlib from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ICONS = resolve(HERE, "../src-tauri/icons");

// ---- Palette ----
const BG = [30, 30, 31]; // dark rounded square (colour icon only)
const TRACK = [255, 255, 255]; // unfilled gauge track
const ACCENT = [217, 119, 87]; // #d97757 Claude terracotta

const TAU = Math.PI * 2;
const D2R = Math.PI / 180;
// Each C spans this arc (gap on the right = the C opening).
const A0 = 40 * D2R; // bottom-right tip
const A1 = 320 * D2R; // top-right tip
const SWEEP = A1 - A0;

function angDist(a, b) {
  const d = Math.abs(a - b) % TAU;
  return d > Math.PI ? TAU - d : d;
}

// One C-gauge in pixel space. Returns "fill" | "track" | "tick" | null.
// In `template` (monochrome) mode the fill/track distinction can't use colour,
// so it's encoded by stroke thickness: filled = thick, track = thin.
function cHit(px, py, c, R, T, template) {
  const dx = px - c.cx;
  const dy = py - c.cy;
  const dist = Math.hypot(dx, dy);
  let a = Math.atan2(dy, dx);
  if (a < 0) a += TAU;

  const tickAng = A0 + SWEEP * c.fill;
  if (dist >= R - T * 1.05 && dist <= R + T * 1.05 && angDist(a, tickAng) <= 0.05) {
    return "tick";
  }
  if (a >= A0 && a <= A1) {
    const t = (a - A0) / SWEEP;
    const isFill = t <= c.fill;
    const half = template && !isFill ? T * 0.22 : T / 2;
    if (Math.abs(dist - R) <= half) return isFill ? "fill" : "track";
  }
  return null;
}

function roundedRect(px, py, W, H) {
  const m = 0;
  const rad = 0.22 * W;
  const x0 = m,
    y0 = m,
    x1 = W - m,
    y1 = H - m;
  if (px < x0 || px > x1 || py < y0 || py > y1) return false;
  const nearLeft = px < x0 + rad;
  const nearRight = px > x1 - rad;
  const nearTop = py < y0 + rad;
  const nearBot = py > y1 - rad;
  if ((nearLeft || nearRight) && (nearTop || nearBot)) {
    const cx = nearLeft ? x0 + rad : x1 - rad;
    const cy = nearTop ? y0 + rad : y1 - rad;
    return Math.hypot(px - cx, py - cy) <= rad;
  }
  return true;
}

// cfg: { W, H, R, T, cs:[{cx,cy,fill}], template, bg }
function pixel(px, py, cfg) {
  let hit = null;
  for (const c of cfg.cs) {
    const h = cHit(px, py, c, cfg.R, cfg.T, cfg.template);
    if (h) {
      hit = h;
      break;
    }
  }
  if (cfg.template) return hit ? [0, 0, 0, 255] : [0, 0, 0, 0];
  if (hit === "fill" || hit === "tick") return [...ACCENT, 255];
  if (hit === "track") return [...TRACK, 255];
  if (cfg.bg && roundedRect(px, py, cfg.W, cfg.H)) return [...BG, 255];
  return [0, 0, 0, 0];
}

function render(cfg) {
  const { W, H } = cfg;
  const buf = Buffer.alloc(W * H * 4);
  const SS = 3;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      let r = 0,
        g = 0,
        b = 0,
        al = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const p = pixel(x + (sx + 0.5) / SS, y + (sy + 0.5) / SS, cfg);
          r += p[0] * p[3];
          g += p[1] * p[3];
          b += p[2] * p[3];
          al += p[3];
        }
      }
      const n = SS * SS;
      const o = (y * W + x) * 4;
      if (al > 0) {
        buf[o] = Math.round(r / al);
        buf[o + 1] = Math.round(g / al);
        buf[o + 2] = Math.round(b / al);
      }
      buf[o + 3] = Math.round(al / n);
    }
  }
  return buf;
}

// ---- Minimal PNG encoder ----
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}
function encodePng(rgba, W, H) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(W, 0);
  ihdr.writeUInt32BE(H, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const raw = Buffer.alloc(H * (W * 4 + 1));
  for (let y = 0; y < H; y++) {
    raw[y * (W * 4 + 1)] = 0;
    rgba.copy(raw, y * (W * 4 + 1) + 1, y * W * 4, (y + 1) * W * 4);
  }
  const idat = zlib.deflateSync(raw, { level: 9 });
  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", idat), chunk("IEND", Buffer.alloc(0))]);
}

// ---- Configs ----
// Colour app icon: square, comfortable margin, slight monogram overlap so the
// C's are noticeably bigger than before.
const COLOR = {
  W: 1024,
  H: 1024,
  R: 225,
  T: 90,
  bg: true,
  template: false,
  cs: [
    { cx: 335, cy: 512, fill: 0.5 },
    { cx: 689, cy: 512, fill: 0.3 },
  ],
};

// Mono menu-bar template: WIDE canvas, C's fill the full height -> large in the
// menu bar.
const MONO = {
  W: 440,
  H: 256,
  R: 92,
  T: 36,
  bg: false,
  template: true,
  cs: [
    { cx: 120, cy: 128, fill: 0.5 },
    { cx: 320, cy: 128, fill: 0.3 },
  ],
};

// ---- Run (preview only) ----
mkdirSync(ICONS, { recursive: true });
writeFileSync(resolve(ICONS, "_preview-color.png"), encodePng(render(COLOR), COLOR.W, COLOR.H));
writeFileSync(resolve(ICONS, "_preview-mono.png"), encodePng(render(MONO), MONO.W, MONO.H));
console.log("wrote _preview-color.png and _preview-mono.png");
