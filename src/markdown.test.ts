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
});
