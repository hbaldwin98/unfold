import DOMPurify from "dompurify";
import katex from "katex";
import { marked, type TokenizerAndRendererExtension } from "marked";

type MathToken = {
  type: string;
  raw: string;
  text: string;
};

const blockMath: TokenizerAndRendererExtension = {
  name: "blockMath",
  level: "block",
  start(source) {
    return source.indexOf("$$");
  },
  tokenizer(source) {
    const match = /^\$\$\s*([\s\S]+?)\s*\$\$(?:\n|$)/.exec(source);
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
    return source.indexOf("$");
  },
  tokenizer(source) {
    const match = /^\$([^\n$]+?)\$/.exec(source);
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

marked.use({ extensions: [blockMath, inlineMath] });

export function renderMarkdown(markdown: string): string {
  const html = marked.parse(markdown, { async: false, gfm: true });
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true, mathMl: true },
  });
}
