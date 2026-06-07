export const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Element #${id} not found`);
  return el as T;
};

export function fmt(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return Math.round(n).toString();
}

export function colorClass(pct: number): string {
  if (pct >= 90) return "danger";
  if (pct >= 70) return "warn";
  return "ok";
}

// Bar colour by alert level (neutral / green / orange / red) — matching the
// tray icon. Mirrors `level_for` in Rust: count of levels the % has reached.
export function tierClass(pct: number | null, levels: number[]): string {
  if (pct === null) return "tier-0";
  const n = levels.filter((l) => pct >= l).length;
  return `tier-${Math.min(3, n)}`;
}

export function shortCwd(cwd: string): string {
  const parts = cwd.split("/").filter(Boolean);
  return parts.length ? parts[parts.length - 1] : cwd;
}

export function hhmm(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function dateHhmm(ts: number | null): string {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const date = d.toLocaleDateString([], { month: "short", day: "numeric" });
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return `${date} ${time}`;
}

// Keeps the version (e.g. "opus-4-8"), unlike modelShort which collapses to family.
export function modelLabel(m: string): string {
  return m.replace(/^claude-/, "").replace(/-\d{8}$/, "");
}

export function modelShort(m: string | null): string {
  if (!m) return "?";
  if (m.includes("opus")) return "Opus";
  if (m.includes("sonnet")) return "Sonnet";
  if (m.includes("haiku")) return "Haiku";
  return m;
}

export function updateLastUpdated() {
  const el = $<HTMLSpanElement>("last-updated");
  const now = new Date();
  el.textContent = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export const SEGMENTS = 15;

export function setSegmentedBar(
  barId: string, subId: string,
  pct: number | null, sub: string, cls: string
) {
  const container = $<HTMLDivElement>(barId);
  const subEl = $<HTMLDivElement>(subId);

  container.innerHTML = "";
  const filled = pct === null ? 0 : Math.min(SEGMENTS, Math.ceil(pct / (100 / SEGMENTS)));
  for (let i = 0; i < SEGMENTS; i++) {
    const seg = document.createElement("div");
    seg.className = "bar-segment" + (i < filled ? " filled " + cls : "");
    container.appendChild(seg);
  }

  subEl.textContent = sub;
}

export function pct(id: string): number | null {
  const v = $<HTMLInputElement>(id).value.trim();
  if (v === "") return null;
  const n = parseFloat(v);
  if (!Number.isFinite(n)) return null;
  return Math.max(0, Math.min(100, n));
}

export function clampInput(id: string) {
  const el = $<HTMLInputElement>(id);
  el.addEventListener("input", () => {
    const n = parseFloat(el.value);
    if (Number.isFinite(n) && n > 100) el.value = "100";
    if (Number.isFinite(n) && n < 0) el.value = "0";
  });
}
