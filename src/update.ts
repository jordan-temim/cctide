import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { $ } from "./utils";
import type { UpdateInfo } from "./types";

let currentUpdate: UpdateInfo | null = null;
let updateStaged = false;

export function renderUpdateBanner(update: UpdateInfo | null) {
  currentUpdate = update;
  const banner = $("update-banner");
  if (!update || updateStaged) {
    if (!updateStaged) banner.classList.add("hidden");
    return;
  }
  banner.classList.remove("hidden");
  $("update-text").textContent = `Update available: v${update.version}`;
}

export function setupUpdate() {
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
