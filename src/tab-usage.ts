import { $, fmt, hhmm, dateHhmm, shortCwd, modelShort, colorClass, tierClass, setSegmentedBar } from "./utils";
import type { SessionUsage, WeeklyUsage, SessionCtx, Config } from "./types";

export function renderUsage(
  session: SessionUsage,
  weekly: WeeklyUsage,
  sessions: SessionCtx[],
  cfg: Config,
) {
  const sessionWindow = session.window_start
    ? `started ${hhmm(session.window_start)} · resets ${hhmm(session.reset_at)}`
    : "no activity in the current window";
  setSegmentedBar(
    "session-bar",
    "session-sub",
    session.percent,
    `${fmt(session.weighted_tokens)} weighted tokens · ${sessionWindow}`,
    tierClass(session.percent, cfg.alert_levels),
  );

  const resetTxt = weekly.next_reset_at
    ? `resets ${dateHhmm(weekly.next_reset_at)}`
    : "reset not set";
  setSegmentedBar(
    "weekly-bar",
    "weekly-sub",
    weekly.percent,
    `${fmt(weekly.weighted_tokens)} tokens · ${resetTxt}`,
    tierClass(weekly.percent, cfg.alert_levels),
  );

  if (import.meta.env.DEV) {
    const fmtPct = (p: number | null) => (p != null ? `${p.toFixed(2)}%` : "—");
    $<HTMLSpanElement>("dbg-session").textContent = fmtPct(session.percent);
    $<HTMLSpanElement>("dbg-weekly").textContent = fmtPct(weekly.percent);
  }

  const list = $<HTMLDivElement>("sessions-list");
  list.innerHTML = "";
  if (sessions.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No active session";
    list.appendChild(empty);
    return;
  }
  for (const s of sessions) {
    const pct = s.percent ?? 0;
    const row = document.createElement("div");
    row.className = "session";

    const top = document.createElement("div");
    top.className = "session-top";
    const nameEl = document.createElement("span");
    nameEl.className = "session-name";
    nameEl.textContent = shortCwd(s.cwd);
    nameEl.title = s.cwd;
    const badgeEl = document.createElement("span");
    badgeEl.className = "badge";
    badgeEl.textContent = modelShort(s.model);
    top.appendChild(nameEl);
    top.appendChild(badgeEl);

    const barEl = document.createElement("div");
    barEl.className = "bar small";
    const fillEl = document.createElement("div");
    fillEl.className = `fill ${colorClass(pct)}`;
    fillEl.style.width = `${Math.min(100, pct)}%`;
    barEl.appendChild(fillEl);

    const subEl = document.createElement("div");
    subEl.className = "sub";
    subEl.textContent = `${fmt(s.context_tokens)} / ${fmt(s.context_limit)} ctx (${Math.min(100, pct).toFixed(0)}%)`;

    row.appendChild(top);
    row.appendChild(barEl);
    row.appendChild(subEl);
    list.appendChild(row);
  }
}
