import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { $, fmt, modelLabel } from "./utils";
import type { DayBucket, MemoryFile } from "./types";

const MODEL_COLORS: Record<string, string> = {
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

  const MODEL_ORDER = ["haiku", "sonnet", "opus"];
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

export async function loadMemory() {
  const body = $<HTMLDivElement>("memory-body");
  const files = await invoke<MemoryFile[]>("get_memory");
  body.innerHTML = "";
  if (files.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No memory for the active sessions";
    body.appendChild(empty);
    return;
  }
  for (const f of files) {
    const item = document.createElement("div");
    item.className = "mem-file";
    const head = document.createElement("button");
    head.className = "mem-head";
    const nameSpan = document.createElement("span");
    nameSpan.textContent = f.name;
    const openSpan = document.createElement("span");
    openSpan.className = "open";
    openSpan.title = "Open";
    openSpan.textContent = "↗";
    head.appendChild(nameSpan);
    head.appendChild(openSpan);
    const pre = document.createElement("pre");
    pre.className = "mem-content hidden";
    pre.textContent = f.content;
    head.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).classList.contains("open")) {
        openPath(f.path).catch(() => {});
        return;
      }
      pre.classList.toggle("hidden");
    });
    item.appendChild(head);
    item.appendChild(pre);
    body.appendChild(item);
  }
}
