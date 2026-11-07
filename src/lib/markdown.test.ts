import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown.ts";

// Agent output is rendered with {@html}; a model echoing a malicious file or web page must not run script.
describe("renderMarkdown", () => {
  it.each([
    ["a script tag", "<script>alert(1)</script>", "<script"],
    ["an inline handler", '<img src=x onerror="alert(1)">', "<img"],
    ["a raw HTML block", '<div onclick="alert(1)">hi</div>', "<div"],
    ["a javascript: link", "[click](javascript:alert(1))", "javascript:"],
    ["a data: link", "[click](data:text/html,<script>alert(1)</script>)", "data:"],
    ["a quote breaking out of href", '[x](https://a.example/" onmouseover="alert(1))', '" onmouseover'],
    ["a markdown image", '![x" onerror="alert(1)](https://a.example/i.png)', "<img"],
  ])("neutralizes %s", (_name, input, forbidden) => {
    expect(renderMarkdown(input)).not.toContain(forbidden);
  });

  it("keeps ordinary links, opening outside the app", () => {
    expect(renderMarkdown("[docs](https://example.com/a?b=1&c=2)")).toContain(
      '<a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noreferrer">docs</a>',
    );
  });
});
