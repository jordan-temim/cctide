import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";

import { $, updateLastUpdated, setupCollapsibles } from "./utils";
import { renderUsage } from "./tab-usage";
import { renderSessions, setupSessions, SESSIONS_COLLAPSIBLES } from "./tab-sessions";
import { setupCalibration, setupNotifications } from "./tab-settings";
import {
  renderChart,
  renderBreakdownChart,
  renderCostChart,
  refreshOutcomesIfOpen,
  updateProjectFilter,
  getProjectFilter,
  setupProjectFilter,
  ANALYTICS_COLLAPSIBLES,
} from "./tab-analytics";
import { renderRtk } from "./tab-extras";
import { renderUpdateBanner, setupUpdate } from "./update";
import type { PanelData, Config } from "./types";

async function refresh() {
  const data = await invoke<PanelData>("get_panel_data", {
    projectFilter: getProjectFilter(),
  });
  const { session, weekly, sessions, chart, config: cfg, rtk } = data;
  renderUpdateBanner(data.update);
  renderUsage(session, weekly, cfg);
  renderSessions(session, sessions);
  updateProjectFilter(data.projects);
  renderChart(chart);
  renderBreakdownChart(chart);
  renderCostChart(chart);
  renderRtk(rtk);
  updateLastUpdated();
}

function setupTabs() {
  const tabs = document.querySelectorAll<HTMLButtonElement>(".tab");
  const panels = document.querySelectorAll<HTMLDivElement>(".tab-panel");
  tabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      tabs.forEach((t) => t.classList.remove("active"));
      panels.forEach((p) => p.classList.add("hidden"));
      tab.classList.add("active");
      document.getElementById(`tab-${tab.dataset.tab}`)?.classList.remove("hidden");
      // Outcomes is open by default; load it lazily when the tab is first shown
      // (the section's own onOpen only fires on collapse toggle).
      if (tab.dataset.tab === "analytics") refreshOutcomesIfOpen();
    });
  });
}

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

function setupTracking(cfg: Config) {
  const toggle = $<HTMLInputElement>("tracking-toggle");
  const label = $<HTMLElement>("track-label");
  const syncLabel = () => {
    label.textContent = toggle.checked ? "Tracking on" : "Paused";
    label.classList.toggle("paused", !toggle.checked);
  };
  toggle.checked = cfg.tracking_enabled ?? true;
  syncLabel();
  toggle.addEventListener("change", async () => {
    // Optimistic UI update; revert + surface the error if the backend call fails.
    const desired = toggle.checked;
    syncLabel();
    try {
      await invoke("set_tracking", { enabled: desired });
    } catch (e) {
      console.error("set_tracking failed:", e);
      toggle.checked = !desired;
      syncLabel();
    }
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  setupAutoResize();
  setupUpdate();
  setupTabs();
  setupSessions(refresh);
  setupCollapsibles([...SESSIONS_COLLAPSIBLES, ...ANALYTICS_COLLAPSIBLES]);
  setupProjectFilter(async () => {
    await refresh();
    refreshOutcomesIfOpen();
  });
  const notifLabel = document.getElementById("notif-section-label");
  if (notifLabel) notifLabel.textContent = "macOS notifications";
  getVersion().then(v => {
    const el = document.getElementById("app-version");
    if (el) el.textContent = `v ${v}`;
  });
  const cfg = await invoke<Config>("get_config");
  setupCalibration(cfg, refresh);
  setupNotifications(cfg, refresh);
  setupTracking(cfg);

  void listen("refresh", () => refresh());
  void listen("UPDATE_AVAILABLE", () => refresh());
});
