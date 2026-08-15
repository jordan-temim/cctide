import { invoke } from "@tauri-apps/api/core";

import { $, fmt, modelLabel, formatOutcomePercent, MODEL_FAMILIES, type CollapsibleSpec } from "./utils";
import { createProjectFilter, type ProjectFilter } from "./project-filter";
import {
  PAD_L,
  PAD_TOP,
  CHART_W,
  createChartSvg,
  drawDayLabels,
  buildLegend,
  sortModelsByFamily,
  type DayLabel,
} from "./chart-utils";
import type { DayBucket, OutcomeReport } from "./types";

const MODEL_COLORS: Record<string, string> = {
  fable: "var(--fable)",
  opus: "var(--accent)",
  sonnet: "var(--neutral)",
  haiku: "var(--ok)",
};
const EXTRA_COLORS = ["#9b59b6", "#e67e22", "#1abc9c", "#e74c3c"];

export function modelColor(model: string): string {
  for (const key of MODEL_FAMILIES) {
    if (model.includes(key)) return MODEL_COLORS[key];
  }
  let h = 0;
  for (const c of model) h = (h * 31 + c.charCodeAt(0)) & 0xffff;
  return EXTRA_COLORS[h % EXTRA_COLORS.length];
}

export function renderChart(buckets: DayBucket[]) {
  const container = $<HTMLDivElement>("chart-container");
  container.innerHTML = "";
  if (buckets.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "Set a weekly reset date to see activity";
    container.appendChild(empty);
    return;
  }

  const CHART_H = 72;
  const n = buckets.length;

  const modelSet = new Set<string>();
  for (const b of buckets)
    for (const m of b.by_model)
      if (m.model && !m.model.startsWith("<")) modelSet.add(m.model);

  // Display order: least to most "premium" — the reverse of MODEL_FAMILIES,
  // which is ordered for the substring-match scan (modelColor, modelShort).
  const MODEL_ORDER = [...MODEL_FAMILIES].reverse();
  const models = sortModelsByFamily(Array.from(modelSet), MODEL_ORDER);

  const series = new Map<string, number[]>();
  for (const m of models) {
    series.set(m, buckets.map((b) => {
      const e = b.by_model.find((x) => x.model === m);
      return e ? e.weighted : 0;
    }));
  }

  const maxVal = Math.max(...Array.from(series.values()).flat(), 1);
  const xOf = (i: number) => n <= 1 ? PAD_L + CHART_W / 2 : PAD_L + (i / (n - 1)) * CHART_W;
  const yOf = (v: number) => PAD_TOP + CHART_H - (v / maxVal) * CHART_H;

  const svgNS = "http://www.w3.org/2000/svg";
  const { svg, H } = createChartSvg(CHART_H);

  const todayIdx = buckets.findIndex((b) => b.is_today);
  if (todayIdx >= 0) {
    const tx = xOf(todayIdx);
    const guide = document.createElementNS(svgNS, "line");
    guide.setAttribute("x1", tx.toFixed(1));
    guide.setAttribute("y1", PAD_TOP.toString());
    guide.setAttribute("x2", tx.toFixed(1));
    guide.setAttribute("y2", (PAD_TOP + CHART_H).toFixed(1));
    guide.setAttribute("stroke", "var(--accent)");
    guide.setAttribute("stroke-width", "1");
    guide.setAttribute("stroke-opacity", "0.2");
    svg.appendChild(guide);
  }

  for (const [model, values] of series) {
    const color = modelColor(model);

    const pts = values.map((v, i) => `${xOf(i).toFixed(1)},${yOf(v).toFixed(1)}`).join(" ");
    const polyline = document.createElementNS(svgNS, "polyline");
    polyline.setAttribute("points", pts);
    polyline.setAttribute("fill", "none");
    polyline.setAttribute("stroke", color);
    polyline.setAttribute("stroke-width", "2");
    polyline.setAttribute("stroke-linejoin", "round");
    polyline.setAttribute("stroke-linecap", "round");
    svg.appendChild(polyline);

    for (let i = 0; i < values.length; i++) {
      const circle = document.createElementNS(svgNS, "circle");
      circle.setAttribute("cx", xOf(i).toFixed(1));
      circle.setAttribute("cy", yOf(values[i]).toFixed(1));
      circle.setAttribute("r", "3");
      circle.setAttribute("fill", color);
      circle.setAttribute("opacity", values[i] > 0 ? "1" : "0.25");
      const title = document.createElementNS(svgNS, "title");
      title.textContent = `${modelLabel(model)} — ${buckets[i].label}: ${fmt(values[i])} tokens`;
      circle.appendChild(title);
      svg.appendChild(circle);
    }
  }

  const dayLabels: DayLabel[] = buckets.map((b, i) => ({
    x: xOf(i),
    text: b.label,
    isToday: b.is_today,
  }));
  drawDayLabels(svg, H, dayLabels);

  container.appendChild(svg);

  if (models.length > 0) {
    buildLegend(
      container,
      models.map((m) => ({ color: modelColor(m), label: modelLabel(m) })),
    );
  }
}


