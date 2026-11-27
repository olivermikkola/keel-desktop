import assert from "node:assert/strict";
import test from "node:test";
import {
  AUDIO_TRY_STT_INPUT_HELP,
  AUDIO_TRY_TTS_INPUT_HELP,
  AUDIO_TRY_TTS_SAMPLE,
  capabilityBadgeLabel,
  formatAudioTrySttSuccess,
  formatAudioTryTtsSuccess,
} from "../audio.js";
import { CAPABILITIES } from "../images.js";

test("Try STT documents a local file dialog, not a model download", () => {
  assert.match(AUDIO_TRY_STT_INPUT_HELP, /file dialog/i);
  assert.match(AUDIO_TRY_STT_INPUT_HELP, /wav|mp3|flac/i);
  assert.match(AUDIO_TRY_STT_INPUT_HELP, /Nothing is downloaded/i);
});

test("Try TTS uses a short sample line and writes under app data", () => {
  assert.ok(AUDIO_TRY_TTS_SAMPLE.length > 0);
  assert.ok(AUDIO_TRY_TTS_SAMPLE.length < 80);
  assert.match(
    AUDIO_TRY_TTS_INPUT_HELP,
    new RegExp(AUDIO_TRY_TTS_SAMPLE.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")),
  );
  assert.match(AUDIO_TRY_TTS_INPUT_HELP, /audio artifacts/i);
});

test("try-run success copy stays short and path-aware", () => {
  assert.match(formatAudioTrySttSuccess("  hello   world  "), /STT ok: hello world/);
  assert.match(formatAudioTryTtsSuccess("/tmp/out.wav"), /TTS ok.*\/tmp\/out\.wav/);
  assert.match(formatAudioTryTtsSuccess(""), /artifacts/i);
});

test("hub badges: audio and short video are Optional", () => {
  const audio = CAPABILITIES.find((c) => c.id === "audio");
  const video = CAPABILITIES.find((c) => c.id === "video");
  assert.equal(audio.state, "available");
  assert.equal(capabilityBadgeLabel(audio), "Optional");
  assert.match(audio.detail, /Try STT|Try TTS|Settings packs/i);
  assert.match(audio.detail, /Not a chat-mode audio product/i);
  assert.equal(video.state, "available");
  assert.equal(capabilityBadgeLabel(video), "Optional");
  assert.equal(capabilityBadgeLabel({ id: "video", state: "planned" }), "Planned");
  assert.equal(capabilityBadgeLabel({ id: "text", state: "available" }), "Built in");
  assert.equal(capabilityBadgeLabel({ id: "images", state: "available" }), "Optional");
});
