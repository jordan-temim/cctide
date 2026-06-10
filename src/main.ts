import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";

import { $, updateLastUpdated } from "./utils";
import { renderUsage } from "./tab-usage";
import { renderSessions, setupSessions, loadMemory } from "./tab-sessions";
import { setupCalibration, setupNotifications } from "./tab-settings";
import { renderChart } from "./tab-analytics";
import { renderRtk } from "./tab-extras";
import { renderUpdateBanner, setupUpdate } from "./update";
import type { PanelData, Config } from "./types";

async function refresh() {
  const data = await invoke<PanelData>("get_panel_data");
  const { session, weekly, sessions, chart, config: cfg, rtk } = data;
  renderUpdateBanner(data.update);
  renderUsage(session, weekly, cfg);
  renderSessions(session, sessions);
  renderChart(chart);
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
    });
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
  toggle.checked = cfg.tracking_enabled ?? true;
  toggle.addEventListener("change", async () => {
    await invoke("set_tracking", { enabled: toggle.checked });
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  setupAutoResize();
  setupUpdate();
  setupTabs();
  setupSessions(refresh);
  setupCollapse("sessions-toggle", "sessions-body");
  setupCollapse("memory-toggle", "memory-body", loadMemory);
  const osName = navigator.userAgent.toLowerCase().includes("mac") ? "macOS" : "Windows";
  const notifLabel = document.getElementById("notif-section-label");
  if (notifLabel) notifLabel.textContent = `${osName} notifications`;
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
