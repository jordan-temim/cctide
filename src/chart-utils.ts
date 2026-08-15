// Shared SVG chart helpers for tab-analytics.ts. All three weekly charts
// (tokens-by-model line chart, breakdown stacked bars, cost bars) share the
// same viewport size, padding, x-axis day labels, and legend markup — this
// module factors that out so each renderer only owns its chart-specific draw
// logic (lines vs stacked bars vs plain bars).

export const SVG_NS = "http://www.w3.org/2000/svg";

// Shared layout constants (identical across all weekly charts).
export const W = 356;
export const PAD_L = 8;
export const PAD_R = 8;
export const PAD_TOP = 8;
export const PAD_B = 16;
export const CHART_W = W - PAD_L - PAD_R;

/** Creates the `<svg>` element shared by every chart, given the chart's own
 * plot height (excludes top/bottom padding). Returns the element plus the
 * computed total height `H`, needed by callers for label placement. */
export function createChartSvg(chartH: number): { svg: SVGSVGElement; H: number } {
  const H = chartH + PAD_TOP + PAD_B;
  const svg = document.createElementNS(SVG_NS, "svg") as SVGSVGElement;
  svg.setAttribute("width", "100%");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  return { svg, H };
}

export interface DayLabel {
  x: number;
  text: string;
  isToday: boolean;
}

/** Draws the x-axis day labels shared by every chart (today highlighted). */
export function drawDayLabels(svg: SVGSVGElement, H: number, labels: DayLabel[]) {
  for (const l of labels) {
    const text = document.createElementNS(SVG_NS, "text");
    text.setAttribute("x", l.x.toFixed(1));
    text.setAttribute("y", (H - 3).toFixed(1));
    text.setAttribute("text-anchor", "middle");
    text.setAttribute("font-size", "9");
    text.setAttribute("fill", l.isToday ? "var(--accent)" : "var(--muted)");
    text.setAttribute("font-weight", l.isToday ? "600" : "normal");
    text.textContent = l.text;
    svg.appendChild(text);
  }
}

export interface LegendItem {
  color: string;
  label: string;
}

/** Builds a dot + label legend row and appends it to `container`. */
export function buildLegend(container: HTMLElement, items: LegendItem[]): HTMLDivElement {
  const legend = document.createElement("div");
  legend.className = "chart-legend";
  for (const { color, label } of items) {
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
  return legend;
}

/** Sorts model ids by family priority (as given by `order`, matched by
 * substring), falling back to alphabetical within/after unknown families. */
export function sortModelsByFamily(models: string[], order: readonly string[]): string[] {
  return [...models].sort((a, b) => {
    const ai = order.findIndex((k) => a.includes(k));
    const bi = order.findIndex((k) => b.includes(k));
    if (ai !== bi) return (ai === -1 ? 99 : ai) - (bi === -1 ? 99 : bi);
    return a.localeCompare(b);
  });
}
