import assert from "node:assert/strict";
import test from "node:test";
import {
  clipFractions,
  clipIndexAt,
  nextSceneStart,
  planToStoryboardText,
  previousSceneStart,
} from "../video.js";

const CLIPS = [
  { image_path: "assets/a.png", narration: "First scene.", overlay: "HOOK", motion: "slow_zoom", start: 0, duration: 2.5 },
  { image_path: "assets/b.png", narration: "Second scene.", overlay: null, motion: "pan_left", start: 2.5, duration: 5.0 },
  { image_path: "assets/c.png", narration: "Final scene.", overlay: "CTA", motion: "none", start: 7.5, duration: 2.5 },
];

test("timeline helpers match Keel Mobile scene-jump navigation", () => {
  assert.equal(clipIndexAt(CLIPS, 0), 0);
  assert.equal(clipIndexAt(CLIPS, 2.5), 1);
  assert.equal(clipIndexAt(CLIPS, 99), 2);
  assert.equal(previousSceneStart(CLIPS, 3.0, 1.0), 0);
  assert.equal(previousSceneStart(CLIPS, 4.2, 1.0), 2.5);
  assert.equal(nextSceneStart(CLIPS, 1.0), 2.5);
  assert.equal(nextSceneStart(CLIPS, 8.0), null);
  const fracs = clipFractions(CLIPS);
  assert.deepEqual(fracs, [0.25, 0.5, 0.25]);
});

test("planToStoryboardText serializes scenes for iterative re-renders", () => {
  const text = planToStoryboardText({ clips: CLIPS });
  assert.match(text, /image: assets\/a\.png/);
  assert.match(text, /overlay: HOOK/);
  assert.match(text, /motion: slow_zoom/);
  assert.match(text, /motion: pan_left/);
  assert.doesNotMatch(text, /motion: none/);
});
