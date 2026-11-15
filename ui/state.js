export const THEME_PALETTES = [
  { id: "original", label: "Original", ink: "#0c1415", surface: "#141f21", accent: "#a4cfc0", rust: "#c77851" },
  { id: "jade", label: "Jade", ink: "#0a1714", surface: "#11241f", accent: "#88d4b8", rust: "#d98a5f" },
  { id: "pine", label: "Pine", ink: "#0d1612", surface: "#15231c", accent: "#9cc5a1", rust: "#c88a4b" },
  { id: "brass", label: "Brass", ink: "#141310", surface: "#1e1c17", accent: "#d8b982", rust: "#c96e4b" },
  { id: "graphite", label: "Graphite", ink: "#111315", surface: "#1b1e22", accent: "#c5ced8", rust: "#d27a57" },
];

export const SPACING_MODES = ["tight", "regular", "airy"];
export const TEXT_SIZES = ["small", "regular", "large"];

export const APPEARANCE_DEFAULTS = {
  palette: "original",
  spacing: "regular",
  textSize: "regular",
};

export function normalizeAppearance(raw = {}) {
  const paletteIds = new Set(THEME_PALETTES.map((p) => p.id));
  return {
    palette: paletteIds.has(raw?.palette) ? raw.palette : APPEARANCE_DEFAULTS.palette,
    spacing: SPACING_MODES.includes(raw?.spacing) ? raw.spacing : APPEARANCE_DEFAULTS.spacing,
    textSize: TEXT_SIZES.includes(raw?.textSize) ? raw.textSize : APPEARANCE_DEFAULTS.textSize,
  };
}

export function computeWelcomeVisible(storageValue) {
  return storageValue !== "1";
}

export function formatRelativeTime(epochSecs, nowSecs = Math.floor(Date.now() / 1000)) {
  if (!Number.isFinite(epochSecs) || epochSecs <= 0) return "now";
  const diff = Math.max(0, Math.floor(nowSecs - epochSecs));
  if (diff < 60) return "now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  const days = Math.floor(diff / 86400);
  if (days < 30) return `${days}d`;
  return `${Math.floor(days / 30)}mo`;
}

export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let val = bytes;
  let idx = 0;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx += 1;
  }
  return `${val >= 10 || idx === 0 ? val.toFixed(0) : val.toFixed(1)} ${units[idx]}`;
}

export function countStagedEdits(edits = []) {
  let applied = 0;
  let pending = 0;
  let discarded = 0;
  for (const edit of edits) {
    if (edit.status === "applied") applied += 1;
    else if (edit.status === "pending") pending += 1;
    else if (edit.status === "discarded") discarded += 1;
  }
  return { applied, pending, discarded };
}

export function shortPath(fullPath) {
  if (!fullPath) return "No folder";
  const parts = fullPath.replace(/\\/g, "/").split("/").filter(Boolean);
  if (parts.length <= 2) return fullPath;
  return `.../${parts.slice(-2).join("/")}`;
}

export function detectViewerForPreset(preset, files = []) {
  const onlyFiles = files.filter((f) => !f.endsWith("/"));
  if (preset === "websites") {
    return (
      onlyFiles.find((f) => f === "index.html") ||
      onlyFiles.find((f) => f.endsWith(".html")) ||
      null
    );
  }
  if (preset === "slides") {
    return (
      onlyFiles.find((f) => f === "slides.html") ||
      onlyFiles.find((f) => f.endsWith(".html")) ||
      null
    );
  }
  if (preset === "sheets") {
    return onlyFiles.find((f) => f.toLowerCase().endsWith(".csv")) || null;
  }
  if (preset === "docs" || preset === "study") {
    return onlyFiles.find((f) => f.toLowerCase().endsWith(".md")) || null;
  }
  return (
    onlyFiles.find((f) => f === "index.html" || f === "slides.html") ||
    onlyFiles.find((f) => f.toLowerCase().endsWith(".md")) ||
    onlyFiles.find((f) => f.toLowerCase().endsWith(".csv")) ||
    null
  );
}
