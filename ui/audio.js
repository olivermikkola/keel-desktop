/** Settings audio try-run helpers. Pack install / runners still live in keel-core. */

/** Short sample spoken by Try TTS (no model download). */
export const AUDIO_TRY_TTS_SAMPLE = "Hello from Keel Desktop.";

/** Documented STT input: local file via dialog — never a bundled large model. */
export const AUDIO_TRY_STT_INPUT_HELP =
  "Try STT opens a file dialog for a local audio clip (wav, mp3, flac, ogg, or m4a). Nothing is downloaded.";

/** Documented TTS input for Settings try-run. */
export const AUDIO_TRY_TTS_INPUT_HELP = `Try TTS speaks “${AUDIO_TRY_TTS_SAMPLE}” into a wav under the app data directory, then lists it with other audio artifacts.`;

/**
 * Hub badge copy. Audio is optional Settings packs/try-run — not a chat-mode product.
 * @param {{ id: string, state: string }} cap
 */
export function capabilityBadgeLabel(cap) {
  if (cap?.state === "planned") return "Planned";
  if (cap?.id === "images" || cap?.id === "audio" || cap?.id === "video") return "Optional";
  return "Built in";
}

/** @param {string} transcript */
export function formatAudioTrySttSuccess(transcript) {
  const preview = String(transcript ?? "")
    .trim()
    .replace(/\s+/g, " ");
  if (!preview) return "STT finished (empty transcript).";
  const short = preview.length > 160 ? `${preview.slice(0, 160)}…` : preview;
  return `STT ok: ${short}`;
}

/** @param {string} [outputPath] */
export function formatAudioTryTtsSuccess(outputPath) {
  const path = String(outputPath ?? "").trim();
  return path ? `TTS ok — wrote ${path}` : "TTS ok — check Recent audio artifacts.";
}
