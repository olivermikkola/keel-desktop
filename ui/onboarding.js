// Pure helpers for first-run: download progress, plain-language errors, and
// the "wake the assistant on launch" decision. No DOM access, so they run in `node --test`.
import { formatBytes } from "./state.js";

const SAMPLE_WINDOW_MS = 6000;

export function formatEta(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  if (seconds < 45) return "less than a minute";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `about ${Math.max(minutes, 1)} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `about ${hours} h ${rest} min` : `about ${hours} h`;
}

export function formatSpeed(bytesPerSecond) {
  return Number.isFinite(bytesPerSecond) && bytesPerSecond > 0 ? `${formatBytes(bytesPerSecond)}/s` : "";
}

/**
 * Combines the runner and model downloads into one bar with a speed and a time left.
 * `expected` seeds the totals from the setup plan so the bar is honest before the first byte.
 */
export function createProgressTracker(expected = {}) {
  const parts = new Map();
  for (const [id, total] of Object.entries(expected)) parts.set(id, { downloaded: 0, total: total || 0, done: total === 0 });
  let samples = [];
  return {
    update(progress, nowMs = Date.now()) {
      const previous = parts.get(progress.model_id) || { downloaded: 0, total: 0, done: false };
      const total = progress.total_bytes || previous.total || 0;
      parts.set(progress.model_id, {
        downloaded: progress.done ? Math.max(total, progress.downloaded_bytes) : progress.downloaded_bytes,
        total: progress.done ? Math.max(total, progress.downloaded_bytes) : total,
        done: Boolean(progress.done),
      });
      const sum = [...parts.values()].reduce((n, p) => n + p.downloaded, 0);
      samples.push({ t: nowMs, sum });
      samples = samples.filter((s) => nowMs - s.t <= SAMPLE_WINDOW_MS);
    },
    snapshot() {
      const values = [...parts.values()];
      const downloaded = values.reduce((n, p) => n + p.downloaded, 0);
      const total = values.reduce((n, p) => n + Math.max(p.total, p.downloaded), 0);
      let bytesPerSecond = 0;
      if (samples.length >= 2) {
        const first = samples[0];
        const last = samples[samples.length - 1];
        const seconds = (last.t - first.t) / 1000;
        if (seconds >= 1) bytesPerSecond = Math.max(0, (last.sum - first.sum) / seconds);
      }
      const remaining = Math.max(0, total - downloaded);
      return {
        downloaded,
        total,
        fraction: total > 0 ? Math.min(1, downloaded / total) : 0,
        bytesPerSecond,
        etaSeconds: bytesPerSecond > 0 ? remaining / bytesPerSecond : null,
      };
    },
  };
}

export const SETUP_STEPS = [
  { id: "runner", label: "Getting Keel's engine" },
  { id: "model", label: "Downloading your AI" },
  { id: "start", label: "Waking it up" },
];

/** Marks each step done, active or waiting for the current stage. */
export function stepStates(stage) {
  const index = SETUP_STEPS.findIndex((s) => s.id === stage);
  return SETUP_STEPS.map((step, i) => ({ ...step, state: index < 0 ? "waiting" : i < index ? "done" : i === index ? "active" : "waiting" }));
}

/** Turns a raw backend error into something a non-technical person can act on. */
export function friendlySetupError(raw) {
  const text = String(raw ?? "");
  const detail = text.replace(/^Error:\s*/i, "").slice(0, 300);
  if (/no space|disk (is )?full|ENOSPC|not enough (disk )?space/i.test(text)) {
    return { message: "Your disk is full, so the download couldn't finish. Free up a few GB, then try again.", retry: true, existing: false, detail };
  }
  if (/needs a system library/i.test(text)) {
    return { message: detail, retry: true, existing: false, detail: "" };
  }
  if (/sha-?256|checksum|did not match|mismatch/i.test(text)) {
    return { message: "The download didn't pass Keel's safety check, so it was thrown away. Try again to get a clean copy.", retry: true, existing: false, detail };
  }
  if (/not available for this platform|Automatic setup is not available/i.test(text)) {
    return { message: "Automatic setup isn't available on this computer yet. If you already use Ollama or LM Studio, connect it instead.", retry: false, existing: true, detail };
  }
  if (/network|dns|resolve|connect|timed? ?out|error sending request|offline|unreachable|tls|certificate/i.test(text)) {
    return { message: "Keel couldn't reach the download server. Check your internet connection, then try again. Anything already downloaded is kept.", retry: true, existing: false, detail };
  }
  return { message: "Something went wrong while setting up. Trying again usually fixes it.", retry: true, existing: false, detail };
}

/** Which installed model to start automatically when the app opens, or null. */
export function modelToWake(bootstrap) {
  if (!bootstrap || bootstrap.runtime?.state !== "stopped" || bootstrap.config?.external_base_url) return null;
  const wanted = bootstrap.config?.selected_model_id;
  const models = bootstrap.models || [];
  const chosen = models.find((m) => m.id === wanted && m.installed) || models.find((m) => m.installed && m.tier !== "desktop");
  return chosen ? chosen.id : null;
}

/** First-run means: no flag, nothing installed, nothing connected. */
export function isFirstRun(storageValue, bootstrap) {
  if (storageValue === "1") return false;
  if (!bootstrap) return true;
  if (["running", "external"].includes(bootstrap.runtime?.state)) return false;
  return !(bootstrap.models || []).some((m) => m.installed);
}

export function assistantStatus({ runtime, waking, setup }) {
  if (setup?.busy) return { key: "setup", label: `Setting up… ${Math.round((setup.fraction || 0) * 100)}%` };
  if (setup?.paused) return { key: "paused", label: "Download paused" };
  if (waking) return { key: "waking", label: "Waking up your AI…" };
  switch (runtime?.state) {
    case "running": return { key: "ready", label: "Ready" };
    case "external": return { key: "ready", label: "Connected" };
    case "error": return { key: "error", label: "Needs attention" };
    default: return { key: "off", label: "Off" };
  }
}
