import assert from "node:assert/strict";
import test from "node:test";
import { parseCsv, parseStudyDeck, renderMarkdown } from "../markdown.js";
import { countStagedEdits, detectViewerForPreset, formatBytes, shortPath } from "../state.js";

test("formats byte sizes cleanly", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1024), "1.0 KB");
  assert.equal(formatBytes(15 * 1024 * 1024), "15 MB");
});

test("counts staged edits by status", () => {
  const counts = countStagedEdits([
    { status: "applied" },
    { status: "pending" },
    { status: "pending" },
    { status: "discarded" },
  ]);
  assert.deepEqual(counts, { applied: 1, pending: 2, discarded: 1 });
});

test("detects viewer file for each preset", () => {
  const files = ["src/", "src/app.js", "index.html", "sales.csv", "notes.md", "slides.html"];
  assert.equal(detectViewerForPreset("websites", files), "index.html");
  assert.equal(detectViewerForPreset("slides", files), "slides.html");
  assert.equal(detectViewerForPreset("sheets", files), "sales.csv");
  assert.equal(detectViewerForPreset("docs", files), "notes.md");
  assert.equal(shortPath("/home/user/projects/keel-desktop"), ".../projects/keel-desktop");
});

test("escapes HTML in markdown renderer", () => {
  const html = renderMarkdown("# Title\n\nHello <script>alert(1)</script> **world**");
  assert.match(html, /<h1>Title<\/h1>/);
  assert.ok(!html.includes("<script>"));
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.match(html, /<strong>world<\/strong>/);
});

test("parses quoted CSV rows and study flashcards", () => {
  const rows = parseCsv('name,note\n"Cafe, North",42\nTea,10');
  assert.equal(rows.length, 3);
  assert.deepEqual(rows[1], ["Cafe, North", "42"]);

  const deck = parseStudyDeck(
    "# Space Quiz\n\nQ: Closest planet to the Sun?\n- Venus\n- Mercury\nA: Mercury\n",
  );
  assert.equal(deck.title, "Space Quiz");
  assert.equal(deck.cards.length, 1);
  assert.deepEqual(deck.cards[0].choices, ["Venus", "Mercury"]);
  assert.equal(deck.cards[0].answer, "Mercury");
});
