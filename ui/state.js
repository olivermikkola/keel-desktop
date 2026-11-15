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
