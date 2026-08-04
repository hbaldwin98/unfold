import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

describe("renderMarkdown", () => {
  it("removes executable model output", () => {
    const html = renderMarkdown('<img src="x" onerror="alert(1)"><script>alert(2)</script>');
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("<script");
  });

  it("renders inline mathematics", () => {
    const html = renderMarkdown("Use $x^2 + 1$ next.");
    expect(html).toContain("katex");
    expect(html).toContain("x");
  });

  it("renders LaTeX bracket delimiters instead of exposing them as text", () => {
    const html = renderMarkdown("\\[ \\text{complement} = \\text{target} - x \\]");

    expect(html).toContain("katex-display");
    expect(html).toContain("complement");
    expect(html).not.toMatch(/<p>\s*\[/);
  });

  it("renders LaTeX parenthesis delimiters inline", () => {
    const html = renderMarkdown("Use \\(x + 2\\) in the next step.");

    expect(html).toContain("katex");
    expect(html).not.toContain("\\(");
  });
});
