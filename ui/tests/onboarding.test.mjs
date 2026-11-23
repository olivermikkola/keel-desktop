import assert from "node:assert/strict";
import test from "node:test";
import { assistantStatus, createProgressTracker, formatEta, formatSpeed, friendlyRuntimeError, friendlySetupError, isFirstRun, isRemoteModelServer, modelLocation, modelTierLabel, modelToWake, serverDataNotice, speedSettingsCopy, answerTimePhrase, modelCheckIntro, stepStates } from "../onboarding.js";

const MB = 1024 * 1024;

test("progress combines the runner and the model into one bar with speed and time left", () => {
  const tracker = createProgressTracker({ runtime: 20 * MB, big: 980 * MB });
  tracker.update({ model_id: "runtime", downloaded_bytes: 20 * MB, total_bytes: 20 * MB, done: true }, 0);
  tracker.update({ model_id: "big", downloaded_bytes: 100 * MB, total_bytes: 980 * MB, done: false }, 1000);
  tracker.update({ model_id: "big", downloaded_bytes: 300 * MB, total_bytes: 980 * MB, done: false }, 3000);
  const s = tracker.snapshot();
  assert.equal(s.total, 1000 * MB);
  assert.equal(s.downloaded, 320 * MB);
  assert.ok(Math.abs(s.fraction - 0.32) < 1e-9);
  assert.ok(Math.abs(s.bytesPerSecond - 100 * MB) < 1);
  assert.ok(Math.abs(s.etaSeconds - 6.8) < 0.01);
});

test("progress is honest before the first byte and when a download resumes", () => {
  const tracker = createProgressTracker({ runtime: 0, big: 1000 });
  assert.deepEqual(tracker.snapshot(), { downloaded: 0, total: 1000, fraction: 0, bytesPerSecond: 0, etaSeconds: null });
  // A resumed download starts mid-file; the jump is not counted as speed.
  tracker.update({ model_id: "big", downloaded_bytes: 600, total_bytes: 1000, done: false }, 0);
  assert.equal(tracker.snapshot().bytesPerSecond, 0);
  assert.equal(tracker.snapshot().fraction, 0.6);
  tracker.update({ model_id: "big", downloaded_bytes: 1000, total_bytes: 1000, done: true }, 2000);
  assert.equal(tracker.snapshot().fraction, 1);
});

test("old samples drop out so a stalled download stops claiming a speed", () => {
  const tracker = createProgressTracker({ big: 1000 });
  tracker.update({ model_id: "big", downloaded_bytes: 0, total_bytes: 1000, done: false }, 0);
  tracker.update({ model_id: "big", downloaded_bytes: 500, total_bytes: 1000, done: false }, 2000);
  assert.ok(tracker.snapshot().bytesPerSecond > 0);
  tracker.update({ model_id: "big", downloaded_bytes: 500, total_bytes: 1000, done: false }, 60000);
  assert.equal(tracker.snapshot().bytesPerSecond, 0);
});

test("times and speeds read like a person would say them", () => {
  assert.equal(formatEta(10), "less than a minute");
  assert.equal(formatEta(90), "about 2 min");
  assert.equal(formatEta(3600 * 1.5), "about 1 h 30 min");
  assert.equal(formatEta(null), "");
  assert.equal(formatSpeed(5 * MB), "5.0 MB/s");
  assert.equal(formatSpeed(0), "");
});

test("setup steps show what is done, what is happening and what is next", () => {
  assert.deepEqual(stepStates("model").map((s) => s.state), ["done", "active", "waiting"]);
  assert.deepEqual(stepStates("start").map((s) => s.state), ["done", "done", "active"]);
  assert.deepEqual(stepStates("nonsense").map((s) => s.state), ["waiting", "waiting", "waiting"]);
});

test("setup errors become plain advice and keep the raw text for the details drawer", () => {
  const offline = friendlySetupError("error sending request for url (https://huggingface.co/x): dns error");
  assert.match(offline.message, /internet connection/);
  assert.ok(offline.retry && !offline.existing);
  assert.match(offline.detail, /huggingface/);
  assert.doesNotMatch(offline.message, /huggingface|dns/);
  assert.match(friendlySetupError("No space left on device (os error 28)").message, /disk is full/);
  assert.match(friendlySetupError("SHA-256 mismatch for bonsai-9b: expected a, got b").message, /safety check/);
  const unsupported = friendlySetupError("Automatic setup is not available for this platform yet.");
  assert.ok(unsupported.existing && !unsupported.retry);
  assert.match(friendlySetupError("???").message, /Something went wrong/);
});

test("the installed model starts by itself, but only when that is safe and unambiguous", () => {
  const base = { runtime: { state: "stopped" }, config: { selected_model_id: "b" }, models: [{ id: "a", installed: true }, { id: "b", installed: true }] };
  assert.equal(modelToWake(base), "b");
  assert.equal(modelToWake({ ...base, config: { selected_model_id: "missing" } }), "a");
  assert.equal(modelToWake({ ...base, runtime: { state: "running" } }), null);
  assert.equal(modelToWake({ ...base, config: { selected_model_id: "b", external_base_url: "http://127.0.0.1:11434/v1" } }), null);
  assert.equal(modelToWake({ ...base, models: [{ id: "b", installed: false }] }), null);
  assert.equal(modelToWake(null), null);
});

test("first run means nothing is set up yet, not merely a missing flag", () => {
  const empty = { runtime: { state: "stopped" }, models: [{ id: "a", installed: false }] };
  assert.equal(isFirstRun(null, empty), true);
  assert.equal(isFirstRun("1", empty), false);
  assert.equal(isFirstRun(null, { ...empty, models: [{ id: "a", installed: true }] }), false);
  assert.equal(isFirstRun(null, { ...empty, runtime: { state: "external" } }), false);
});

