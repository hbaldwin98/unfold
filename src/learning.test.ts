import { describe, expect, it } from "vitest";
import {
  labelForAction,
  previousTurns,
  renderLearningTurns,
  type LearningTurn,
} from "./learning";

const turns: LearningTurn[] = [
  { action: "initial", label: "Opening guidance", content: "Use **factoring** first." },
  { action: "another_hint", label: "Another hint", content: "Look for a common factor." },
];

describe("learning turns", () => {
  it("retains all completed turns as provider context", () => {
    expect(previousTurns(turns)).toEqual([
      { label: "Opening guidance", content: "Use **factoring** first." },
      { label: "Another hint", content: "Look for a common factor." },
    ]);
  });

  it("renders appended turns without replacing earlier guidance", () => {
    const host = document.createElement("div");
    host.append(renderLearningTurns(turns));

    expect(host.querySelectorAll(".learning-turn")).toHaveLength(2);
    expect(host.textContent).toContain("Use factoring first.");
    expect(host.textContent).toContain("Look for a common factor.");
  });

  it("uses a distinct label for an explicit target reveal", () => {
    expect(labelForAction("reveal_solution")).toBe("Target solution");
  });

  it("renders learner detail as text before its assistant turn", () => {
    const host = document.createElement("div");
    host.append(
      renderLearningTurns([
        {
          action: "check_attempt",
          label: "Attempt feedback",
          detail: "<img src=x onerror=alert(1)>",
          content: "Check the sign in line two.",
        },
      ]),
    );

    expect(host.querySelector(".learner-detail")?.textContent).toContain("<img src=x");
    expect(host.querySelector(".learner-detail img")).toBeNull();
  });
});
