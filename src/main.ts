import { invoke } from "@tauri-apps/api/core";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";

// --- Types returned by the Rust backend ---
interface SessionUsage {
  window_start: number | null;
  reset_at: number | null;
  weighted_tokens: number;
  percent: number | null;
  calibrated: boolean;
}
interface WeeklyUsage {
  weighted_tokens: number;
  percent: number | null;
  reset_date: string | null;
  week_start: number | null;
  next_reset_at: number | null;
  calibrated: boolean;
}
interface SessionCtx {
  session_id: string;
  cwd: string;
  version: string;
  model: string | null;
  context_tokens: number;
  context_limit: number;
  percent: number | null;
}
interface MemoryFile {
  project: string;
  name: string;
  path: string;
  content: string;
}
interface RtkSavings {
  summary: {
    total_commands: number;
    total_input: number;
    total_output: number;
    total_saved: number;
    avg_savings_pct: number;
  };
  weekly: { week_start: string; saved_tokens: number; savings_pct: number }[];
}
interface ModelUsage {
  model: string;
  tokens: number;
}
interface UpdateInfo {
  version: string;
  notes: string | null;
  url: string;
}
interface PanelData {
  session: SessionUsage;
  weekly: WeeklyUsage;
  sessions: SessionCtx[];
  models: ModelUsage[];
  config: Config;
  update: UpdateInfo | null;
}
interface Calibration {
  percent: number;
  budget: number;
  calibrated_at: number;
}
interface Config {
  refresh_secs: number;
  weekly_reset_date: string | null;
  notifications_enabled: boolean;
  alert_levels: number[];
  tracking_enabled: boolean;
  session_calibration: Calibration | null;
  session_calibration_2: Calibration | null;
  weekly_calibration: Calibration | null;
  weekly_calibration_2: Calibration | null;
}

// --- Helpers ---
const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Element #${id} not found`);
  return el as T;
};

function fmt(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return Math.round(n).toString();
}

function colorClass(pct: number): string {
  if (pct >= 90) return "danger";
  if (pct >= 70) return "warn";
  return "ok";
}

// Bar colour by alert level (neutral / green / orange / red) — matching the
// tray icon. Mirrors `level_for` in Rust: count of levels the % has reached.
function tierClass(pct: number | null, levels: number[]): string {
  if (pct === null) return "tier-0";
  const n = levels.filter((l) => pct >= l).length;
  return `tier-${Math.min(3, n)}`;
}


function shortCwd(cwd: string): string {
  const parts = cwd.split("/").filter(Boolean);
  return parts.length ? parts[parts.length - 1] : cwd;
}