export function renderBreakdownChart(buckets: DayBucket[]) {
  const container = $<HTMLDivElement>("breakdown-chart-container");
  container.innerHTML = "";

  const hasData = buckets.some(
    (b) => b.breakdown.input + b.breakdown.output + b.breakdown.cache_write > 0
  );
  if (buckets.length === 0 || !hasData) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "Set a weekly reset date to see activity";
    container.appendChild(empty);
    return;
  }

  const CHART_H = 60;
  const n = buckets.length;
  const colW = CHART_W / n;
  const GAP = 3;
  const barW = colW - GAP * 2;

  const maxVal = Math.max(
    ...buckets.map((b) => b.breakdown.input + b.breakdown.output + b.breakdown.cache_write),
    1
  );
  const colCx = (i: number) => PAD_L + (i + 0.5) * colW;

  const svgNS = "http://www.w3.org/2000/svg";
  const { svg, H } = createChartSvg(CHART_H);

  for (let i = 0; i < buckets.length; i++) {
    const b = buckets[i];
    const { input, output, cache_write } = b.breakdown;
    const total = input + output + cache_write;
    const cx = colCx(i);
    const x = (cx - barW / 2).toFixed(1);
    const base = PAD_TOP + CHART_H;

    // Stacked from bottom: output (accent), cache_write (ok/green), input (neutral/blue)
    const layers: [number, string][] = [
      [output, "var(--accent)"],
      [cache_write, "var(--ok)"],
      [input, "var(--neutral)"],
    ];
    const activeLayers = layers.filter(([v]) => v > 0);
    let yTop = base;
    for (let li = 0; li < activeLayers.length; li++) {
      const [val, color] = activeLayers[li];
      const h = (val / maxVal) * CHART_H;
      const gap = li < activeLayers.length - 1 ? 1 : 0;
      yTop -= h;
      const rect = document.createElementNS(svgNS, "rect");
      rect.setAttribute("x", x);
      rect.setAttribute("y", yTop.toFixed(1));
      rect.setAttribute("width", barW.toFixed(1));
      rect.setAttribute("height", Math.max(h - gap, 1).toFixed(1));
      rect.setAttribute("rx", "2");
      rect.setAttribute("fill", color);
      svg.appendChild(rect);
    }

    if (total > 0) {
      const title = document.createElementNS(svgNS, "title");
      title.textContent = `${b.label}: output ${fmt(output)} · cache ${fmt(cache_write)} · input ${fmt(input)}`;
      // attach to a transparent full-height rect for hover
      const hit = document.createElementNS(svgNS, "rect");
      hit.setAttribute("x", x);
      hit.setAttribute("y", (PAD_TOP + CHART_H - (total / maxVal) * CHART_H).toFixed(1));
      hit.setAttribute("width", barW.toFixed(1));
      hit.setAttribute("height", ((total / maxVal) * CHART_H).toFixed(1));
      hit.setAttribute("fill", "transparent");
      hit.appendChild(title);
      svg.appendChild(hit);
    }

  }

  drawDayLabels(
    svg, H,
    buckets.map((b, i) => ({ x: colCx(i), text: b.label, isToday: b.is_today })),
  );

  container.appendChild(svg);

  buildLegend(container, [
    { label: "Output", color: "var(--accent)" },
    { label: "Cache write", color: "var(--ok)" },
    { label: "Input", color: "var(--neutral)" },
  ]);
}

