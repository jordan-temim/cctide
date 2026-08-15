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

// Canonical model family list, matched by substring against model ids.
// Order matters for the scan (first match wins) — also the source of truth
// for MODEL_COLORS' key order and the chart display order in tab-analytics.ts.
export const MODEL_FAMILIES = ["fable", "opus", "sonnet", "haiku"] as const;

export function modelShort(m: string | null): string {
  if (!m) return "?";
  for (const fam of MODEL_FAMILIES) {
    if (m.includes(fam)) return fam[0].toUpperCase() + fam.slice(1);
  }
  return m;
}

export function entrypointShort(ep: string): string {
  if (ep === "cli") return "CLI";
  if (ep === "claude-vscode") return "VSCode";
  return ep;
}

// Compact "last activity" label. `nowMs` is injectable for tests.
export function timeAgo(ts: number, nowMs = Date.now()): string {
  const secs = Math.max(0, Math.floor(nowMs / 1000) - ts);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} h ago`;
  return `${Math.floor(secs / 86400)} d ago`;
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

// Outcome-category percent label: sub-0.5% rounds to "0%" which reads as
// "nothing happened", so anything strictly between 0 and 0.5 shows "<1%".
export function formatOutcomePercent(percent: number): string {
  return percent > 0 && percent < 0.5 ? "<1%" : `${Math.round(percent)}%`;
}

export interface CollapsibleSpec {
  toggleId: string;
  bodyId: string;
  onOpen?: () => void;
}

// Wires a toggle button + collapsible body pair (chevron rotation, optional
// lazy-load callback fired only when opening). Each tab module owns its own
// list of `CollapsibleSpec`s; main.ts just aggregates and wires them all.
export function setupCollapse(toggleId: string, bodyId: string, onOpen?: () => void) {
  const toggle = $<HTMLButtonElement>(toggleId);
  const body = $<HTMLElement>(bodyId);
  toggle.addEventListener("click", () => {
    const opening = body.classList.contains("hidden");
    body.classList.toggle("hidden");
    toggle.querySelector(".chev")?.classList.toggle("open", opening);
    if (opening && onOpen) onOpen();
  });
}

export function setupCollapsibles(specs: CollapsibleSpec[]) {
  for (const s of specs) setupCollapse(s.toggleId, s.bodyId, s.onOpen);
}

export function clampInput(id: string) {
  const el = $<HTMLInputElement>(id);
  el.addEventListener("input", () => {
    const n = parseFloat(el.value);
    if (Number.isFinite(n) && n > 100) el.value = "100";
    if (Number.isFinite(n) && n < 0) el.value = "0";
  });
}

// The stored weekly reset is the *anchor* (often in the past). Returns the next
// upcoming reset as a `datetime-local` value (`YYYY-MM-DDTHH:MM`), mirroring the
// backend's rolling logic (step by 7 days until strictly in the future). Invalid
// input is returned unchanged. Re-running it on its own output is idempotent.
export function nextWeeklyReset(resetStr: string): string {
  const d = new Date(resetStr.length <= 10 ? `${resetStr}T00:00` : resetStr);
  if (isNaN(d.getTime())) return resetStr;
  const now = Date.now();
  while (d.getTime() > now) d.setDate(d.getDate() - 7);
  while (d.getTime() <= now) d.setDate(d.getDate() + 7);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}
