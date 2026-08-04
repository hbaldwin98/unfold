import DOMPurify from "dompurify";
import katex from "katex";
import { marked, type TokenizerAndRendererExtension } from "marked";

type MathToken = {
  type: string;
  raw: string;
  text: string;
};

type TermToken = {
  type: string;
  raw: string;
  text: string;
};

const blockMath: TokenizerAndRendererExtension = {
  name: "blockMath",
  level: "block",
  start(source) {
    return firstDelimiter(source, ["$$", "\\["]);
  },
  tokenizer(source) {
    const match =
      /^\$\$\s*([\s\S]+?)\s*\$\$(?:\n|$)/.exec(source) ??
      /^\\\[\s*([\s\S]+?)\s*\\\](?:\n|$)/.exec(source);
    if (!match?.[1]) return;
    return { type: "blockMath", raw: match[0], text: match[1] };
  },
  renderer(token) {
    const math = token as MathToken;
    return katex.renderToString(math.text, {
      displayMode: true,
      output: "htmlAndMathml",
      strict: "ignore",
      throwOnError: false,
      trust: false,
    });
  },
};

const inlineMath: TokenizerAndRendererExtension = {
  name: "inlineMath",
  level: "inline",
  start(source) {
    return firstDelimiter(source, ["$", "\\("]);
  },
  tokenizer(source) {
    const match = /^\$([^\n$]+?)\$/.exec(source) ?? /^\\\(([^\n]+?)\\\)/.exec(source);
    if (!match?.[1]) return;
    return { type: "inlineMath", raw: match[0], text: match[1] };
  },
  renderer(token) {
    const math = token as MathToken;
    return katex.renderToString(math.text, {
      displayMode: false,
      output: "htmlAndMathml",
      strict: "ignore",
      throwOnError: false,
      trust: false,
    });
  },
};

const learningTerm: TokenizerAndRendererExtension = {
  name: "learningTerm",
  level: "inline",
  start(source) {
    const index = source.indexOf("[[term:");
    return index >= 0 ? index : undefined;
  },
  tokenizer(source) {
    const match = /^\[\[term:([^\]\n]{1,100})\]\]/.exec(source);
    const text = match?.[1]?.trim();
    if (!match || !text) return;
    return { type: "learningTerm", raw: match[0], text };
  },
  renderer(token) {
    const term = escapeHtml((token as TermToken).text);
    return `<button type="button" class="learning-term" data-term="${term}" title="Explain this term">${term}</button>`;
  },
};

marked.use({ extensions: [blockMath, inlineMath, learningTerm] });

function firstDelimiter(source: string, delimiters: string[]): number | undefined {
  const indexes = delimiters.map((delimiter) => source.indexOf(delimiter)).filter((index) => index >= 0);
  return indexes.length > 0 ? Math.min(...indexes) : undefined;
}

function escapeHtml(value: string): string {
  return value.replace(
    /[&<>"']/g,
    (character) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        character
      ] ?? character,
  );
}

export function renderMarkdown(markdown: string): string {
  const html = marked.parse(markdown, { async: false, gfm: true });
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true, mathMl: true },
  });
}
