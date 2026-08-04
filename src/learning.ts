import { renderMarkdown } from "./markdown";

export type LearningAction =
  | "initial"
  | "another_hint"
  | "explain_step"
  | "check_attempt"
  | "reveal_solution";

export type LearningTurn = {
  action: LearningAction;
  label: string;
  content: string;
  detail?: string;
};

export type PreviousTurn = {
  label: string;
  content: string;
};

const labels: Record<LearningAction, string> = {
  initial: "Opening guidance",
  another_hint: "Another hint",
  explain_step: "Step explanation",
  check_attempt: "Attempt feedback",
  reveal_solution: "Target solution",
};

export function labelForAction(action: LearningAction): string {
  return labels[action];
}

export function previousTurns(turns: LearningTurn[]): PreviousTurn[] {
  return turns
    .filter((turn) => turn.content.trim().length > 0)
    .map(({ label, content }) => ({ label, content }));
}

export function renderLearningTurns(turns: LearningTurn[]): DocumentFragment {
  const fragment = document.createDocumentFragment();

  for (const turn of turns) {
    if (!turn.content) continue;
    const section = document.createElement("section");
    section.className = "learning-turn";
    section.dataset.action = turn.action;

    if (turn.detail) {
      const learnerDetail = document.createElement("div");
      learnerDetail.className = "learner-detail";
      const learnerLabel = document.createElement("span");
      learnerLabel.textContent = "You";
      const learnerText = document.createElement("p");
      learnerText.textContent = turn.detail;
      learnerDetail.append(learnerLabel, learnerText);
      section.append(learnerDetail);
    }

    const label = document.createElement("p");
    label.className = "turn-label";
    label.textContent = turn.label;

    const content = document.createElement("div");
    content.className = "turn-content";
    content.innerHTML = renderMarkdown(turn.content);

    section.append(label, content);
    fragment.append(section);
  }

  return fragment;
}
