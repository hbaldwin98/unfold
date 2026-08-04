import { describe, expect, it } from "vitest";
import {
  extractSuggestions,
  extractTerms,
  labelForAction,
  isLearningMode,
  previousTurns,
  renderLearningTurns,
  supportsLearningActions,
  withoutHiddenReasoning,
  withoutTermMarker,
  withoutSuggestionMarker,
  type LearningTurn,
} from "./learning";

const turns: LearningTurn[] = [
  { action: "initial", label: "First question", content: "What operation might help first?" },
  {
    action: "socratic_response",
    label: "Next question",
    detail: "I would subtract four.",
    content: "What remains after that operation?",
  },
];

describe("learning turns", () => {
  it("retains all completed turns as provider context", () => {
    expect(previousTurns(turns)).toEqual([
      { label: "First question", content: "What operation might help first?" },
      {
        label: "Next question",
        content: "What remains after that operation?",
        learnerDetail: "I would subtract four.",
      },
    ]);
  });

  it("renders appended turns without replacing earlier guidance", () => {
    const host = document.createElement("div");
    host.append(renderLearningTurns(turns));

    expect(host.querySelectorAll(".learning-turn")).toHaveLength(2);
    expect(host.textContent).toContain("What operation might help first?");
    expect(host.textContent).toContain("I would subtract four.");
    expect(host.textContent).toContain("What remains after that operation?");
  });

  it("uses a distinct label for an explicit target reveal", () => {
    expect(labelForAction("reveal_solution")).toBe("Target solution");
  });

  it("labels a one-shot initial turn as a worked example", () => {
    expect(labelForAction("initial", "worked_example")).toBe("Worked example");
    expect(labelForAction("initial", "socratic")).toBe("First question");
  });

  it("offers follow-up learning actions only in Socratic mode", () => {
    expect(supportsLearningActions("socratic")).toBe(true);
    expect(supportsLearningActions("worked_example")).toBe(false);
  });

  it("accepts only persisted learning-mode values", () => {
    expect(isLearningMode("socratic")).toBe(true);
    expect(isLearningMode("worked_example")).toBe(true);
    expect(isLearningMode("solve_everything")).toBe(false);
    expect(isLearningMode(null)).toBe(false);
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

  it("renders a submitted learner response before the assistant starts streaming", () => {
    const host = document.createElement("div");
    host.append(
      renderLearningTurns([
        {
          action: "socratic_response",
          label: "Next question",
          detail: "I think the two quantities are proportional.",
          content: "",
        },
      ]),
    );

    expect(host.querySelector(".learner-detail")?.textContent).toContain("proportional");
    expect(host.querySelector(".turn-label")).toBeNull();
    expect(host.querySelector(".turn-content")).toBeNull();
  });

  it("extracts bounded unique model-generated suggestions", () => {
    const content =
      '## First question\nWhat would you try?\n<!--SUGGESTIONS:["Subtract four", " subtract   four ", "Draw a diagram", "Check the units"]-->';

    expect(extractSuggestions(content)).toEqual(["Subtract four", "Draw a diagram", "Check the units"]);
  });

  it("removes complete and streaming suggestion markers from visible content", () => {
    expect(withoutSuggestionMarker("Question?\n<!--SUGGESTIONS:[\"Try x\"]-->")).toBe("Question?");
    expect(withoutSuggestionMarker("Question?\n<!--SUGGESTIONS:[\"Try")).toBe("Question?");
  });

  it("ignores malformed optional suggestions", () => {
    expect(extractSuggestions("Question? <!--SUGGESTIONS:not-json-->")).toEqual([]);
  });

  it("hides complete and streaming reasoning blocks", () => {
    expect(withoutHiddenReasoning("<think>private chain</think>Visible answer")).toBe(
      "Visible answer",
    );
    expect(withoutHiddenReasoning("<analysis>still thinking")).toBe("");
    expect(withoutHiddenReasoning("Answer<reasoning>hidden tail")).toBe("Answer");
  });

  it("extracts bounded unique unfamiliar terms and hides their metadata", () => {
    const content =
      'Use an inverse operation on the coefficient.\n<!--TERMS:["inverse operation","coefficient","Coefficient"]-->';

    expect(extractTerms(content)).toEqual(["inverse operation", "coefficient"]);
    expect(withoutTermMarker(content)).toBe("Use an inverse operation on the coefficient.");
    expect(withoutTermMarker("Answer\n<!--TERMS:[\"partial")).toBe("Answer");
  });

  it("marks first-use technical terms in rendered turns for either learning mode", () => {
    const host = document.createElement("div");
    host.append(
      renderLearningTurns([
        {
          action: "initial",
          label: "Worked example",
          content:
            'Apply the inverse operation. The inverse operation preserves equality.\n<!--TERMS:["inverse operation"]-->',
        },
      ]),
    );

    const buttons = host.querySelectorAll<HTMLButtonElement>(".learning-term");
    expect(buttons).toHaveLength(1);
    expect(buttons[0]?.dataset.term).toBe("inverse operation");
    expect(host.textContent).not.toContain("TERMS");
    expect(host.textContent).toContain("The inverse operation preserves equality.");
  });
});