function hhmm(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function dateHhmm(ts: number | null): string {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const date = d.toLocaleDateString([], { month: "short", day: "numeric" });
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return `${date} ${time}`;
}

// Keeps the version (e.g. "opus-4-8"), unlike modelShort which collapses to family.
function modelLabel(m: string): string {
  return m.replace(/^claude-/, "").replace(/-\d{8}$/, "");
}

function modelShort(m: string | null): string {
  if (!m) return "?";
  if (m.includes("opus")) return "Opus";
  if (m.includes("sonnet")) return "Sonnet";
  if (m.includes("haiku")) return "Haiku";
  return m;
}

function updateLastUpdated() {
  const el = $<HTMLSpanElement>("last-updated");
  const now = new Date();
  el.textContent = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

const SEGMENTS = 15;

function setSegmentedBar(
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

// --- Rendering ---
// --- Update banner ---------------------------------------------------------
let currentUpdate: UpdateInfo | null = null;
let updateStaged = false; // set once install succeeds; button becomes "Restart now"

function renderUpdateBanner(update: UpdateInfo | null) {
  currentUpdate = update;
  const banner = $("update-banner");
  if (!update || updateStaged) {
    if (!updateStaged) banner.classList.add("hidden");
    return;
  }
  banner.classList.remove("hidden");
  $("update-text").textContent = `Update available: v${update.version}`;
}

function setupUpdate() {
  const link = $<HTMLAnchorElement>("update-changelog");
  link.addEventListener("click", (e) => {
    e.preventDefault();
    if (currentUpdate) openUrl(currentUpdate.url).catch(() => {});
  });

  const btn = $<HTMLButtonElement>("update-install");
  btn.addEventListener("click", async () => {
    if (updateStaged) {
      await invoke("restart_app");
      return;
    }
    btn.disabled = true;
    btn.textContent = "Installing…";
    try {
      await invoke("install_update");
      updateStaged = true;
      btn.disabled = false;
      btn.textContent = "Restart now";
      $("update-text").textContent = "Update ready";
    } catch (e) {
      btn.disabled = false;
      btn.textContent = "Install";
      $("update-text").textContent = `Update failed: ${e}`;
    }
  });
}

async function refresh() {
  const [data, rtk] = await Promise.all([
    invoke<PanelData>("get_panel_data"),
    invoke<RtkSavings | null>("get_rtk_savings"),
  ]);
  const { session, weekly, sessions, models, config: cfg } = data;

  renderUpdateBanner(data.update);

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
    const fmtPct = (p: number | null) =>
      p != null ? `${p.toFixed(2)}%` : "—";
    $<HTMLSpanElement>("dbg-session").textContent = fmtPct(session.percent);
    $<HTMLSpanElement>("dbg-weekly").textContent = fmtPct(weekly.percent);
  }

  // Open sessions
  const list = $<HTMLDivElement>("sessions-list");
  list.innerHTML = "";
  if (sessions.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No active session";
    list.appendChild(empty);
  } else {
    for (const s of sessions) {
      const pct = s.percent ?? 0;
      const row = document.createElement("div");
      row.className = "session";

      const top = document.createElement("div");
      top.className = "session-top";
      const nameEl = document.createElement("span");
      nameEl.className = "session-name";
      nameEl.textContent = shortCwd(s.cwd);
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

  // Weekly models — horizontal bars, longest = most tokens.
  const modelsList = $<HTMLDivElement>("models-list");
  modelsList.innerHTML = "";
  if (models.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No data yet";
    modelsList.appendChild(empty);
  } else {
    const max = Math.max(...models.map((m) => m.tokens), 1);
    for (const m of models) {
      const row = document.createElement("div");
      row.className = "model-row";

      const modelTop = document.createElement("div");
      modelTop.className = "model-top";
      const labelEl = document.createElement("span");
      labelEl.textContent = modelLabel(m.model);
      const tokensEl = document.createElement("span");
      tokensEl.className = "sub";
      tokensEl.textContent = fmt(m.tokens);
      modelTop.appendChild(labelEl);
      modelTop.appendChild(tokensEl);

      const barEl = document.createElement("div");
      barEl.className = "bar small";
      const fillEl = document.createElement("div");
      fillEl.className = "fill";
      fillEl.style.width = `${(m.tokens / max) * 100}%`;
      barEl.appendChild(fillEl);

      row.appendChild(modelTop);
      row.appendChild(barEl);
      modelsList.appendChild(row);
    }
  }

  updateLastUpdated();

  // RTK — greyed out (but visible) when not installed.
  const rtkBlock = $<HTMLElement>("rtk-block");
  const rtkContent = $<HTMLDivElement>("rtk-content");
  if (rtk) {
    rtkBlock.classList.remove("disabled");
    rtkContent.innerHTML = "";
    const head = document.createElement("div");
    head.className = "block-head";
    const savLabel = document.createElement("span");
    savLabel.textContent = "Savings";
    const savVal = document.createElement("span");
    savVal.className = "val good";
    savVal.textContent = `${rtk.summary.avg_savings_pct.toFixed(0)}%`;
    head.appendChild(savLabel);
    head.appendChild(savVal);
    const savSub = document.createElement("div");
    savSub.className = "sub";
    savSub.textContent = `${fmt(rtk.summary.total_saved)} tokens saved across ${rtk.summary.total_commands} commands`;
    rtkContent.appendChild(head);
    rtkContent.appendChild(savSub);
  } else {
    rtkBlock.classList.add("disabled");
    rtkContent.innerHTML = "";
    const sub = document.createElement("div");
    sub.className = "sub";
    sub.textContent = "RTK is not installed on this machine.";
    rtkContent.appendChild(sub);
  }
}

async function loadMemory() {
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

// Reads a percentage field, clamped to 0–100 (null if empty).
function pct(id: string): number | null {
  const v = $<HTMLInputElement>(id).value.trim();
  if (v === "") return null;
  const n = parseFloat(v);
  if (!Number.isFinite(n)) return null;
  return Math.max(0, Math.min(100, n));
}

// Keeps a percentage input within 0–100 as the user types.
function clampInput(id: string) {
  const el = $<HTMLInputElement>(id);
  el.addEventListener("input", () => {
    const n = parseFloat(el.value);
    if (Number.isFinite(n) && n > 100) el.value = "100";
    if (Number.isFinite(n) && n < 0) el.value = "0";
  });
}

function updateCalibStatus(cfg: Config) {
  const el = $<HTMLSpanElement>("calib-status");
  const done =
    cfg.session_calibration != null && cfg.session_calibration_2 != null &&
    cfg.weekly_calibration != null && cfg.weekly_calibration_2 != null;
  el.textContent = done ? "✓" : "●";
  el.className = "calib-status " + (done ? "done" : "pending");

  // Dynamic labels and hint visibility.
  $("calib-label-session").textContent =
    cfg.session_calibration != null ? "2nd - Session (5h)" : "First - Session (5h)";
  $("calib-label-weekly").textContent =
    cfg.weekly_calibration != null ? "2nd - Weekly limit" : "First - Weekly limit";
  const hint = $("calib-hint");
  if (done) hint.classList.add("hidden"); else hint.classList.remove("hidden");
}

function setupCalibration(cfg: Config) {
  if (cfg.weekly_reset_date) $<HTMLInputElement>("calib-reset").value = cfg.weekly_reset_date;
  updateCalibStatus(cfg);
  clampInput("calib-session");
  clampInput("calib-weekly");

  $<HTMLFormElement>("calib-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const msg = $<HTMLSpanElement>("calib-msg");
    try {
      await invoke("set_calibration", {
        sessionPercent: pct("calib-session"),
        weeklyPercent: pct("calib-weekly"),
        resetDate: $<HTMLInputElement>("calib-reset").value || null,
      });
      const updated = await invoke<Config>("get_config");
      const allDone =
        updated.session_calibration_2 != null && updated.weekly_calibration_2 != null;
      msg.textContent = allDone ? "Calibrated ✓" : "Saved — calibrate once more when notified.";
      updateCalibStatus(updated);
      await refresh();
    } catch (err) {
      msg.textContent = "Error: " + err;
    }
  });
}

function setupNotifications(cfg: Config) {
  $<HTMLInputElement>("notif-enabled").checked = cfg.notifications_enabled;
  const levels = cfg.alert_levels ?? [33, 66, 90];
  ["1", "2", "3"].forEach((i, idx) => {
    const el = $<HTMLInputElement>(`notif-level-${i}`);
    el.value = String(levels[idx] ?? 0);
    clampInput(`notif-level-${i}`);
  });
  // Prevent the toggle label click from bubbling up to the collapse button.
  $<HTMLInputElement>("notif-enabled").closest("label")
    ?.addEventListener("click", (e) => e.stopPropagation());

  $<HTMLButtonElement>("notif-save").addEventListener("click", async () => {
    const msg = $<HTMLSpanElement>("notif-msg");
    try {
      const enabled = $<HTMLInputElement>("notif-enabled").checked;
      const lvls = ["1", "2", "3"].map((i) => {
        const n = parseFloat($<HTMLInputElement>(`notif-level-${i}`).value);
        return Number.isFinite(n) ? Math.max(0, Math.min(100, n)) : 0;
      });
      await invoke("set_notifications", { enabled, levels: lvls });
      msg.textContent = "Saved ✓";
      await refresh();
    } catch (err) {
      msg.textContent = "Error: " + err;
    }
  });
}

function setupCollapse(toggleId: string, bodyId: string, onOpen?: () => void) {
  const toggle = $<HTMLButtonElement>(toggleId);
  const body = $<HTMLElement>(bodyId);
  toggle.addEventListener("click", () => {
    const opening = body.classList.contains("hidden");
    body.classList.toggle("hidden");
    toggle.querySelector(".chev")?.classList.toggle("open", opening);
    if (opening && onOpen) onOpen();
  });
}

// Resize the popup window to match its content height (no empty space when
// panels are collapsed). A ResizeObserver re-applies on any layout change.
const PANEL_WIDTH = 380;
function setupAutoResize() {
  const win = getCurrentWindow();
  const apply = () => {
    const h = Math.ceil(document.body.scrollHeight);
    void win.setSize(new LogicalSize(PANEL_WIDTH, h));
  };
  new ResizeObserver(apply).observe(document.body);
  apply();
}

let timer: number | undefined;

function setupTracking(cfg: Config) {
  const toggle = $<HTMLInputElement>("tracking-toggle");
  toggle.checked = cfg.tracking_enabled ?? true;
  toggle.addEventListener("change", async () => {
    await invoke("set_tracking", { enabled: toggle.checked });
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  setupAutoResize();
  setupUpdate();
  setupCollapse("sessions-toggle", "sessions-body");
  setupCollapse("models-toggle", "models-body");
  setupCollapse("memory-toggle", "memory-body", loadMemory);
  setupCollapse("notif-toggle", "notif-body");
  setupCollapse("rtk-toggle", "rtk-body");
  setupCollapse("calib-toggle", "calib-form");
  const osName = navigator.userAgent.toLowerCase().includes("mac") ? "macOS" : "Windows";
  const notifLabel = document.querySelector<HTMLSpanElement>("#notif-toggle > span");
  if (notifLabel) notifLabel.textContent = `${osName} notifications`;
  getVersion().then(v => {
    const el = document.getElementById("app-version");
    if (el) el.textContent = `v ${v}`;
  });
  const cfg = await invoke<Config>("get_config");
  setupCalibration(cfg);
  setupNotifications(cfg);
  setupTracking(cfg);
  await refresh();

  const interval = Math.max(5, cfg.refresh_secs) * 1000;
  timer = window.setInterval(refresh, interval);
});

window.addEventListener("beforeunload", () => {
  if (timer) window.clearInterval(timer);
});
