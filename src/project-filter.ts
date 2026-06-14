// Reusable per-project filter dropdown (custom — forces open-downward).
// Shared by the Analytics and Sessions tabs so "a project" means the same
// control in both. Each instance is bound to its own set of element IDs.

export interface ProjectFilter {
  /** Currently selected cwd, or null for "All projects". */
  getValue(): string | null;
  /** Rebuild the option list; hides the bar when there is nothing to filter. */
  update(projects: string[]): void;
}

export interface FilterIds {
  /** The bar wrapper, hidden when fewer than two projects exist. */
  bar: string;
  /** The toggle button. */
  btn: string;
  /** The options container. */
  menu: string;
  /** The element showing the current selection's label. */
  value: string;
}

export function createProjectFilter(ids: FilterIds, onChange: () => void): ProjectFilter {
  let value: string | null = null;

  const label = () => document.getElementById(ids.value);
  const menu = () => document.getElementById(ids.menu);
  const closeMenu = () => menu()?.classList.add("hidden");
  const labelFor = (cwd: string | null) => (cwd ? cwd.split("/").pop() || cwd : "All projects");

  const setValue = (cwd: string | null) => {
    value = cwd;
    const l = label();
    if (l) l.textContent = labelFor(cwd);
    closeMenu();
    onChange();
  };

  document.getElementById(ids.btn)?.addEventListener("click", (e) => {
    e.stopPropagation();
    menu()?.classList.toggle("hidden");
  });
  document.addEventListener("click", closeMenu);

  return {
    getValue: () => value,
    update(projects: string[]) {
      const bar = document.getElementById(ids.bar);
      if (!bar) return;
      bar.style.display = projects.length > 1 ? "" : "none";

      // Reset the selection if the chosen project vanished from the list.
      if (value && !projects.includes(value)) {
        value = null;
        const l = label();
        if (l) l.textContent = "All projects";
      }

      const m = menu();
      if (!m) return;
      while (m.firstChild) m.removeChild(m.firstChild);
      const addOption = (text: string, v: string | null) => {
        const el = document.createElement("div");
        el.className = "filter-option" + (value === v ? " active" : "");
        el.textContent = text;
        el.addEventListener("click", (e) => {
          e.stopPropagation();
          setValue(v);
        });
        m.appendChild(el);
      };
      addOption("All projects", null);
      for (const cwd of projects) addOption(labelFor(cwd), cwd);
    },
  };
}
