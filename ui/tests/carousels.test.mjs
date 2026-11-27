import assert from "node:assert/strict";
import test from "node:test";
import {
  defaultCarouselSlides,
  isScreenshotPath,
  moveSlidePhoto,
  slugifySlideName,
} from "../carousels.js";
import { CAPABILITIES } from "../images.js";

test("Photo Slides helpers reorder photos, detect screenshots, and build cover/photo/closing slides", () => {
  assert.deepEqual(moveSlidePhoto(["a", "b", "c"], 0, 1), ["b", "a", "c"]);
  assert.deepEqual(moveSlidePhoto(["a", "b", "c"], 0, -1), ["a", "b", "c"]);
  assert.equal(slugifySlideName("Weekend Trip to Porvoo!"), "photo-slides/weekend-trip-to-porvoo.html");
  assert.equal(slugifySlideName("   "), "photo-slides/my-photo-slides.html");
  assert.equal(isScreenshotPath("assets/Screenshot_2026.png"), true);
  assert.equal(isScreenshotPath("assets/sunset.jpg"), false);

  const slides = defaultCarouselSlides([
    { path: "assets/Screenshot_1.png", screenshot: true },
    { path: "assets/dog.jpg", screenshot: false },
  ]);
  assert.equal(slides.length, 4);
  assert.equal(slides[0].kind, "cover");
  assert.equal(slides[0].style, "screenshot");
  assert.equal(slides[1].kind, "photo");
  assert.equal(slides[2].kind, "photo");
  assert.equal(slides[2].style, "photo");
  assert.equal(slides[3].kind, "closing");
  assert.equal(slides[3].photo_path, "assets/dog.jpg");
});

test("Apps hub exposes Photo Slides in plain language", () => {
  const card = CAPABILITIES.find((c) => c.id === "carousels");
  assert.ok(card, "Photo Slides card missing from CAPABILITIES");
  assert.equal(card.title, "Photo Slides");
  assert.match(card.detail, /swipeable photo slides/i);
});