test("the status label is one plain word or phrase", () => {
  assert.equal(assistantStatus({ runtime: { state: "running" } }).label, "Ready");
  assert.equal(assistantStatus({ runtime: { state: "external" } }).label, "Connected");
  assert.equal(assistantStatus({ runtime: { state: "stopped" } }).label, "Off");
  assert.equal(assistantStatus({ runtime: { state: "error" } }).label, "Needs attention");
  assert.equal(assistantStatus({ runtime: { state: "stopped" }, waking: true }).label, "Waking up your AI…");
  assert.equal(assistantStatus({ runtime: { state: "stopped" }, setup: { busy: true, fraction: 0.426 } }).label, "Setting up… 43%");
});

test("a missing system library shows the exact fix command, not generic advice", () => {
  const raw = "Keel's engine needs a system library this computer doesn't have yet (libssl.so.3). Open a terminal, run: sudo dnf install openssl3-libs — then press Try again.";
  const e = friendlySetupError(raw);
  assert.match(e.message, /sudo dnf install openssl3-libs/);
  assert.equal(e.retry, true);
});

test("modelLocation and serverDataNotice distinguish on-device, local server, and remote endpoints", () => {
  assert.equal(modelLocation({ state: "running", base_url: "http://127.0.0.1:18180/v1" }), "On this computer");
  assert.equal(modelLocation({ state: "external", base_url: "http://127.0.0.1:11434/v1" }), "Local model server");
  assert.equal(modelLocation({ state: "external", base_url: "http://localhost:8080/v1" }), "Local model server");
  assert.equal(modelLocation({ state: "external", base_url: "http://[::1]:8080/v1" }), "Local model server");
  assert.equal(modelLocation({ state: "external", base_url: "https://models.example.com/v1" }), "Leaves this computer");
  assert.equal(modelLocation({ state: "external", base_url: "not-a-url" }), "Leaves this computer");
  assert.equal(isRemoteModelServer("https://models.example.com/v1"), true);
  assert.equal(isRemoteModelServer("http://127.0.0.1:11434/v1"), false);
  assert.match(serverDataNotice("http://127.0.0.1:11434/v1"), /on this computer/);
  assert.match(serverDataNotice("https://models.example.com/v1"), /Leaves this computer/);
  assert.match(serverDataNotice("https://models.example.com/v1"), /may incur provider cost/);
});

test("friendlyRuntimeError strips raw HTTP endpoints, ports, and engine jargon", () => {
  const raw = "Runtime error: Could not reach model endpoint http://127.0.0.1:18180/v1/chat/completions";
  const cleaned = friendlyRuntimeError(raw);
  assert.doesNotMatch(cleaned, /127\.0\.0\.1|18180|\/v1\/chat\/completions|Runtime error/i);
  assert.match(cleaned, /stopped responding/);

  const runner = friendlyRuntimeError("Model runner exited (exit status: 1) llama-server");
  assert.doesNotMatch(runner, /llama-server|exit status/i);
  assert.match(runner, /lighter model/);

  const oom = friendlyRuntimeError("ggml: failed to allocate memory (out of memory)");
  assert.match(oom, /enough free memory/);

  const lib = "Keel's engine needs a system library this computer doesn't have yet (libssl.so.3). Open a terminal, run: sudo dnf install openssl3-libs — then press Try again.";
  assert.equal(friendlyRuntimeError(lib), lib);
});

test("modelToWake skips damaged models and recovers from error state", () => {
  const models = [
    { id: "bonsai-9b", installed: false, damaged: true, tier: "balanced" },
    { id: "bonsai-4b", installed: true, damaged: false, tier: "fast" },
  ];
  assert.equal(
    modelToWake({ runtime: { state: "error" }, config: { selected_model_id: "bonsai-9b" }, models }),
    "bonsai-4b",
  );
  assert.equal(
    modelToWake({ runtime: { state: "stopped" }, config: { selected_model_id: "bonsai-9b" }, models: [models[0]] }),
    null,
  );
  assert.equal(modelTierLabel({ tier: "fast" }), "Fast");
  assert.equal(modelTierLabel({ tier: "balanced" }), "Recommended");
  assert.equal(modelTierLabel({ id: "bonsai-4b", tier: "balanced" }, "bonsai-4b"), "Good fit");
  assert.equal(modelTierLabel({ id: "bonsai-9b", tier: "balanced", checkPassed: false }, "bonsai-9b"), "Didn't pass the check");
  assert.equal(modelTierLabel({ tier: "desktop" }), "Most capable");
  assert.equal(modelTierLabel({ tier: "balanced", damaged: true }), "Needs repair");
  assert.equal(
    modelToWake({
      runtime: { state: "stopped" },
      config: { selected_model_id: "bonsai-9b" },
      offered_model_id: "bonsai-4b",
      model_checks: { "bonsai-9b": { passed: false } },
      models: [
        { id: "bonsai-9b", installed: true, damaged: false, tier: "balanced" },
        { id: "bonsai-4b", installed: true, damaged: false, tier: "fast" },
      ],
    }),
    "bonsai-4b",
  );
});

test("answer time and the model check speak in everyday words", () => {
  assert.equal(answerTimePhrase(0), "");
  assert.equal(answerTimePhrase(1.2), "last answer took 1 second");
  assert.equal(answerTimePhrase(12.4), "last answer took 12 seconds");
  const copy = speedSettingsCopy();
  assert.match(copy.where, /Where should the model run/);
  assert.equal(copy.whereOptions[1][1], "Graphics chip");
  assert.match(modelCheckIntro(), /saved preference/);
  assert.doesNotMatch(modelCheckIntro(), /GPU|token|llama/i);
});
