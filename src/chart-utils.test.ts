import { describe, it, expect } from "vitest";
import { sortModelsByFamily } from "./chart-utils";

describe("sortModelsByFamily", () => {
  const ORDER = ["haiku", "sonnet", "opus", "fable"];

  it("orders models by family priority", () => {
    const models = ["claude-opus-4-8", "claude-haiku-4-5", "claude-fable-5", "claude-sonnet-4-6"];
    expect(sortModelsByFamily(models, ORDER)).toEqual([
      "claude-haiku-4-5",
      "claude-sonnet-4-6",
      "claude-opus-4-8",
      "claude-fable-5",
    ]);
  });

  it("sorts unknown families alphabetically after known ones", () => {
    const models = ["claude-opus-4-8", "mystery-model-b", "mystery-model-a"];
    expect(sortModelsByFamily(models, ORDER)).toEqual([
      "claude-opus-4-8",
      "mystery-model-a",
      "mystery-model-b",
    ]);
  });

  it("does not mutate the input array", () => {
    const models = ["claude-opus-4-8", "claude-haiku-4-5"];
    const copy = [...models];
    sortModelsByFamily(models, ORDER);
    expect(models).toEqual(copy);
  });
});
