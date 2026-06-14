import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { $, fmt, shortCwd, modelShort, entrypointShort, colorClass, timeAgo } from "./utils";
import { createProjectFilter, type ProjectFilter } from "./project-filter";
import type { SessionUsage, SessionCtx, MemoryFile } from "./types";

let refreshCb: () => void = () => {};

// Per-project filter for the open-sessions list, plus the last data we rendered
// so a filter change can re-render without waiting for the next refresh.
let sessionFilter: ProjectFilter | null = null;
let lastUsage5h: SessionUsage | null = null;
let lastSessions: SessionCtx[] = [];

export function setupSessions(refresh: () => void) {
  refreshCb = refresh;
  sessionFilter = createProjectFilter(
    {
      bar: "sessions-filter-bar",
      btn: "sessions-filter-btn",
      menu: "sessions-filter-menu",
      value: "sessions-filter-value",
    },
    () => {
      if (lastUsage5h) renderSessions(lastUsage5h, lastSessions);
      // Keep Memory in sync with the same project when its section is open.
      if (!document.getElementById("memory-body")?.classList.contains("hidden")) {
        void loadMemory();
      }
    },
  );
  const btn = $<HTMLButtonElement>("sessions-cleanup");
  btn.addEventListener("click", async () => {
    const msg = $<HTMLSpanElement>("sessions-cleanup-msg");
    try {
      const n = await invoke<number>("cleanup_stale_sessions");
      msg.textContent = n > 0 ? `${n} stale file${n > 1 ? "s" : ""} removed` : "nothing to clean";
    } catch (e) {
      msg.textContent = String(e);
    }
    setTimeout(() => (msg.textContent = ""), 4000);
    refreshCb();
  });
}

function copyText(text: string) {
  // WebView clipboard fallback (API absent or permission denied).
  const fallback = () => {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  };
  if (navigator.clipboard) navigator.clipboard.writeText(text).catch(fallback);
  else fallback();
}

/// Number of currently armed confirmations — while > 0 the sessions list must
/// not re-render (the periodic refresh would silently wipe the armed prompt).
let armedConfirms = 0;

/// Two-step inline confirmation: first click arms the control (and shows the
/// optional warning note), second click runs the action. Clicking anywhere
/// else disarms it — no auto-timeout, the prompt stays until the user decides.
function confirmable(
  el: HTMLElement,
  confirmLabel: string,
  action: () => void,
  note?: HTMLElement,
) {
  const original = el.textContent ?? "";
  let armed = false;
  const disarm = () => {
    armed = false;
    armedConfirms = Math.max(0, armedConfirms - 1);
    el.textContent = original;
    el.classList.remove("danger");
    note?.classList.add("hidden");
    document.removeEventListener("click", onOutside, true);
  };
  const onOutside = (e: MouseEvent) => {
    if (e.target !== el) disarm();
  };
  el.addEventListener("click", () => {
    if (!armed) {
      armed = true;
      armedConfirms += 1;
      el.textContent = confirmLabel;
      el.classList.add("danger");
      note?.classList.remove("hidden");
      // Deferred so the arming click itself doesn't immediately disarm.
      setTimeout(() => document.addEventListener("click", onOutside, true), 0);
    } else {
      disarm();
      action();
    }
  });
}

