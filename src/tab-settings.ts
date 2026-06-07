import { invoke } from "@tauri-apps/api/core";
import { $, pct, clampInput } from "./utils";
import type { Config } from "./types";

function updateCalibStatus(cfg: Config) {
  const el = $<HTMLSpanElement>("calib-status");
  const done =
    cfg.session_calibration != null && cfg.session_calibration_2 != null &&
    cfg.weekly_calibration != null && cfg.weekly_calibration_2 != null;
  el.textContent = done ? "✓" : "●";
  el.className = "calib-status " + (done ? "done" : "pending");

  $("calib-label-session").textContent =
    cfg.session_calibration != null ? "2nd - Session (5h)" : "First - Session (5h)";
  $("calib-label-weekly").textContent =
    cfg.weekly_calibration != null ? "2nd - Weekly limit" : "First - Weekly limit";
  const hint = $("calib-hint");
  if (done) hint.classList.add("hidden"); else hint.classList.remove("hidden");
}

export function setupCalibration(cfg: Config, onSave: () => Promise<void>) {
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
      await onSave();
    } catch (err) {
      msg.textContent = "Error: " + err;
    }
  });
}

export function setupNotifications(cfg: Config, onSave: () => Promise<void>) {
  $<HTMLInputElement>("notif-enabled").checked = cfg.notifications_enabled;
  const levels = cfg.alert_levels ?? [33, 66, 90];
  ["1", "2", "3"].forEach((i, idx) => {
    const el = $<HTMLInputElement>(`notif-level-${i}`);
    el.value = String(levels[idx] ?? 0);
    clampInput(`notif-level-${i}`);
  });
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
      await onSave();
    } catch (err) {
      msg.textContent = "Error: " + err;
    }
  });
}
