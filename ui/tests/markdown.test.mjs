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