export function renderSessions(usage5h: SessionUsage, sessions: SessionCtx[]) {
  // Don't wipe an armed Close/Delete prompt on the periodic refresh.
  if (armedConfirms > 0) return;
  lastUsage5h = usage5h;
  lastSessions = sessions;

  // Refresh the filter's options from the cwds currently open, then narrow the
  // list to the selected project (null → all). Options are derived from the
  // live sessions, so the active selection always matches at least one.
  const projects = [...new Set(sessions.map((s) => s.cwd))].sort();
  sessionFilter?.update(projects);
  const selected = sessionFilter?.getValue() ?? null;
  const shown = selected ? sessions.filter((s) => s.cwd === selected) : sessions;

  const list = $<HTMLDivElement>("sessions-list");
  list.innerHTML = "";
  if (shown.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No active session";
    list.appendChild(empty);
    return;
  }
  for (const s of shown) {
    const pct = s.percent ?? 0;
    const row = document.createElement("div");
    row.className = "session";

    const top = document.createElement("div");
    top.className = "session-top";
    const nameEl = document.createElement("span");
    nameEl.className = "session-name";
    if (s.status) {
      const dot = document.createElement("span");
      dot.className = `status-dot${s.status === "idle" ? "" : " active"}`;
      dot.title = s.status;
      nameEl.appendChild(dot);
    }
    // Title from the conversation's first prompt; fall back to the folder name.
    nameEl.appendChild(document.createTextNode(s.title ?? shortCwd(s.cwd)));
    nameEl.title = `${shortCwd(s.cwd)} · ${s.cwd}`;
    const badgesEl = document.createElement("span");
    badgesEl.className = "badges";
    if (s.entrypoint) {
      const epEl = document.createElement("span");
      epEl.className = "badge";
      epEl.textContent = entrypointShort(s.entrypoint);
      badgesEl.appendChild(epEl);
    }
    const badgeEl = document.createElement("span");
    badgeEl.className = "badge";
    badgeEl.textContent = modelShort(s.model);
    badgesEl.appendChild(badgeEl);
    top.appendChild(nameEl);
    top.appendChild(badgesEl);

    const barEl = document.createElement("div");
    barEl.className = "bar small";
    const fillEl = document.createElement("div");
    fillEl.className = `fill ${colorClass(pct)}`;
    fillEl.style.width = `${Math.min(100, pct)}%`;
    barEl.appendChild(fillEl);

    const subEl = document.createElement("div");
    subEl.className = "sub";
    const parts = [
      `${fmt(s.context_tokens)} / ${fmt(s.context_limit)} ctx (${Math.min(100, pct).toFixed(0)}%)`,
    ];
    if (s.weighted_5h > 0 && usage5h.weighted_tokens > 0) {
      parts.push(`${((s.weighted_5h / usage5h.weighted_tokens) * 100).toFixed(0)}% of 5h window`);
    }
    subEl.textContent = parts.join(" · ");

    const actionsEl = document.createElement("div");
    actionsEl.className = "session-actions";
    const warnEl = document.createElement("div");
    warnEl.className = "warn-note hidden";

    const copyBtn = document.createElement("button");
    copyBtn.type = "button";
    copyBtn.className = "act-btn";
    copyBtn.textContent = "Copy resume";
    copyBtn.title = "Copy the claude --resume command";
    copyBtn.addEventListener("click", () => {
      copyText(`claude --resume ${s.session_id}`);
      copyBtn.textContent = "Copied!";
      setTimeout(() => (copyBtn.textContent = "Copy resume"), 1500);
    });

    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.className = "act-btn";
    closeBtn.textContent = "Close";
    closeBtn.title = "Terminate the Claude Code process";
    confirmable(closeBtn, "Confirm close?", async () => {
      try {
        await invoke("kill_session", { pid: s.pid });
      } catch (e) {
        warnEl.textContent = String(e);
        warnEl.classList.remove("hidden");
      }
      refreshCb();
    });

    const inWindow = s.weighted_5h > 0;
    const delBtn = document.createElement("button");
    delBtn.type = "button";
    delBtn.className = "act-btn";
    delBtn.textContent = "Delete";
    delBtn.title = "Delete the session transcript (removes it from /resume)";
    const delNote = document.createElement("div");
    delNote.className = "warn-note hidden";
    delNote.textContent = inWindow
      ? "This session has activity in the current 5h window: after deleting, the gauges will under-count until the next reset. Recalibrate via /usage afterwards."
      : "Deletes the transcript file — the conversation can no longer be resumed.";
    confirmable(
      delBtn,
      "Confirm delete?",
      async () => {
        try {
          await invoke("delete_session_transcript", { sessionId: s.session_id });
        } catch (e) {
          warnEl.textContent = String(e);
          warnEl.classList.remove("hidden");
        }
        refreshCb();
      },
      delNote,
    );

    actionsEl.appendChild(copyBtn);
    actionsEl.appendChild(closeBtn);
    actionsEl.appendChild(delBtn);
    if (s.updated_at) {
      const activityEl = document.createElement("span");
      activityEl.className = "session-activity";
      activityEl.textContent = `${s.status ?? "active"} ${timeAgo(s.updated_at)}`;
      actionsEl.appendChild(activityEl);
    }

    row.appendChild(top);
    row.appendChild(barEl);
    row.appendChild(subEl);
    row.appendChild(actionsEl);
    row.appendChild(delNote);
    row.appendChild(warnEl);
    list.appendChild(row);
  }
}

export async function loadMemory() {
  const body = $<HTMLDivElement>("memory-body");
  const selected = sessionFilter?.getValue() ?? null;
  const files = await invoke<MemoryFile[]>("get_memory", { projectFilter: selected });
  body.innerHTML = "";
  if (files.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = selected
      ? "No memory for this project"
      : "No memory for the active sessions";
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
    const delSpan = document.createElement("span");
    delSpan.className = "mem-del";
    delSpan.title = "Delete this memory file";
    delSpan.textContent = "✕";
    const openSpan = document.createElement("span");
    openSpan.className = "open";
    openSpan.title = "Open";
    openSpan.textContent = "↗";
    head.appendChild(nameSpan);
    head.appendChild(delSpan);
    head.appendChild(openSpan);
    const pre = document.createElement("pre");
    pre.className = "mem-content hidden";
    pre.textContent = f.content;
    confirmable(delSpan, "sure?", () => {
      invoke("delete_memory_file", { path: f.path })
        .then(() => loadMemory())
        .catch(() => {});
    });
    head.addEventListener("click", (e) => {
      const target = e.target as HTMLElement;
      if (target.classList.contains("open")) {
        openPath(f.path).catch(() => {});
        return;
      }
      // Delete clicks are handled by the confirmable on the ✕ itself.
      if (target.classList.contains("mem-del")) return;
      pre.classList.toggle("hidden");
    });
    item.appendChild(head);
    item.appendChild(pre);
    body.appendChild(item);
  }
}
