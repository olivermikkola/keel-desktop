import assert from "node:assert/strict";
import test from "node:test";
import { parseCsv, parseStudyDeck, renderMarkdown } from "../markdown.js";
import {
  ACCENT_MODES,
  APPEARANCE_DEFAULTS,
  DENSITY_MODES,
  TONE_MODES,
  countStagedEdits,
  detectViewerForPreset,
  estimateSessionTokens,
  filterModels,
  findReusableEmptySession,
  formatBytes,
  formatContextMeter,
  formatLiveTelemetry,
  formatRelativeTime,
  modelFitLabel,
  normalizeAppearance,
  parseDiffLines,
  pickInitialSession,
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

test("normalizes appearance settings", () => {
  assert.deepEqual(ACCENT_MODES, ["sage", "copper", "ice"]);
  assert.deepEqual(TONE_MODES, ["slate", "ink", "paper"]);
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

test("estimates model RAM fit clearly", () => {
  const gb = 1024 * 1024 * 1024;
  assert.equal(modelFitLabel(2 * gb, 16), "Fits easily");
  assert.equal(modelFitLabel(4 * gb, 8), "Tight fit");
  assert.equal(modelFitLabel(8 * gb, 8), "Heavy for your RAM");
  assert.equal(modelFitLabel(0, 16), "");
  assert.equal(modelFitLabel(2 * gb, 16, 8192), "Fits on graphics");
  assert.equal(modelFitLabel(4 * gb, 16, 3072), "Tight on graphics");
  assert.equal(modelFitLabel(8 * gb, 32, 2048), "Fits easily");
});

test("formats live context window meter and estimates session tokens", () => {
  assert.equal(formatContextMeter(0, 8192), null);
  assert.equal(formatContextMeter(1200, 0), null);
  assert.deepEqual(formatContextMeter(2400, 8192), {
    label: "2.4K / 8.2K (29%)",
    percent: 29,
    isHigh: false,
  });
  assert.deepEqual(formatContextMeter(13000, 16384), {
    label: "13K / 16K (79%)",
    percent: 79,
    isHigh: true,
  });

  assert.equal(estimateSessionTokens([]), 0);
  const est = estimateSessionTokens([
    { role: "user", content: "Hello world".repeat(40), attachments: [{ text: "abc".repeat(100) }] },
    { role: "assistant", content: "Sure thing".repeat(20) },
  ]);
  assert.ok(est > 150 && est < 400, `unexpected estimate ${est}`);
});

test("reuses an untouched empty session with matching preset, folder, and project", () => {
  const current = {
    id: "cur-empty",
    app_preset: "chat",
    workspace_path: null,
    project_id: null,
    messages: [],
    staged_edits: [],
  };
  assert.equal(
    findReusableEmptySession([], { currentSession: current, preset: "chat" }),
    "cur-empty",
  );
  assert.equal(
    findReusableEmptySession(
      [
        { id: "used", app_preset: "work", workspace_path: "/p", project_id: null, message_count: 2 },
        { id: "empty-work", app_preset: "work", workspace_path: "/p", project_id: null, message_count: 0 },
      ],
      { currentSession: current, preset: "work", workspacePath: "/p" },
    ),
    "empty-work",
  );
  assert.equal(
    findReusableEmptySession(
      [{ id: "pinned-empty", app_preset: "chat", workspace_path: null, project_id: null, message_count: 0, pinned: true }],
      { currentSession: { ...current, messages: [{ id: "m1" }] }, preset: "chat" },
    ),
    null,
  );
});

test("picks initial session matching startup workspace when provided", () => {
  const sessions = [
    { id: "recent-other", app_preset: "general", workspace_path: "/other" },
    { id: "ws-chat", app_preset: "chat", workspace_path: "/target" },
    { id: "ws-work", app_preset: "general", workspace_path: "/target" },
  ];
  assert.equal(pickInitialSession(sessions, null), "recent-other");
  assert.equal(pickInitialSession(sessions, "/target"), "ws-work");
  assert.equal(pickInitialSession(sessions, "/new-folder"), null);
});

test("preserves speed and token counts in live telemetry status line", () => {
  assert.equal(
    formatLiveTelemetry({
      location: "On this computer",
      messageCount: 4,
      took: "2.3s",
      tokensPerSec: 34.56,
      completionTokens: 120,
    }),
    "On this computer · 4 messages · 2.3s · 34.6 tok/s · 120 tokens",
  );
  assert.equal(
    formatLiveTelemetry({
      location: "Leaves this computer",
      messageCount: 2,
    }),
    "Leaves this computer · 2 messages",
  );
});
