import { expect, test } from "vitest";
import { renderMarkdown } from "./markdown";

test("markdown never renders raw html", async () => {
  const html = await renderMarkdown("# T\n<script>alert(1)</script>\n[x](javascript:alert(1))");
  expect(html).toContain("<h1>");
  expect(html).not.toContain("<script");
  expect(html).not.toContain('href="javascript:');
});

test("markdown does not load remote images", async () => {
  const html = await renderMarkdown("![x](http://example.com/p.png) <img src=x onerror=alert(1)>");
  expect(html).not.toContain("<img");
});

test("markdown keeps ordinary links", async () => {
  expect(await renderMarkdown("[x](https://example.com)")).toContain('href="https://example.com"');
});
