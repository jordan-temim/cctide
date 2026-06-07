import { $, fmt } from "./utils";
import type { RtkSavings } from "./types";

export function renderRtk(rtk: RtkSavings | null) {
  const rtkBlock = $<HTMLElement>("rtk-block");
  const rtkSavings = $<HTMLSpanElement>("rtk-savings");
  const rtkContent = $<HTMLDivElement>("rtk-content");
  rtkContent.innerHTML = "";
  if (rtk) {
    rtkBlock.classList.remove("disabled");
    rtkSavings.textContent = `${rtk.summary.avg_savings_pct.toFixed(0)}% saved`;
    const savSub = document.createElement("div");
    savSub.className = "sub";
    savSub.textContent = `${fmt(rtk.summary.total_saved)} tokens saved across ${rtk.summary.total_commands} commands`;
    rtkContent.appendChild(savSub);
  } else {
    rtkBlock.classList.add("disabled");
    rtkSavings.textContent = "";
    const sub = document.createElement("div");
    sub.className = "sub";
    sub.textContent = "RTK is not installed on this machine.";
    rtkContent.appendChild(sub);
  }
}
