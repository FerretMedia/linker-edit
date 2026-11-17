import { describe, expect, it } from "vitest";
import { shortenText, shortenToolOutput } from "./shorten.ts";

const clips = (count: number) => JSON.stringify({ project: "Doc", items: Array.from({ length: count }, (_, i) => ({ name: `clip ${i}` })) });

describe("shortenText", () => {
  it("leaves a result that fits alone", () => {
    expect(shortenText(clips(3), 1000)).toBeNull();
  });

  it("caps long lists in JSON, keeping the rest and saying how much was left out", () => {
    const text = shortenText(clips(500), 2000)!;
    expect(text.length).toBeLessThanOrEqual(2000 + 200);
    expect(text).toContain('"project":"Doc"');
    expect(text).toContain('"name":"clip 0"');
    expect(text).toMatch(/… \d+ more items left out/);
    expect(text).toContain("Ask again for a narrower part");
  });

  it.each([
    ["text that isn't JSON", "x".repeat(5000)],
    ["JSON whose size is one long string", JSON.stringify({ transcript: "word ".repeat(1000) })],
  ])("cuts %s at the limit", (_name, input) => {
    const text = shortenText(input, 1000)!;
    expect(text.startsWith(input.slice(0, 1000))).toBe(true);
    expect(text).toContain(`${input.length - 1000} more characters left out`);
  });
});

describe("shortenToolOutput", () => {
  it("shortens the text blocks of Claude's MCP result array and keeps the others", () => {
    const image = { type: "image", data: "abc" };
    const output = shortenToolOutput([{ type: "text", text: clips(500) }, image], 2000) as { type: string; text?: string }[];
    expect(output[0]!.text).toMatch(/more items left out/);
    expect(output[1]).toBe(image);
  });

  it("drops structuredContent from an MCP CallToolResult, since it repeats the full result", () => {
    const output = shortenToolOutput({ content: [{ type: "text", text: clips(500) }], structuredContent: { big: true }, isError: false }, 2000);
    expect(output).toMatchObject({ isError: false, content: [{ type: "text" }] });
    expect(output).not.toHaveProperty("structuredContent");
  });

  it.each([
    ["a result that fits", [{ type: "text", text: "{}" }]],
    ["an unknown shape", "plain string"],
  ])("returns undefined for %s, leaving it alone", (_name, input) => {
    expect(shortenToolOutput(input, 2000)).toBeUndefined();
  });
});
