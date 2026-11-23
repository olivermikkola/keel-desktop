import test from "node:test";
import assert from "node:assert/strict";
import { renderMarkdown } from "../markdown.js";

test("renders ordinary assistant answers with tables, ordered steps, quotes and copyable code", () => {
  const result = renderMarkdown('# A plan\n\n1. First\n2. Second\n\n| Choice | Cost |\n| --- | --- |\n| Local | Free |\n\n> A note\n\n```js\nconst x = "<tag>";\n```');
  for (const tag of ['<ol>','<table>','<th>Choice</th>','<blockquote>','data-copy-code']) assert.ok(result.includes(tag),tag);
  assert.ok(result.includes('&lt;tag&gt;'));
});

test("escapes arbitrary HTML and rejects executable link schemes", () => {
  const result = renderMarkdown('[click](javascript:alert) [safe](https://example.com/?a=1&b=2)\n<script>alert(1)</script>\n`**literal**`');
  assert.ok(!result.includes('href="javascript:'));
  assert.ok(!result.includes('<script>'));
  assert.ok(result.includes('https://example.com/?a=1&amp;b=2'));
  assert.ok(result.includes('<code>**literal**</code>'));
});

test("CSV previews keep multiline quoted fields in the same row", async () => {
  const { parseCsv } = await import("../markdown.js");
  assert.deepEqual(parseCsv('name,note\nA,"first\nsecond"\nB,"a ""quote"""'), [['name','note'],['A','first\nsecond'],['B','a "quote"']]);
});

test("handles balanced-paren URLs, single-escaped quotes, merged blockquotes, and task lists", () => {
  const md = [
    "[Rust](https://en.wikipedia.org/wiki/Rust_(programming_language))",
    "[Quote](https://example.com/?q=a'b)",
    "",
    "> Line one",
    "> Line two",
    "",
    "- [x] Done task",
    "- [ ] Pending task",
    "",
    "***important***",
  ].join("\n");
  const html = renderMarkdown(md);
  assert.ok(html.includes('href="https://en.wikipedia.org/wiki/Rust_(programming_language)"'));
  assert.ok(html.includes("&#39;") && !html.includes("&amp;#39;"));
  assert.ok(html.includes("<blockquote>Line one<br>Line two</blockquote>"));
  assert.ok(html.includes('class="md-task-item"') && html.includes("checked"));
  assert.ok(html.includes("<strong><em>important</em></strong>"));
});
