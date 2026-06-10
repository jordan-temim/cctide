import { $, fmt, hhmm, dateHhmm, tierClass, setSegmentedBar } from "./utils";
import type { SessionUsage, WeeklyUsage, Config } from "./types";

export function renderUsage(session: SessionUsage, weekly: WeeklyUsage, cfg: Config) {
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

  const fmtPct = (p: number | null) => p != null ? ` ≈${p.toFixed(1)}%` : "";
  $<HTMLSpanElement>("session-pct").textContent = fmtPct(session.percent);
  $<HTMLSpanElement>("weekly-pct").textContent = fmtPct(weekly.percent);

  $<HTMLSpanElement>("session-eta-head").textContent = session.eta_secs ? `→ ETA ${hhmm(session.eta_secs)}` : "";
}
