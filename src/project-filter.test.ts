// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createProjectFilter } from "./project-filter";

const IDS = { bar: "bar", btn: "btn", menu: "menu", value: "value" };

function scaffold() {
  document.body.innerHTML = `
    <div id="bar">
      <button id="btn"><span id="value">All projects</span></button>
      <div id="menu" class="hidden"></div>
    </div>`;
}

function optionLabels(): (string | null)[] {
  return [...document.querySelectorAll("#menu .filter-option")].map((e) => e.textContent);
}

function option(i: number): HTMLElement {
  return [...document.querySelectorAll("#menu .filter-option")][i] as HTMLElement;
}

beforeEach(scaffold);

describe("createProjectFilter", () => {
  it("starts with no selection", () => {
    const f = createProjectFilter(IDS, () => {});
    expect(f.getValue()).toBe(null);
  });

  it("hides the bar with fewer than two projects, shows it otherwise", () => {
    const f = createProjectFilter(IDS, () => {});
    const bar = document.getElementById("bar")!;
    f.update([]);
    expect(bar.style.display).toBe("none");
    f.update(["/a/one"]);
    expect(bar.style.display).toBe("none");
    f.update(["/a/one", "/b/two"]);
    expect(bar.style.display).toBe("");
  });

  it("builds an 'All projects' option plus one per project, labelled by basename", () => {
    const f = createProjectFilter(IDS, () => {});
    f.update(["/home/u/havi", "/home/u/cctide"]);
    expect(optionLabels()).toEqual(["All projects", "havi", "cctide"]);
  });

  it("selecting an option updates value + label, fires onChange and closes the menu", () => {
    const onChange = vi.fn();
    const f = createProjectFilter(IDS, onChange);
    f.update(["/home/u/havi", "/home/u/cctide"]);
    option(1).click(); // "havi"
    expect(f.getValue()).toBe("/home/u/havi");
    expect(document.getElementById("value")!.textContent).toBe("havi");
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(document.getElementById("menu")!.classList.contains("hidden")).toBe(true);
  });

  it("'All projects' clears the selection", () => {
    const f = createProjectFilter(IDS, () => {});
    f.update(["/home/u/havi", "/home/u/cctide"]);
    option(1).click();
    expect(f.getValue()).toBe("/home/u/havi");
    option(0).click(); // "All projects"
    expect(f.getValue()).toBe(null);
    expect(document.getElementById("value")!.textContent).toBe("All projects");
  });

  it("resets the selection when the chosen project disappears from the list", () => {
    const f = createProjectFilter(IDS, () => {});
    f.update(["/home/u/havi", "/home/u/cctide"]);
    option(1).click(); // select havi
    expect(f.getValue()).toBe("/home/u/havi");
    f.update(["/home/u/cctide", "/home/u/other"]); // havi gone
    expect(f.getValue()).toBe(null);
    expect(document.getElementById("value")!.textContent).toBe("All projects");
  });

  it("the toggle button opens and closes the menu", () => {
    createProjectFilter(IDS, () => {});
    const btn = document.getElementById("btn")!;
    const menu = document.getElementById("menu")!;
    expect(menu.classList.contains("hidden")).toBe(true);
    btn.click();
    expect(menu.classList.contains("hidden")).toBe(false);
    btn.click();
    expect(menu.classList.contains("hidden")).toBe(true);
  });
});
