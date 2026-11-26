export const ACCENT_MODES = ["sage", "copper", "ice"];
export const TONE_MODES = ["slate", "ink", "paper"];
export const DENSITY_MODES = ["cozy", "compact"];
export const TEXT_SIZES = ["small", "regular", "large"];

export const APPEARANCE_DEFAULTS = {
  accent: "sage",
  tone: "ink",
  density: "cozy",
  sidebar: true,
  textSize: "regular",
};

export function normalizeAppearance(raw = {}) {
  return {
    accent: ACCENT_MODES.includes(raw?.accent) ? raw.accent : APPEARANCE_DEFAULTS.accent,
    tone: TONE_MODES.includes(raw?.tone) ? raw.tone : APPEARANCE_DEFAULTS.tone,
    density: DENSITY_MODES.includes(raw?.density) ? raw.density : APPEARANCE_DEFAULTS.density,
    sidebar: typeof raw?.sidebar === "boolean" ? raw.sidebar : APPEARANCE_DEFAULTS.sidebar,
    textSize: TEXT_SIZES.includes(raw?.textSize) ? raw.textSize : APPEARANCE_DEFAULTS.textSize,
  };
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

export function parseDiffLines(diffText = "") {
  if (!diffText) return [];
  return diffText.split("\n").map((line) => {
    if (line.startsWith("--- ") || line.startsWith("+++ ") || line.startsWith("@@")) {
      return { kind: "meta", text: line };
    }
    if (line.startsWith("+")) {
      return { kind: "add", text: line };
    }
    if (line.startsWith("-")) {
      return { kind: "del", text: line };
    }
    return { kind: "ctx", text: line };
  });
}

export function filterModels(models = [], query = "", filter = "all") {
  const q = query.trim().toLowerCase();
  return models.filter((m) => {
    if (filter === "installed" && !m.installed) return false;
    if (filter === "gguf" && m.backend && m.backend !== "gguf") return false;
    if (!q) return true;
    const hay = `${m.name || ""} ${m.id || ""} ${m.detail || ""} ${m.quantization || ""}`.toLowerCase();
    return hay.includes(q);
  });
}

export function modelFitLabel(modelBytes, systemRamGb, vramTotalMib = 0) {
  if (!Number.isFinite(modelBytes) || modelBytes <= 0) {
    return "";
  }
  const modelGb = modelBytes / (1024 * 1024 * 1024);
  if (Number.isFinite(vramTotalMib) && vramTotalMib > 0) {
    const vramGb = vramTotalMib / 1024;
    // Mirror hardware::model_fits_entirely cushion (weights + ~25% + margin).
    if (modelGb * 1.25 + 0.25 <= vramGb) return "Fits on graphics";
    if (modelGb * 0.5 <= vramGb) return "Tight on graphics";
  }
  if (!Number.isFinite(systemRamGb) || systemRamGb <= 0) {
    return "";
  }
  if (modelGb * 2.2 <= systemRamGb) return "Fits easily";
  if (modelGb * 1.4 <= systemRamGb) return "Tight fit";
  return "Heavy for your RAM";
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

export function formatTokenCount(count) {
  const n = Math.max(0, Math.round(Number(count) || 0));
  if (n >= 1000) {
    const k = n / 1000;
    if (k >= 10 || Math.abs(k - Math.round(k)) < 0.05) {
      return `${Math.round(k)}K`;
    }
    return `${k.toFixed(1)}K`;
  }
  return String(n);
}

export function formatContextMeter(usedTokens, contextSize) {
  const used = Math.max(0, Math.round(Number(usedTokens) || 0));
  const cap = Math.max(0, Math.round(Number(contextSize) || 0));
  if (used <= 0 || cap <= 0) return null;
  const fraction = Math.min(1, used / cap);
  const percent = Math.round(fraction * 100);
  return {
    label: `${formatTokenCount(used)} / ${formatTokenCount(cap)} (${percent}%)`,
    percent,
    isHigh: fraction >= 0.75,
  };
}

export function estimateSessionTokens(messages = []) {
  if (!Array.isArray(messages) || messages.length === 0) return 0;
  let chars = 0;
  let images = 0;
  for (const m of messages) {
    chars += (m?.content || "").length + (m?.thinking || "").length;
    for (const tc of m?.tool_calls || []) {
      chars += (tc?.result || "").length + 48;
    }
    for (const doc of m?.attachments || []) {
      chars += Math.min((doc?.text || "").length, 8000);
      if (doc?.image_data_url) images += 1;
    }
  }
  return Math.round(chars / 4) + messages.length * 12 + images * 768;
}

export function findReusableEmptySession(
  sessions = [],
  { currentSession = null, preset = "chat", workspacePath = null, projectId = null } = {},
) {
  const norm = (v) => v || null;
  const targetWs = norm(workspacePath);
  const targetProj = norm(projectId);
  if (
    currentSession &&
    (currentSession.messages?.length || 0) === 0 &&
    (currentSession.staged_edits?.length || 0) === 0 &&
    currentSession.app_preset === preset &&
    norm(currentSession.workspace_path) === targetWs &&
    norm(currentSession.project_id) === targetProj
  ) {
    return currentSession.id;
  }
  const match = (sessions || []).find(
    (s) =>
      s &&
      (s.message_count || 0) === 0 &&
      !s.pinned &&
      s.app_preset === preset &&
      norm(s.workspace_path) === targetWs &&
      norm(s.project_id) === targetProj,
  );
  return match ? match.id : null;
}

export function pickInitialSession(sessions = [], initialWorkspace = null) {
  const list = Array.isArray(sessions) ? sessions : [];
  if (initialWorkspace) {
    const matching = list.filter((s) => s && s.workspace_path === initialWorkspace);
    const nonChat = matching.find((s) => s.app_preset && s.app_preset !== "chat");
    if (nonChat) return nonChat.id;
    if (matching[0]) return matching[0].id;
    return null;
  }
  return list[0]?.id || null;
}
