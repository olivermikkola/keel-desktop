import assert from "node:assert/strict";
import test from "node:test";
import { modeForPreset, promptBlockReason, isRuntimeLive } from "../modes.js";
import { CAPABILITIES } from "../images.js";

test("restores legacy and specialized work sessions to their correct modes", () => {
  assert.equal(modeForPreset("general"), "work");
  for (const preset of ["docs", "websites", "slides", "sheets", "study"]) assert.equal(modeForPreset(preset), "work");
  assert.equal(modeForPreset("chat"), "chat");
  assert.equal(modeForPreset("code"), "code");
});

test("Chat works without a folder, Work and Code require one, and stopped models cannot send", () => {
  const running = { state: "running" };
  assert.equal(promptBlockReason({ app_preset: "chat" }, running, null), null);
  assert.match(promptBlockReason({ app_preset: "general" }, running, null), /folder/);
  assert.match(promptBlockReason({ app_preset: "code" }, running, null), /folder/);
  assert.match(promptBlockReason({ app_preset: "chat" }, { state: "stopped" }, null), /model/);
  assert.equal(promptBlockReason({ app_preset: "code" }, { state: "stopped" }, "/project", "opencode"), null);
  assert.match(promptBlockReason({ app_preset: "chat" }, { state: "stopped" }, "/project", "opencode"), /model/);
  assert.ok(isRuntimeLive({ state: "running" }));
  assert.ok(isRuntimeLive({ state: "external" }));
  assert.ok(!isRuntimeLive({ state: "stopped" }));
});

test("capability library marks Settings audio packs/try-run and Short Video available", () => {
  assert.equal(CAPABILITIES.find((c) => c.id === "images").state, "available");
  const audio = CAPABILITIES.find((c) => c.id === "audio");
  assert.equal(audio.state, "available");
  assert.match(audio.detail, /Try STT|Try TTS|Settings packs/i);
  assert.match(audio.detail, /Not a chat-mode audio product/i);
  const video = CAPABILITIES.find((c) => c.id === "video");
  assert.equal(video.state, "available");
  assert.match(video.detail, /narrated video/i);
});
