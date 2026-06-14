import { describe, it, expect } from "vitest";
import { nextWeeklyReset, entrypointShort, modelShort, timeAgo } from "./utils";

const WEEK_MS = 7 * 24 * 3600 * 1000;
const FMT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/;

// Assertions are timezone-independent on purpose (CI runs in UTC): we check
// invariants relative to "now", not hard-coded absolute strings.
describe("nextWeeklyReset", () => {
  it("returns a datetime-local string", () => {
    expect(nextWeeklyReset("2026-06-09T18:00")).toMatch(FMT);
  });

  it("maps a far-past anchor to a reset in the future, within 7 days", () => {
    const out = nextWeeklyReset("2020-01-01T18:00");
    const t = new Date(out).getTime();
    expect(t).toBeGreaterThan(Date.now());
    expect(t).toBeLessThanOrEqual(Date.now() + WEEK_MS + 60_000);
  });

  it("preserves the wall-clock time of day of the anchor", () => {
    const out = new Date(nextWeeklyReset("2020-01-01T18:30"));
    expect(out.getHours()).toBe(18);
    expect(out.getMinutes()).toBe(30);
  });

  it("keeps the same weekday (advances by whole weeks)", () => {
    const anchor = new Date("2020-01-01T18:00");
    const out = new Date(nextWeeklyReset("2020-01-01T18:00"));
    expect(out.getDay()).toBe(anchor.getDay());
  });

  it("treats a date-only anchor as local midnight", () => {
    const out = new Date(nextWeeklyReset("2020-06-01"));
    expect(out.getHours()).toBe(0);
    expect(out.getMinutes()).toBe(0);
  });

  it("is idempotent: re-running on its own output is a no-op", () => {
    const out = nextWeeklyReset("2020-06-01T09:30");
    expect(nextWeeklyReset(out)).toBe(out);
  });

  it("brings a far-future anchor back to the next upcoming reset", () => {
    // 100 days ahead → result must still be the nearest upcoming (≤ 7 days out).
    const future = new Date(Date.now() + 100 * 24 * 3600 * 1000);
    const iso = future.toISOString().slice(0, 16); // YYYY-MM-DDTHH:MM
    const t = new Date(nextWeeklyReset(iso)).getTime();
    expect(t).toBeGreaterThan(Date.now());
    expect(t).toBeLessThanOrEqual(Date.now() + WEEK_MS + 60_000);
  });

  it("returns invalid input unchanged", () => {
    expect(nextWeeklyReset("not-a-date")).toBe("not-a-date");
    expect(nextWeeklyReset("")).toBe("");
  });
});

describe("entrypointShort", () => {
  it("maps known entrypoints to display labels", () => {
    expect(entrypointShort("cli")).toBe("CLI");
    expect(entrypointShort("claude-vscode")).toBe("VSCode");
  });

  it("passes unknown entrypoints through unchanged", () => {
    expect(entrypointShort("claude-jetbrains")).toBe("claude-jetbrains");
  });
});

describe("modelShort", () => {
  it("collapses model ids to their family name", () => {
    expect(modelShort("claude-fable-5")).toBe("Fable");
    expect(modelShort("claude-opus-4-8")).toBe("Opus");
    expect(modelShort("claude-sonnet-4-6")).toBe("Sonnet");
    expect(modelShort("claude-haiku-4-5-20251001")).toBe("Haiku");
  });

  it("returns ? for null and unknown ids unchanged", () => {
    expect(modelShort(null)).toBe("?");
    expect(modelShort("mystery-model")).toBe("mystery-model");
  });
});

describe("timeAgo", () => {
  const NOW_MS = 1_000_000_000 * 1000; // fixed "now", Unix seconds 1_000_000_000
  const at = (secsAgo: number) => timeAgo(1_000_000_000 - secsAgo, NOW_MS);

  it("under a minute is 'just now'", () => {
    expect(at(0)).toBe("just now");
    expect(at(59)).toBe("just now");
  });

  it("minutes, hours, days tiers", () => {
    expect(at(60)).toBe("1 min ago");
    expect(at(3599)).toBe("59 min ago");
    expect(at(3600)).toBe("1 h ago");
    expect(at(86399)).toBe("23 h ago");
    expect(at(86400)).toBe("1 d ago");
    expect(at(3 * 86400)).toBe("3 d ago");
  });

  it("clamps future timestamps to 'just now'", () => {
    expect(at(-120)).toBe("just now");
  });
});
