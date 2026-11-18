import assert from "node:assert/strict";
import test from "node:test";
import { parseCsv, parseStudyDeck, renderMarkdown } from "../markdown.js";
import {
  ACCENT_MODES,
  APPEARANCE_DEFAULTS,
  DENSITY_MODES,
  TONE_MODES,
  computeWelcomeVisible,
  countStagedEdits,
  detectViewerForPreset,
  filterModels,
  formatBytes,
  formatRelativeTime,
  normalizeAppearance,
  parseDiffLines,
  shortPath,
} from "../state.js";

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

test("formats relative timestamps concisely", () => {
  const base = 1_700_000_000;
  assert.equal(formatRelativeTime(0, base), "now");
  assert.equal(formatRelativeTime(base - 25, base), "now");
  assert.equal(formatRelativeTime(base - 300, base), "5m");
  assert.equal(formatRelativeTime(base - 7200, base), "2h");
  assert.equal(formatRelativeTime(base - 86400, base), "1d");
  assert.equal(formatRelativeTime(base - 40 * 86400, base), "1mo");
});

test("normalizes appearance settings and welcome state", () => {
  assert.deepEqual(ACCENT_MODES, ["sage", "copper", "ice"]);
  assert.deepEqual(TONE_MODES, ["slate", "ink"]);
  assert.deepEqual(DENSITY_MODES, ["cozy", "compact"]);
  assert.deepEqual(APPEARANCE_DEFAULTS, {
    accent: "sage",
    tone: "ink",
    density: "cozy",
    sidebar: true,
    textSize: "regular",
  });
  assert.deepEqual(
    normalizeAppearance({
      accent: "copper",
      tone: "ink",
      density: "compact",
      sidebar: false,
      textSize: "large",
    }),
    {
      accent: "copper",
      tone: "ink",
      density: "compact",
      sidebar: false,
      textSize: "large",
    },
  );
  assert.deepEqual(
    normalizeAppearance({ accent: "bogus", tone: "x", density: "y", sidebar: "nope", textSize: null }),
    APPEARANCE_DEFAULTS,
  );
  assert.equal(computeWelcomeVisible(null), true);
  assert.equal(computeWelcomeVisible("0"), true);
  assert.equal(computeWelcomeVisible("1"), false);
});

test("parses diff lines and filters models", () => {
  const diff = parseDiffLines("--- index.html\n+++ index.html\n@@\n- <p>old</p>\n+ <p>new</p>\n ctx");
  assert.deepEqual(
    diff.map((d) => d.kind),
    ["meta", "meta", "meta", "del", "add", "ctx"],
  );

  const models = [
    { id: "minicpm5", name: "MiniCPM5 · 2B", quantization: "Q4_K_M", detail: "Fast local", installed: true },
    { id: "qwen3", name: "Qwen3-Coder", quantization: "Q4_K_M", detail: "Coding", installed: false },
  ];
  assert.equal(filterModels(models, "", "all").length, 2);
  assert.equal(filterModels(models, "", "installed").length, 1);
  assert.equal(filterModels(models, "qwen", "all")[0].id, "qwen3");
});