// --- Outcomes: fate of each session's edits, classified backend-side ---

const OUTCOME_META: Record<string, { label: string; color: string; note: string; tip: string }> = {
  shipped: {
    label: "Shipped",
    color: "var(--ok)",
    note: "→ main",
    tip: "Edits landed in commits on the main branch",
  },
  on_branch: {
    label: "On a branch",
    color: "var(--tier-2)",
    note: "not on main",
    tip: "Edits committed on a branch not merged into main yet",
  },
  reverted: {
    label: "Reverted",
    color: "var(--tier-3)",
    note: "undone",
    tip: "Edits whose commits were later reverted",
  },
  uncommitted: {
    label: "Uncommitted",
    color: "var(--muted)",
    note: "not committed",
    tip: "Edits not carried by any commit — typically still in the working tree",
  },
  non_repo: {
    label: "Non-repo",
    color: "var(--track)",
    note: "chat, docs, no git",
    tip: "Sessions outside any git repository",
  },
};

export function renderOutcomes(report: OutcomeReport) {
  const container = $<HTMLDivElement>("outcomes-container");
  container.innerHTML = "";

  const active = report.categories.filter((c) => c.weighted > 0);
  if (active.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No activity in this window";
    container.appendChild(empty);
    return;
  }

  const bar = document.createElement("div");
  bar.className = "outcomes-bar";
  for (const c of active) {
    const meta = OUTCOME_META[c.kind];
    if (!meta) continue;
    const seg = document.createElement("div");
    seg.className = "outcomes-seg";
    seg.style.width = `${c.percent}%`;
    seg.style.background = meta.color;
    seg.title = `${meta.label} — ${meta.tip}`;
    bar.appendChild(seg);
  }
  container.appendChild(bar);

  for (const c of active) {
    const meta = OUTCOME_META[c.kind];
    if (!meta) continue;
    const row = document.createElement("div");
    row.className = "outcome-row";
    row.title = `${meta.label} — ${meta.tip}`;

    const dot = document.createElement("span");
    dot.className = "outcome-dot";
    dot.style.background = meta.color;

    const label = document.createElement("span");
    label.className = "outcome-label";
    label.textContent = meta.label;

    const pct = document.createElement("span");
    pct.className = "outcome-pct";
    pct.textContent = formatOutcomePercent(c.percent);

    const note = document.createElement("span");
    note.className = "outcome-note";
    const n = c.session_count;
    note.textContent =
      c.kind === "non_repo"
        ? `${n} session${n > 1 ? "s" : ""} · ${meta.note}`
        : `${n} session${n > 1 ? "s" : ""} ${meta.note}`;

    row.append(dot, label, pct, note);
    container.appendChild(row);
  }
}

// ---------------------------------------------------------------------------
// Project filter dropdown — Analytics instance of the shared component
// ---------------------------------------------------------------------------

let _filter: ProjectFilter | null = null;

export function getProjectFilter(): string | null {
  return _filter?.getValue() ?? null;
}

export function updateProjectFilter(projects: string[]) {
  _filter?.update(projects);
}

export function setupProjectFilter(onChange: () => void) {
  _filter = createProjectFilter(
    {
      bar: "analytics-filter-bar",
      btn: "project-filter-btn",
      menu: "project-filter-menu",
      value: "project-filter-value",
    },
    onChange,
  );
}

