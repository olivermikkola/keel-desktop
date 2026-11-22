// Presets are persisted on sessions, so old `general` threads remain Work threads.
export const MODES = {
  chat: { preset: "chat", title: "Chat", placeholder: "Ask anything, explore an idea, or write together…" },
  work: { preset: "general", title: "Work", placeholder: "What would you like to get done in this folder?" },
  code: { preset: "code", title: "Code", placeholder: "Describe a change, ask about the code, or fix a bug…" },
};

export function modeForPreset(preset) {
  if (preset === "chat") return "chat";
  if (preset === "code") return "code";
  return "work";
}

export function isRuntimeLive(runtime) {
  return ["running", "external"].includes(runtime?.state);
}

export function promptBlockReason(session, runtime, workspace, harnessMode = "keel") {
  if (!session) return "Start a conversation first.";
  const mode = modeForPreset(session.app_preset);
  if (mode !== "chat" && !workspace) return "Choose a folder to use Work or Code.";
  if (mode === "code" && !["keel", "keel-cli"].includes(harnessMode)) return null;
  if (!isRuntimeLive(runtime)) return "Choose a model in Models to get started.";
  return null;
}
