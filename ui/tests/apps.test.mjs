import assert from "node:assert/strict";
import test from "node:test";
import { CAPABILITIES, EVERYDAY_APPS } from "../images.js";

test("Apps hub lists everyday writing, data, and study apps with plain descriptions", () => {
  const ids = EVERYDAY_APPS.map((a) => a.id);
  assert.deepEqual(ids, ["websites", "sheets", "docs", "study", "slides"]);
  for (const app of EVERYDAY_APPS) {
    assert.ok(app.title.length > 0);
    assert.ok(app.summary.length > 15);
    assert.ok(app.example.length > 5);
    for (const jargon of ["SQL", "CSV", "Markdown", "HTML", "GGUF", "Safetensors", "loopback"]) {
      assert.ok(
        !app.title.includes(jargon) && !app.summary.includes(jargon),
        `${app.id} should stay jargon-free (found ${jargon})`,
      );
    }
  }
});

test("Create cards avoid developer jargon on the Apps hub", () => {
  for (const cap of CAPABILITIES) {
    for (const jargon of ["AUTOMATIC1111", "GGUF", "Safetensors", "loopback", "Ken Burns", "FFmpeg"]) {
      assert.ok(
        !cap.detail.includes(jargon) && !cap.title.includes(jargon),
        `capability ${cap.id} should avoid jargon (${jargon})`,
      );
    }
  }
});
