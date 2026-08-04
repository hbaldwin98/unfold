import { renderMarkdown } from "./markdown";

export type LearningAction =
  | "initial"
  | "socratic_response"
  | "follow_up"
  | "explain_term"
  | "another_hint"
  | "explain_step"
  | "check_attempt"
  | "reveal_solution";

export type LearningMode = "socratic" | "worked_example";

export type LearningTurn = {
  action: LearningAction;
  label: string;
  content: string;
  detail?: string;
};

export type PreviousTurn = {
  label: string;
  content: string;
  learnerDetail?: string;
};

const labels: Record<LearningAction, string> = {
  initial: "First question",
  socratic_response: "Next question",
  follow_up: "Follow-up",
  explain_term: "Term explanation",
  another_hint: "Guiding question",
  explain_step: "Step explanation",
  check_attempt: "Attempt feedback",
  reveal_solution: "Target solution",
};

export function labelForAction(action: LearningAction, mode: LearningMode = "socratic"): string {
  if (action === "initial" && mode === "worked_example") return "Worked example";
  return labels[action];
}

export function isLearningMode(value: string | null): value is LearningMode {
  return value === "socratic" || value === "worked_example";
}

export function supportsLearningActions(mode: LearningMode): boolean {
  return mode === "socratic";
}

export function previousTurns(turns: LearningTurn[]): PreviousTurn[] {
  return turns
    .filter((turn) => turn.content.trim().length > 0)
    .map(({ label, content, detail }) => ({
      label,
      content: visibleLearningContent(content),
      ...(detail ? { learnerDetail: detail } : {}),
    }));
}

export function extractSuggestions(content: string): string[] {
  const marker = /<!--\s*SUGGESTIONS\s*:\s*([\s\S]*?)-->/gi;
  let suggestions: string[] = [];

  for (const match of content.matchAll(marker)) {
    try {
      const parsed: unknown = JSON.parse(match[1] ?? "");
      if (!Array.isArray(parsed)) continue;
      const seen = new Set<string>();
      suggestions = parsed
        .filter((value): value is string => typeof value === "string")
        .map((value) => value.replace(/\s+/g, " ").trim())
        .filter((value) => {
          const key = value.toLocaleLowerCase();
          if (!value || value.length > 160 || seen.has(key)) return false;
          seen.add(key);
          return true;
        })
        .slice(0, 3);
    } catch {
      // A malformed optional marker should not affect the visible lesson.
    }
  }

  return suggestions;
}

export function extractTerms(content: string): string[] {
  const marker = /<!--\s*TERMS\s*:\s*([\s\S]*?)-->/gi;
  let terms: string[] = [];

  for (const match of content.matchAll(marker)) {
    try {
      const parsed: unknown = JSON.parse(match[1] ?? "");
      if (!Array.isArray(parsed)) continue;
      const seen = new Set<string>();
      terms = parsed
        .filter((value): value is string => typeof value === "string")
        .map((value) => value.replace(/\s+/g, " ").trim())
        .filter((value) => {
          const key = value.toLocaleLowerCase();
          if (!value || value.length > 100 || seen.has(key)) return false;
          seen.add(key);
          return true;
        })
        .slice(0, 5);
    } catch {
      // Optional term metadata must never prevent the lesson from rendering.
    }
  }

  return terms;
}

export function withoutSuggestionMarker(content: string): string {
  const complete = content.replace(/<!--\s*SUGGESTIONS\s*:[\s\S]*?-->/gi, "");
  const partialStart = complete.search(/<!--\s*SUGGESTIONS\b/i);
  return (partialStart >= 0 ? complete.slice(0, partialStart) : complete).trimEnd();
}

export function withoutTermMarker(content: string): string {
  const complete = content.replace(/<!--\s*TERMS\s*:[\s\S]*?-->/gi, "");
  const partialStart = complete.search(/<!--\s*TERMS\b/i);
  return (partialStart >= 0 ? complete.slice(0, partialStart) : complete).trimEnd();
}

export function withoutHiddenReasoning(content: string): string {
  const complete = content.replace(
    /<(think|analysis|reasoning)\b[^>]*>[\s\S]*?<\/\1\s*>/gi,
    "",
  );
  const partialStart = complete.search(/<(think|analysis|reasoning)\b[^>]*>/i);
  return (partialStart >= 0 ? complete.slice(0, partialStart) : complete).trimEnd();
}

export function visibleLearningContent(content: string): string {
  return withoutSuggestionMarker(withoutTermMarker(withoutHiddenReasoning(content)));
}

function markLearningTerms(root: HTMLElement, terms: string[]) {
  for (const term of terms) {
    const needle = term.toLocaleLowerCase();
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const parent = node.parentElement;
        if (
          !node.nodeValue?.toLocaleLowerCase().includes(needle) ||
          parent?.closest("button, a, code, pre, .katex")
        ) {
          return NodeFilter.FILTER_REJECT;
        }
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    const textNode = walker.nextNode();
    const value = textNode?.nodeValue;
    if (!textNode || !value) continue;
    const index = value.toLocaleLowerCase().indexOf(needle);
    if (index < 0) continue;

    const button = document.createElement("button");
    button.type = "button";
    button.className = "learning-term";
    button.dataset.term = value.slice(index, index + term.length);
    button.title = "Explain this term";
    button.textContent = value.slice(index, index + term.length);
    const replacement = document.createDocumentFragment();
    replacement.append(
      document.createTextNode(value.slice(0, index)),
      button,
      document.createTextNode(value.slice(index + term.length)),
    );
    textNode.parentNode?.replaceChild(replacement, textNode);
  }
}

export function renderLearningTurns(turns: LearningTurn[]): DocumentFragment {
  const fragment = document.createDocumentFragment();

  for (const turn of turns) {
    if (!turn.content && !turn.detail) continue;
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

    if (turn.content) {
      const label = document.createElement("p");
      label.className = "turn-label";
      label.textContent = turn.label;

      const content = document.createElement("div");
      content.className = "turn-content";
      content.innerHTML = renderMarkdown(visibleLearningContent(turn.content));
      markLearningTerms(content, extractTerms(turn.content));

      section.append(label, content);
    }
    fragment.append(section);
  }

  return fragment;
}
