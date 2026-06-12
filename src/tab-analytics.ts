import { invoke } from "@tauri-apps/api/core";

import { $, fmt, modelLabel } from "./utils";
import type { DayBucket, OutcomeReport } from "./types";

const MODEL_COLORS: Record<string, string> = {
  fable: "var(--fable)",
  opus: "var(--accent)",
  sonnet: "var(--neutral)",
  haiku: "var(--ok)",
};
const EXTRA_COLORS = ["#9b59b6", "#e67e22", "#1abc9c", "#e74c3c"];

export function modelColor(model: string): string {
  for (const [key, color] of Object.entries(MODEL_COLORS)) {
    if (model.includes(key)) return color;
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

  const W = 356, PAD_L = 8, PAD_R = 8, PAD_TOP = 8, PAD_B = 16;
  const CHART_W = W - PAD_L - PAD_R;
  const CHART_H = 72;
  const H = CHART_H + PAD_TOP + PAD_B;
  const n = buckets.length;

  const modelSet = new Set<string>();
  for (const b of buckets)
    for (const m of b.by_model)
      if (m.model && !m.model.startsWith("<")) modelSet.add(m.model);

  const MODEL_ORDER = ["haiku", "sonnet", "opus", "fable"];
  const models = Array.from(modelSet).sort((a, b) => {
    const ai = MODEL_ORDER.findIndex((k) => a.includes(k));
    const bi = MODEL_ORDER.findIndex((k) => b.includes(k));
    if (ai !== bi) return (ai === -1 ? 99 : ai) - (bi === -1 ? 99 : bi);
    return a.localeCompare(b);
  });

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
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("width", "100%");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);

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

  for (let i = 0; i < buckets.length; i++) {
    const text = document.createElementNS(svgNS, "text");
    text.setAttribute("x", xOf(i).toFixed(1));
    text.setAttribute("y", (H - 3).toFixed(1));
    text.setAttribute("text-anchor", "middle");
    text.setAttribute("font-size", "9");
    text.setAttribute("fill", buckets[i].is_today ? "var(--accent)" : "var(--muted)");
    text.setAttribute("font-weight", buckets[i].is_today ? "600" : "normal");
    text.textContent = buckets[i].label;
    svg.appendChild(text);
  }

  container.appendChild(svg);

  if (models.length > 1) {
    const legend = document.createElement("div");
    legend.className = "chart-legend";
    for (const m of models) {
      const item = document.createElement("span");
      item.className = "chart-legend-item";
      const dot = document.createElement("span");
      dot.className = "chart-legend-dot";
      dot.style.background = modelColor(m);
      const lbl = document.createElement("span");
      lbl.textContent = modelLabel(m);
      item.appendChild(dot);
      item.appendChild(lbl);
      legend.appendChild(item);
    }
    container.appendChild(legend);
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

  const W = 356, PAD_L = 8, PAD_R = 8, PAD_TOP = 8, PAD_B = 16;
  const CHART_W = W - PAD_L - PAD_R;
  const CHART_H = 60;
  const H = CHART_H + PAD_TOP + PAD_B;
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
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("width", "100%");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);

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

    const text = document.createElementNS(svgNS, "text");
    text.setAttribute("x", cx.toFixed(1));
    text.setAttribute("y", (H - 3).toFixed(1));
    text.setAttribute("text-anchor", "middle");
    text.setAttribute("font-size", "9");
    text.setAttribute("fill", b.is_today ? "var(--accent)" : "var(--muted)");
    text.setAttribute("font-weight", b.is_today ? "600" : "normal");
    text.textContent = b.label;
    svg.appendChild(text);
  }

  container.appendChild(svg);

  // Legend
  const legend = document.createElement("div");
  legend.className = "chart-legend";
  for (const [label, color] of [
    ["Output", "var(--accent)"],
    ["Cache write", "var(--ok)"],
    ["Input", "var(--neutral)"],
  ] as [string, string][]) {
    const item = document.createElement("span");
    item.className = "chart-legend-item";
    const dot = document.createElement("span");
    dot.className = "chart-legend-dot";
    dot.style.background = color;
    const lbl = document.createElement("span");
    lbl.textContent = label;
    item.appendChild(dot);
    item.appendChild(lbl);
    legend.appendChild(item);
  }
  container.appendChild(legend);
}

// --- Outcomes: fate of each session's edits, classified backend-side ---

const OUTCOME_META: Record<string, { label: string; color: string; note: string; tip: string }> = {
  shipped: {
    label: "Shipped",
    color: "var(--ok)",
    note: "→ main",
    tip: "Edits landed in commits on the main branch",
  },
  pending: {
    label: "Pending",
    color: "var(--tier-2)",
    note: "on branches",
    tip: "Edits committed on a branch not merged yet",
  },
  reverted: {
    label: "Reverted",
    color: "var(--tier-3)",
    note: "undone",
    tip: "Edits whose commits were later reverted",
  },
  abandoned: {
    label: "Abandoned",
    color: "var(--muted)",
    note: "no commit",
    tip: "Edits never committed",
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
    pct.textContent = c.percent > 0 && c.percent < 0.5 ? "<1%" : `${Math.round(c.percent)}%`;

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

/** Lazy fetch on section open; the backend caches the git work (5 min TTL). */
export async function loadOutcomes() {
  const container = $<HTMLDivElement>("outcomes-container");
  if (!container.hasChildNodes()) {
    const computing = document.createElement("div");
    computing.className = "empty";
    computing.textContent = "Computing…";
    container.appendChild(computing);
  }
  renderOutcomes(await invoke<OutcomeReport>("get_outcomes"));
}

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

  const W = 356, PAD_L = 8, PAD_R = 8, PAD_TOP = 8, PAD_B = 16;
  const CHART_W = W - PAD_L - PAD_R;
  const CHART_H = 60;
  const H = CHART_H + PAD_TOP + PAD_B;
  const n = buckets.length;
  const colW = CHART_W / n;
  const GAP = 3;
  const barW = colW - GAP * 2;

  const maxVal = Math.max(...buckets.map((b) => b.cost_usd), 0.000001);
  const colCx = (i: number) => PAD_L + (i + 0.5) * colW;

  const svgNS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("width", "100%");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);

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

    const text = document.createElementNS(svgNS, "text");
    text.setAttribute("x", cx.toFixed(1));
    text.setAttribute("y", (H - 3).toFixed(1));
    text.setAttribute("text-anchor", "middle");
    text.setAttribute("font-size", "9");
    text.setAttribute("fill", b.is_today ? "var(--accent)" : "var(--muted)");
    text.setAttribute("font-weight", b.is_today ? "600" : "normal");
    text.textContent = b.label;
    svg.appendChild(text);
  }

  container.appendChild(svg);
}