/** Lazy fetch on section open; the backend caches the git work (5 min TTL). */
export async function loadOutcomes() {
  const container = $<HTMLDivElement>("outcomes-container");
  if (!container.hasChildNodes()) {
    const computing = document.createElement("div");
    computing.className = "empty";
    computing.textContent = "Computing…";
    container.appendChild(computing);
  }
  renderOutcomes(
    await invoke<OutcomeReport>("get_outcomes", { projectFilter: getProjectFilter() }),
  );
}

/** Reloads Outcomes only if its collapsible section is currently open — the
 * section's own onOpen only fires on the collapse toggle, so anything else
 * that can change what Outcomes should show (switching to the Analytics tab,
 * changing the project filter) needs to re-check and reload explicitly. */
export function refreshOutcomesIfOpen() {
  if (!$<HTMLElement>("analytics-outcomes-body").classList.contains("hidden")) {
    void loadOutcomes();
  }
}

// Collapsible sections owned by this tab; aggregated by main.ts alongside
// other tabs' lists and wired via utils.ts's setupCollapsibles().
export const ANALYTICS_COLLAPSIBLES: CollapsibleSpec[] = [
  { toggleId: "analytics-models-toggle", bodyId: "analytics-models-body" },
  { toggleId: "analytics-breakdown-toggle", bodyId: "analytics-breakdown-body" },
  { toggleId: "analytics-cost-toggle", bodyId: "analytics-cost-body" },
  { toggleId: "analytics-outcomes-toggle", bodyId: "analytics-outcomes-body", onOpen: () => void loadOutcomes() },
];

export function renderCostChart(buckets: DayBucket[]) {
  const container = $<HTMLDivElement>("cost-chart-container");
  container.innerHTML = "";

  const weeklyTotal = buckets.reduce((s, b) => s + b.cost_usd, 0);
  $<HTMLSpanElement>("weekly-cost").textContent =
    weeklyTotal > 0 ? `$${weeklyTotal.toFixed(2)} this week` : "";

  if (buckets.length === 0 || weeklyTotal === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = weeklyTotal === 0 ? "No cost data for this week" : "Set a weekly reset date to see activity";
    container.appendChild(empty);
    return;
  }

  const CHART_H = 60;
  const n = buckets.length;
  const colW = CHART_W / n;
  const GAP = 3;
  const barW = colW - GAP * 2;

  const maxVal = Math.max(...buckets.map((b) => b.cost_usd), 0.000001);
  const colCx = (i: number) => PAD_L + (i + 0.5) * colW;

  const svgNS = "http://www.w3.org/2000/svg";
  const { svg, H } = createChartSvg(CHART_H);

  for (let i = 0; i < buckets.length; i++) {
    const b = buckets[i];
    const bh = (b.cost_usd / maxVal) * CHART_H;
    const cx = colCx(i);
    const x = cx - barW / 2;
    const y = PAD_TOP + CHART_H - bh;

    const rect = document.createElementNS(svgNS, "rect");
    rect.setAttribute("x", x.toFixed(1));
    rect.setAttribute("y", y.toFixed(1));
    rect.setAttribute("width", barW.toFixed(1));
    rect.setAttribute("height", Math.max(bh, 1).toFixed(1));
    rect.setAttribute("rx", "2");
    rect.setAttribute(
      "fill",
      b.is_today ? "var(--accent)" : b.cost_usd > 0 ? "var(--neutral)" : "var(--track)"
    );
    const title = document.createElementNS(svgNS, "title");
    title.textContent = `${b.label}: $${b.cost_usd.toFixed(2)}`;
    rect.appendChild(title);
    svg.appendChild(rect);
  }

  drawDayLabels(
    svg, H,
    buckets.map((b, i) => ({ x: colCx(i), text: b.label, isToday: b.is_today })),
  );

  container.appendChild(svg);
}
