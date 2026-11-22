import { createEveryday } from "./everyday.js";
import { createImageWorkspace } from "./images.js";
import { assistantStatus, friendlySetupError, isFirstRun, modelLocation, modelToWake, serverDataNotice } from "./onboarding.js";
import { MODES, modeForPreset, isRuntimeLive, promptBlockReason } from "./modes.js";
import { escapeHtml, renderMarkdown } from "./markdown.js";
import {
  ACCENT_MODES,
  APPEARANCE_DEFAULTS,
  DENSITY_MODES,
  TEXT_SIZES,
  TONE_MODES,
  countStagedEdits,
  detectViewerForPreset,
  filterModels,
  formatBytes,
  formatRelativeTime,
  normalizeAppearance,
  shortPath,
} from "./state.js";
import { renderCodeView, renderDiffHtml, renderDiffView, renderViewer, inlineWorkspaceImages } from "./viewers.js";
import { createWelcome } from "./welcome.js";

const tauri = window.__TAURI__?.core;
const events = window.__TAURI__?.event;

const APPEARANCE_STORAGE_KEY = "keel_appearance_v2";
const WELCOME_STORAGE_KEY = "keel_welcome_done";

const appState = {
  bootstrap: null,
  session: null,
  activeProjectId: null,
  pendingDocuments: [],
  searchResults: null,
  workspacePath: null,
  choosingWorkspace: false,
  workspaceFiles: [],
  activeView: "chat", // "chat" | "code" | "models" | "connectors"
  viewerOpen: false,
  viewerTab: "diff", // "diff" | "code" | "preview"
  viewerFile: null,
  selectedEditId: null,
  selectedModelInspectorId: null,
  modelSearchQuery: "",
  modelFilter: "all", // "all" | "installed" | "gguf"
  streaming: false,
  streamingText: "",
  streamingThinking: "",
  streamingTools: [],
  downloadProgress: {},
  waking: false,
  sendWhenReady: false,
  appearance: loadAppearance(),
};

const els = {
  sidebar: document.getElementById("sidebar"),
  appsList: document.getElementById("apps-list"),
  sessionsList: document.getElementById("sessions-list"),
  sidebarRuntimeBox: document.getElementById("sidebar-runtime-box"),
  newChatBtn: document.getElementById("new-chat-btn"),
  newAgentBtn: document.getElementById("new-agent-btn"),
  toggleSidebarBtn: document.getElementById("toggle-sidebar-btn"),
  modelPillBtn: document.getElementById("model-pill-btn"),
  runtimeDot: document.getElementById("runtime-dot"),
  modelPillLabel: document.getElementById("model-pill-label"),
  workspacePillBtn: document.getElementById("workspace-pill-btn"),
  workspacePillLabel: document.getElementById("workspace-pill-label"),
  harnessPillBtn: document.getElementById("harness-pill-btn"),
  harnessPillLabel: document.getElementById("harness-pill-label"),
  diffsPillBtn: document.getElementById("diffs-pill-btn"),
  diffsPillLabel: document.getElementById("diffs-pill-label"),
  toggleViewerBtn: document.getElementById("toggle-viewer-btn"),
  viewChatCode: document.getElementById("view-chat-code"),
  viewModels: document.getElementById("view-models"),
  viewTools: document.getElementById("view-tools"),
  viewLibrary: document.getElementById("view-library"),
  viewConnectors: document.getElementById("view-connectors"),
  chatScroll: document.getElementById("chat-scroll"),
  approvalSlot: document.getElementById("approval-slot"),
  promptInput: document.getElementById("prompt-input"),
  composerWsChip: document.getElementById("composer-ws-chip"),
  thinkingToggle: document.getElementById("thinking-toggle"),
  sendBtn: document.getElementById("send-btn"),
  stopBtn: document.getElementById("stop-btn"),
  viewerPane: document.getElementById("viewer-pane"),
  viewerFileSelect: document.getElementById("viewer-file-select"),
  refreshViewerBtn: document.getElementById("refresh-viewer-btn"),
  closeViewerBtn: document.getElementById("close-viewer-btn"),
  viewerContent: document.getElementById("viewer-content"),
  telemetryLeft: document.getElementById("telemetry-left"),
  telemetryRight: document.getElementById("telemetry-right"),
  modalBackdrop: document.getElementById("modal-backdrop"),
  modalTitle: document.getElementById("modal-title"),
  modalBody: document.getElementById("modal-body"),
  welcomeOverlay: document.getElementById("welcome-overlay"),
};

const images = createImageWorkspace({
  invoke,
  getWorkspace: () => appState.workspacePath,
  getEndpoint: () => appState.bootstrap?.config.image_endpoint,
  refresh: refreshBootstrap,
  selectFolder: () => els.workspacePillBtn.click(),
  openModels: () => setActiveView("models"),
  notice: showToast,
});

const drafts = new Map();
const everyday = createEveryday({
  invoke, state: () => appState, refresh: refreshBootstrap, render: renderAll,
  modal: openModal, closeModal, toast: showToast, renderSidebar,
  selectProject: async (id) => {
    if (appState.streaming) return;
    appState.activeProjectId = id;
    appState.searchResults = null;
    const project = appState.bootstrap.projects?.find((p) => p.id === id);
    if (project?.workspace_path) appState.workspacePath = project.workspace_path;
    await createNewSession("chat");
  },
  openArtifact: async (item) => {
    appState.workspacePath = item.workspace_path;
    appState.activeProjectId = item.project_id;
    await createNewSession(item.kind === "html" ? "websites" : item.kind === "csv" ? "sheets" : item.kind === "md" ? "docs" : "general");
    appState.viewerFile = item.path; appState.viewerOpen = true; appState.viewerTab = "preview"; renderAll();
  },
});

const welcome = createWelcome({
  invoke,
  root: els.welcomeOverlay,
  toast: showToast,
  bootstrap: () => appState.bootstrap,
  refresh: refreshBootstrap,
  starters: () => appState.bootstrap?.presets?.find((p) => p.id === "chat")?.starters || [],
  markDone: markWelcomeDone,
  changed: () => {
    if (!appState.bootstrap) return;
    renderHeader();
    renderSidebar();
    if (appState.session && !appState.session.messages.length && !appState.streaming) renderChat();
  },
  queuePrompt: queueFirstPrompt,
  ready: async () => { renderAll(); await sendQueuedPrompt(); els.promptInput.focus(); },
});

function saveDraft() {
  if (!appState.session) return;
  drafts.set(appState.session.id, { text: els.promptInput.value, documents: appState.pendingDocuments });
  try { localStorage.setItem(`keel_draft_${appState.session.id}`, els.promptInput.value); } catch { /* Storage quota may be unavailable. */ }
}
function restoreDraft() {
  const draft = drafts.get(appState.session.id);
  let text = draft?.text || "";
  if (!draft) { try { text = localStorage.getItem(`keel_draft_${appState.session.id}`) || ""; } catch { /* Ignore unavailable storage. */ } }
  els.promptInput.value = text;
  appState.pendingDocuments = draft?.documents || [];
  autoSizePromptInput(); renderAttachmentChips();
}
function renderAttachmentChips() {
  const target = document.getElementById("attachment-chips");
  target.innerHTML = appState.pendingDocuments.map((doc) => `<span class="attachment-chip"><button class="text-action" data-pending-source="${escapeHtml(doc.id)}">${escapeHtml(doc.name)}</button><button class="text-action" data-remove-attachment="${escapeHtml(doc.id)}" aria-label="Remove ${escapeHtml(doc.name)}">×</button></span>`).join("");
  target.querySelectorAll("[data-remove-attachment]").forEach((button) => { button.onclick = () => { appState.pendingDocuments = appState.pendingDocuments.filter((doc) => doc.id !== button.dataset.removeAttachment); saveDraft(); renderAttachmentChips(); }; });
  target.querySelectorAll("[data-pending-source]").forEach((button) => { button.onclick = () => everyday.previewSource(appState.pendingDocuments.find((doc) => doc.id === button.dataset.pendingSource)); });
}

function loadAppearance() {
  try {
    const raw = localStorage.getItem(APPEARANCE_STORAGE_KEY);
    return normalizeAppearance(raw ? JSON.parse(raw) : APPEARANCE_DEFAULTS);
  } catch {
    return { ...APPEARANCE_DEFAULTS };
  }
}

function applyAppearance(next) {
  appState.appearance = normalizeAppearance(next);
  document.documentElement.dataset.accent = appState.appearance.accent;
  document.documentElement.dataset.tone = appState.appearance.tone;
  document.documentElement.dataset.density = appState.appearance.density;
  document.documentElement.dataset.textSize = appState.appearance.textSize;
  els.sidebar.classList.toggle("collapsed", !appState.appearance.sidebar);
  try {
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(appState.appearance));
  } catch {
    // Ignore storage errors.
  }
}

function markWelcomeDone() {
  try {
    localStorage.setItem(WELCOME_STORAGE_KEY, "1");
  } catch {
    // Ignore storage errors.
  }
}

function welcomeFlag() {
  try {
    return localStorage.getItem(WELCOME_STORAGE_KEY);
  } catch {
    return "1";
  }
}

/** Starts the installed model without the person having to find a button. */
async function wakeAssistant() {
  const id = modelToWake(appState.bootstrap);
  if (!id || appState.waking) return false;
  appState.waking = true;
  renderAll();
  try {
    await invoke("start_model", { modelId: id });
    appState.waking = false;
    await refreshBootstrap();
    renderAll();
    await sendQueuedPrompt();
    return true;
  } catch (err) {
    appState.waking = false;
    appState.sendWhenReady = false;
    renderAll();
    showNotice(`Your AI didn't start: ${friendlySetupError(err).detail || "unknown problem"}. Tap the model name at the top to try again or pick another.`);
    return false;
  }
}

/** The composer keeps what was typed; it is sent as soon as the assistant is ready. */
function queueFirstPrompt(text) {
  els.promptInput.value = text;
  autoSizePromptInput();
  saveDraft();
  waitForAssistant();
}

function waitForAssistant() {
  appState.sendWhenReady = true;
  if (welcome.status().busy || appState.waking) { renderHeader(); return; }
  if (modelToWake(appState.bootstrap)) { wakeAssistant(); return; }
  renderHeader();
  welcome.show();
}

async function sendQueuedPrompt() {
  if (!appState.sendWhenReady) return;
  appState.sendWhenReady = false;
  if (isRuntimeLive(appState.bootstrap?.runtime) && els.promptInput.value.trim()) await sendCurrentPrompt();
  renderHeader();
}

async function invoke(cmd, args = {}) {
  if (!tauri) {
    throw new Error("Tauri runtime API is unavailable outside the desktop window.");
  }
  return tauri.invoke(cmd, args);
}

const APP_ICONS = {
  websites: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"></circle><line x1="3" y1="12" x2="21" y2="12"></line><path d="M12 3a13 13 0 0 1 0 18a13 13 0 0 1 0-18z"></path></svg>`,
  sheets: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"></rect><line x1="3" y1="10" x2="21" y2="10"></line><line x1="9" y1="4" x2="9" y2="20"></line></svg>`,
  docs: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline><line x1="8" y1="13" x2="16" y2="13"></line><line x1="8" y1="17" x2="13" y2="17"></line></svg>`,
  study: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M22 10l-10-5-10 5 10 5 10-5z"></path><path d="M6 12v5c3 2 9 2 12 0v-5"></path></svg>`,
  slides: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="2"></rect><line x1="12" y1="16" x2="12" y2="20"></line><line x1="8" y1="20" x2="16" y2="20"></line></svg>`,
};

const TOOL_VERBS = {
  read_file: "Read",
  list_files: "List",
  search_files: "Search",
  run_sql: "SQL",
  run_shell: "Shell",
  propose_edit: "Edit",
  create_file: "Create",
  delete_file: "Delete",
  rename_file: "Rename",
};

function toolTargetLabel(tool) {
  const args = tool.arguments || {};
  return (
    args.path ||
    args.rel_path ||
    args.dir ||
    args.pattern ||
    args.query ||
    args.command ||
    ""
  );
}

function renderEditFileSvg() {
  return `<svg class="inline-edit-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline><polyline points="10 13 8 15 10 17"></polyline><polyline points="14 13 16 15 14 17"></polyline></svg>`;
}

function renderInlineFileSvg() {
  return `<svg class="tool-inline-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline></svg>`;
}

function renderToolOrEditHtml(tool) {
  const staged = tool.edit_id
    ? (appState.session?.staged_edits || []).find((e) => e.id === tool.edit_id)
    : null;

  if (staged) {
    const plus = staged.added_lines ?? 0;
    const minus = staged.removed_lines ?? 0;
    const isPending = staged.status === "pending";
    const statusLabel =
      staged.status === "applied"
        ? "Approved · written to workspace"
        : "Rejected · discarded";
    const footer = isPending
      ? `<div class="inline-edit-footer">
           <button class="inline-edit-btn reject" ${appState.streaming ? "disabled" : ""} data-edit-reject="${escapeHtml(staged.id)}">Reject</button>
           <button class="inline-edit-btn approve" ${appState.streaming ? "disabled" : ""} data-edit-approve="${escapeHtml(staged.id)}">Approve</button>
         </div>`
      : `<div class="inline-edit-resolved ${staged.status === "applied" ? "applied" : ""}">
           <span>${statusLabel}</span>
           <span>${staged.status === "applied" ? "✓" : "×"}</span>
         </div>`;

    return `
      <details class="inline-edit-card" data-details-id="edit-${escapeHtml(staged.id)}" ${isPending ? "open" : ""}>
        <summary class="inline-edit-head">
          ${renderEditFileSvg()}
          <span class="inline-edit-title">${escapeHtml(staged.summary)}</span>
          <span class="inline-edit-counts">
            ${plus > 0 ? `<span class="diff-plus">+${plus}</span>` : ""}
            ${minus > 0 ? `<span class="diff-minus">-${minus}</span>` : ""}
          </span>
        </summary>
        <div class="inline-edit-diff-wrap">${renderDiffHtml(staged.diff || "")}</div>
        ${footer}
      </details>
    `;
  }

  const verb = TOOL_VERBS[tool.name] || tool.name;
  const target = toolTargetLabel(tool);
  const statusMark = tool.result === null ? "…" : tool.is_error ? "!" : "✓";
  return `
    <details class="tool-inline-row ${tool.is_error ? "error" : ""}">
      <summary class="tool-inline-summary">
        ${renderInlineFileSvg()}
        <span class="tool-inline-action">${escapeHtml(verb)}</span>
        ${target ? `<span class="tool-inline-target">${escapeHtml(target)}</span>` : ""}
        <span class="tool-inline-check">${statusMark}</span>
      </summary>
      ${tool.result ? `<pre class="tool-inline-output">${escapeHtml(tool.result)}</pre>` : ""}
    </details>
  `;
}

async function setActiveView(view) {
  if (appState.streaming || appState.choosingWorkspace) return;
  if (MODES[view] && modeForPreset(appState.session?.app_preset) !== view) {
    const recent = appState.bootstrap.sessions.find((s) => modeForPreset(s.app_preset) === view && (s.project_id || null) === appState.activeProjectId);
    if (recent) await loadSession(recent.id);
    else await createNewSession(MODES[view].preset);
  }
  appState.activeView = view;
  if (view === "code") {
    appState.viewerOpen = hasViewerContent();
    appState.viewerTab = (appState.session?.staged_edits?.length || 0) > 0 ? "diff" : "code";
  }
  renderAll();
}

async function init() {
  applyAppearance(appState.appearance);

  if (events) {
    await events.listen("keel://preference-saved", () => showToast("Saved to preferences. You can edit or remove it in the sidebar."));
    await events.listen("keel://setup-stage", (event) => welcome.stage(event.payload));
    await events.listen("keel://image-job", () => images.onJob());
    await events.listen("keel://agent-event", (ev) => handleAgentEvent(ev.payload));
    await events.listen("keel://download-progress", (ev) => {
      const p = ev.payload;
      appState.downloadProgress[p.model_id] = p;
      welcome.download(p);
      const bar = document.getElementById(`dl-prog-${p.model_id}`);
      if (bar) {
        const pct = p.total_bytes ? Math.round((p.downloaded_bytes / p.total_bytes) * 100) : 0;
        bar.textContent =
          p.done
            ? "Installed"
            : `${formatBytes(p.downloaded_bytes)}${pct ? ` (${pct}%)` : ""}`;
      }
    });
  }

  await refreshBootstrap();

  if (appState.bootstrap?.sessions?.length > 0) {
    await loadSession(appState.bootstrap.sessions[0].id);
  } else {
    await createNewSession("chat");
  }

  wireUi();
  await images.reload();
  renderAll();
  window.__KEEL_READY__ = { preset: appState.session.app_preset };
  if (isFirstRun(welcomeFlag(), appState.bootstrap)) welcome.show();
  else {
    markWelcomeDone();
    wakeAssistant();
  }
  setInterval(async () => {
    if (!appState.bootstrap) return;
    try {
      const next = await invoke("get_runtime_status");
      if (JSON.stringify(next) !== JSON.stringify(appState.bootstrap.runtime)) {
        appState.bootstrap.runtime = next; renderHeader(); renderSidebar();
        if (appState.activeView === "models") renderModelsWorkspace();
        if (next.error) showToast(next.error);
      }
    } catch { /* The next interaction can report a disconnected desktop. */ }
  }, 5000);
}

async function refreshBootstrap() {
  appState.bootstrap = await invoke("get_bootstrap_state", {
    workspacePath: appState.workspacePath,
  });
  if (!appState.workspacePath && appState.bootstrap.config.recent_workspaces.length > 0) {
    appState.workspacePath = appState.bootstrap.config.recent_workspaces[0];
    appState.bootstrap = await invoke("get_bootstrap_state", {
      workspacePath: appState.workspacePath,
    });
  }
  await refreshWorkspaceFiles();
}

// The side panel only opens when it has something to show: a previewable file or pending edits.
function hasViewerContent() {
  if (modeForPreset(appState.session?.app_preset) === "chat") return false;
  if ((appState.session?.staged_edits?.length || 0) > 0) return true;
  return Boolean(detectViewerForPreset(appState.session?.app_preset, appState.workspaceFiles));
}

async function createNewSession(preset = "chat", workspacePath = appState.workspacePath) {
  if (appState.streaming) return;
  saveDraft();
  appState.session = await invoke("create_session", {
    preset,
    workspacePath,
    projectId: appState.activeProjectId,
  });
  appState.workspacePath = appState.session.workspace_path || null;
  appState.selectedEditId = null;
  appState.activeView = modeForPreset(preset);
  appState.viewerOpen = hasViewerContent();
  appState.viewerTab = appState.activeView === "code" ? "code" : "preview";
  restoreDraft();
  await refreshBootstrap();
  renderAll();
}

async function loadSession(id) {
  if (appState.streaming) return;
  saveDraft();
  appState.session = await invoke("load_session_cmd", { id });
  appState.activeProjectId = appState.session.project_id || null;
  appState.searchResults = null;
  appState.workspacePath = appState.session.workspace_path || null;
  await refreshWorkspaceFiles();
  appState.activeView = modeForPreset(appState.session.app_preset);
  appState.viewerOpen = hasViewerContent();
  appState.viewerTab = appState.activeView === "code" ? "code" : "preview";
  appState.viewerFile = detectViewerForPreset(appState.session.app_preset, appState.workspaceFiles);
  restoreDraft();
  renderAll();
}

async function refreshWorkspaceFiles() {
  if (!appState.workspacePath) {
    appState.workspaceFiles = [];
    return;
  }
  try {
    appState.workspaceFiles = await invoke("list_workspace_files_cmd", {
      workspacePath: appState.workspacePath,
    });
    renderViewerOptions();
  } catch {
    appState.workspaceFiles = [];
  }
}

async function openWorkspace({ sample = false, carryDraft = false } = {}) {
  if (appState.choosingWorkspace || appState.streaming) return false;
  appState.choosingWorkspace = true;
  try {
    const path = await invoke(sample ? "prepare_sample_workspace" : "pick_workspace_folder");
    if (!path) return false;
    const session = appState.session;
    const mode = modeForPreset(session?.app_preset);
    const editingThread = mode !== "chat" && appState.activeView === mode;
    const hasHistory = Boolean(session?.messages.length || session?.staged_edits.length);
    const next = await invoke("set_workspace_folder", { path, sessionId: editingThread && !hasHistory ? session.id : null });
    if (editingThread && hasHistory && session.workspace_path !== next) {
      const text = els.promptInput.value;
      const documents = appState.pendingDocuments;
      await createNewSession(session.app_preset, next);
      if (carryDraft) {
        els.promptInput.value = text;
        appState.pendingDocuments = documents;
        saveDraft();
        autoSizePromptInput();
        renderAttachmentChips();
      }
    } else {
      appState.workspacePath = next;
      if (editingThread) appState.session.workspace_path = next;
      await refreshBootstrap();
      appState.viewerFile = detectViewerForPreset(appState.session?.app_preset, appState.workspaceFiles);
      appState.viewerOpen = hasViewerContent();
      renderAll();
    }
    return true;
  } catch (error) {
    showNotice(`Couldn’t open the folder. ${String(error)}`);
    return false;
  } finally {
    appState.choosingWorkspace = false;
    if (MODES[appState.activeView]) els.promptInput.focus();
  }
}

async function importKeelSession() {
  if (appState.streaming || appState.choosingWorkspace) return;
  if (!appState.workspacePath && !await openWorkspace()) return;
  try {
    const imported = await invoke("import_keel_session", { workspacePath: appState.workspacePath });
    if (imported) { await refreshBootstrap(); await loadSession(imported.id); }
  } catch (error) {
    showNotice(`Couldn’t open the CLI session. ${String(error)}`);
  }
}

function renderAll() {
  renderSidebar();
  renderHeader();
  renderViewVisibility();
  if (appState.activeView === "models") {
    renderModelsWorkspace();
  } else if (appState.activeView === "library") {
    everyday.renderLibrary(els.viewLibrary).catch((error) => showToast(String(error)));
  } else if (appState.activeView === "tools") {
    images.render(els.viewTools);
  } else if (appState.activeView === "connectors") {
    renderConnectorsWorkspace();
  } else {
    renderChat();
    renderViewerPane();
  }
}

function renderViewVisibility() {
  document.querySelectorAll(".mode-nav-item").forEach((btn) => {
    btn.classList.toggle("active", btn.getAttribute("data-view") === appState.activeView);
  });
  const isChatOrCode = appState.activeView === "chat" || appState.activeView === "work" || appState.activeView === "code";
  els.viewChatCode.classList.toggle("hidden", !isChatOrCode);
  els.viewModels.classList.toggle("hidden", appState.activeView !== "models");
  els.viewTools.classList.toggle("hidden", appState.activeView !== "tools");
  els.viewLibrary.classList.toggle("hidden", appState.activeView !== "library");
  els.viewConnectors.classList.toggle("hidden", appState.activeView !== "connectors");
}

function renderSidebar() {
  const b = appState.bootstrap;
  if (!b) return;

  els.appsList.innerHTML = "";
  const activePreset = appState.session?.app_preset || "general";
  els.appsList.classList.toggle("hidden", appState.activeView !== "work");
  const circleIds = ["websites", "sheets", "docs", "study", "slides"];
  for (const id of circleIds) {
    const preset = b.presets.find((p) => p.id === id);
    if (!preset) continue;
    const btn = document.createElement("button");
    btn.className = `app-circle-btn ${activePreset === preset.id ? "active" : ""}`;
    btn.title = preset.summary;
    btn.innerHTML = `
      <div class="app-circle-ring">${APP_ICONS[id] || APP_ICONS.docs}</div>
      <span>${escapeHtml(preset.title)}</span>
    `;
    btn.onclick = () => createNewSession(preset.id);
    els.appsList.appendChild(btn);
  }

  els.sessionsList.innerHTML = "";
  const projectsList = document.getElementById("projects-list");
  projectsList.innerHTML = `<button class="project-row ${!appState.activeProjectId ? "active" : ""}" data-project="">All conversations</button>` + (b.projects || []).map((project) => `<div class="project-row-wrap"><button class="project-row ${appState.activeProjectId === project.id ? "active" : ""}" data-project="${escapeHtml(project.id)}">${escapeHtml(project.name)}</button><button class="text-action" data-edit-project="${escapeHtml(project.id)}" aria-label="Edit ${escapeHtml(project.name)}">⋯</button></div>`).join("");
  projectsList.querySelectorAll("[data-project]").forEach((button) => { button.onclick = async () => {
    if (appState.streaming) return;
    appState.activeProjectId = button.dataset.project || null; appState.searchResults = null;
    const recent = b.sessions.find((session) => (session.project_id || null) === appState.activeProjectId);
    if (recent) await loadSession(recent.id);
    else {
      const project = b.projects?.find((p) => p.id === appState.activeProjectId);
      if (project?.workspace_path) appState.workspacePath = project.workspace_path;
      await createNewSession("chat");
    }
  }; });
  projectsList.querySelectorAll("[data-edit-project]").forEach((button) => { button.onclick = () => everyday.editProject(button.dataset.editProject); });
  const visibleSessions = appState.searchResults || b.sessions.filter((session) => (!appState.activeProjectId || session.project_id === appState.activeProjectId) && (session.message_count > 0 || session.id === appState.session?.id || session.pinned));
  if (!visibleSessions.length) els.sessionsList.innerHTML = '<p class="sidebar-empty">No conversations found.</p>';
  for (const s of visibleSessions) {
    const row = document.createElement("div");
    row.className = `recent-item ${appState.session?.id === s.id ? "active" : ""}`;
    row.innerHTML = `
      <span class="recent-mode">${MODES[modeForPreset(s.app_preset)].title}</span><span class="recent-title">${s.pinned ? "⌑ " : ""}${escapeHtml(s.title)}</span>
      <span class="recent-age">${escapeHtml(formatRelativeTime(s.updated_at))}</span>
      <button class="recent-del" title="Delete chat" aria-label="Delete chat">&times;</button>
    `;
    row.onclick = (e) => {
      if (e.target.closest(".recent-del")) return;
      loadSession(s.id);
    };
    row.querySelector(".recent-del").onclick = async (e) => {
      e.stopPropagation();
      if (appState.streaming) return;
      const remaining = await invoke("delete_session_cmd", { id: s.id });
      if (appState.session?.id === s.id) {
        if (remaining.length > 0) {
          await loadSession(remaining[0].id);
        } else {
          await createNewSession("chat");
        }
      } else {
        await refreshBootstrap();
        renderSidebar();
      }
      showToast("Conversation deleted.");
      const undo = document.createElement("button"); undo.className = "text-action"; undo.textContent = "Undo";
      undo.onclick = async () => { await invoke("restore_thread", { id: s.id }); await refreshBootstrap(); await loadSession(s.id); showToast("Conversation restored."); };
      document.getElementById("global-notice").appendChild(undo);
    };
    els.sessionsList.appendChild(row);
  }

  const rt = b.runtime;
  const activeModel = b.models.find((m) => m.id === (rt.active_model_id || b.config.selected_model_id));
  const setup = welcome.status();
  const status = assistantStatus({ runtime: rt, waking: appState.waking, setup });
  const sidebarLocation = modelLocation(rt);
  const runningLabel =
    rt.state === "external"
      ? `${b.config.external_model_name || sidebarLocation}${sidebarLocation === "External model server" ? " · Remote" : ""}`
      : rt.state === "running" ? activeModel?.name || "Local model" : status.key === "setup" ? "Setting up your AI" : "No model loaded";
  const isLive = isRuntimeLive(rt);
  const fill = status.key === "setup" ? ` style="width:${Math.round(setup.fraction * 100)}%"` : "";
  const canOpenSetup = status.key === "setup" || status.key === "paused";
  els.sidebarRuntimeBox.classList.toggle("clickable", canOpenSetup);
  els.sidebarRuntimeBox.onclick = canOpenSetup ? () => welcome.show() : null;
  els.sidebarRuntimeBox.innerHTML = `
    <span class="status-dot ${status.key === "setup" || status.key === "waking" ? "starting" : rt.state}"></span>
    <div class="sidebar-runtime-meta">
      <div class="sidebar-runtime-name">${escapeHtml(runningLabel)}</div>
      ${status.key === "setup" ? `<div class="sidebar-runtime-bar"><div class="sidebar-runtime-fill active"${fill}></div></div>` : ""}
    </div>
    <span class="sidebar-runtime-state">${escapeHtml(status.label)}</span>
  `;
}

function renderHeader() {
  const b = appState.bootstrap;
  if (!b) return;
  const rt = b.runtime;
  els.runtimeDot.className = `status-dot ${rt.state}`;
  const selected = b.models.find(
    (m) => m.id === (rt.active_model_id || b.config.selected_model_id),
  );
  const setup = welcome.status();
  const location = modelLocation(rt);
  els.modelPillBtn.title = isRuntimeLive(rt) ? location : "";
  els.modelPillLabel.textContent = setup.busy || setup.paused ? assistantStatus({ runtime: rt, setup }).label : appState.waking ? "Waking up your AI…" : rt.state === "stopped" ? (b.models.some((m) => m.installed) ? "Start your AI" : "Set up your AI") : rt.state === "error" ? "Model needs attention" :
    rt.state === "external"
      ? `${b.config.external_model_name || location}${location === "External model server" ? " · Remote" : ""}`
      : selected
        ? selected.name
        : b.config.selected_model_id || "Select model";

  const shortWs = shortPath(appState.workspacePath);
  els.workspacePillLabel.textContent = appState.workspacePath ? shortWs : "Open folder";

  const harness = b.harnesses.find((h) => h.id === b.config.harness_mode);
  const boundaryLimited = b.code_execution && !b.code_execution.can_run_commands;
  els.harnessPillLabel.textContent = boundaryLimited
    ? "Engine: Inspect & propose"
    : `Engine: ${harness ? harness.name.replace(" Harness", "") : "Keel"}`;
  els.harnessPillBtn.title = b.code_execution?.notice || "Agent harness";

  const counts = countStagedEdits(appState.session?.staged_edits ?? []);
  els.diffsPillLabel.textContent =
    counts.pending > 0
      ? `${counts.pending} pending`
      : `${counts.applied} applied`;
  els.diffsPillBtn.classList.toggle("has-pending", counts.pending > 0);
  els.telemetryRight.textContent = `Keel v${b.version}`;
  if (!appState.streaming) {
    els.telemetryLeft.textContent = appState.sendWhenReady ? "Your message sends as soon as your AI is ready" : appState.waking ? "Waking up your AI. This takes a few seconds." : isRuntimeLive(rt) ? `${location} · ${appState.session?.messages.length || 0} messages` : "Your AI is off. Send a message to start it.";
  }
  const mode = modeForPreset(appState.session?.app_preset);
  els.promptInput.placeholder = MODES[mode].placeholder;
  els.composerWsChip.textContent = appState.workspacePath ? shortWs : "Open folder";
  els.composerWsChip.classList.toggle("hidden", mode === "chat");
  els.workspacePillBtn.classList.toggle("hidden", mode === "chat" && appState.activeView !== "tools");
  els.harnessPillBtn.classList.toggle("hidden", mode !== "code");
  els.toggleViewerBtn.classList.toggle("hidden", mode === "chat");
  els.diffsPillBtn.classList.toggle("hidden", mode === "chat" || counts.pending + counts.applied === 0);
  els.newChatBtn.disabled = appState.streaming;
  els.newAgentBtn.disabled = appState.streaming;
}

function renderChat() {
  const s = appState.session;
  const b = appState.bootstrap;
  if (!s || !b) return;

  if (s.messages.length === 0 && !appState.streaming) {
    const preset = b.presets.find((p) => p.id === s.app_preset) || b.presets[0];
    const project = b.projects?.find((p) => p.id === appState.activeProjectId);
    const starters = preset.starters
      .map(
        (st) =>
          `<button class="starter-card" data-starter="${escapeHtml(st)}">${escapeHtml(st)}</button>`,
      )
      .join("");
    const live = isRuntimeLive(b.runtime);
    const setup = welcome.status();
    const setupBusy = setup.busy;
    const canWake = Boolean(modelToWake(b));
    const needsFolder = modeForPreset(preset.id) !== "chat" && !appState.workspacePath;
    const action = needsFolder
      ? `<button class="btn btn-primary" id="empty-open-folder">Open folder</button>${preset.id === "general" ? ' <button class="btn btn-ghost" id="empty-sample-folder">Try with sample files</button>' : ""}`
      : live || appState.waking ? "" : setup.paused ? '<button class="btn btn-primary" id="chat-setup-btn">Resume setup</button>' : setupBusy ? '<button class="btn btn-primary" id="chat-setup-btn">See setup progress</button>' : canWake ? '<button class="btn btn-primary" id="chat-setup-btn">Start your AI</button>' : '<button class="btn btn-primary" id="chat-setup-btn">Set up my AI</button>';
    const status = appState.waking ? '<p class="empty-status" role="status">Waking up your AI. You can type your question now.</p>' : "";
    const openCli = preset.id === "code" && b.config.harness_mode === "keel-cli"
      ? '<button class="btn btn-ghost" id="empty-open-cli-session">Open CLI session</button>' : "";
    const boundaryNotice =
      preset.id === "code" && b.code_execution && !b.code_execution.can_run_commands && b.code_execution.notice
        ? `<p id="code-boundary-notice" class="empty-status" role="status">${escapeHtml(b.code_execution.notice)}</p>`
        : "";
    const heading = preset.id === "chat" && !project ? "What can I help with?" : preset.title;
    els.chatScroll.innerHTML = `
      <div class="empty-state">
        ${project ? `<span class="eyebrow">${escapeHtml(project.name)}</span>` : ""}
        <h1>${escapeHtml(heading)}</h1>
        <p>${escapeHtml(preset.summary)}</p>
        ${action}${openCli}${status}${boundaryNotice}
        <div class="starters-grid">${starters}</div>
      </div>
    `;
    els.chatScroll.querySelector("#chat-setup-btn")?.addEventListener("click", () => (canWake && !setupBusy && !setup.paused ? wakeAssistant() : welcome.show()));
    els.chatScroll.querySelector("#empty-open-folder")?.addEventListener("click", () => openWorkspace());
    els.chatScroll.querySelector("#empty-sample-folder")?.addEventListener("click", () => openWorkspace({ sample: true }));
    els.chatScroll.querySelector("#empty-open-cli-session")?.addEventListener("click", importKeelSession);
    els.chatScroll.querySelectorAll("[data-starter]").forEach((btn) => {
      btn.onclick = () => {
        els.promptInput.value = btn.getAttribute("data-starter");
        saveDraft();
        sendCurrentPrompt();
      };
    });
    return;
  }

  const shouldStick = els.chatScroll.scrollHeight - els.chatScroll.scrollTop - els.chatScroll.clientHeight < 100;
  const oldTop = els.chatScroll.scrollTop;
  const details = new Map([...els.chatScroll.querySelectorAll("details[data-details-id]")].map((d) => [d.dataset.detailsId, d.open]));
  let html = "";
  for (const msg of s.messages) {
    html += renderMessageHtml(msg);
  }

  if (appState.streaming) {
    const toolsHtml = appState.streamingTools.map(renderToolOrEditHtml).join("");
    const thinkingHtml = appState.streamingThinking
      ? `<details class="thinking-box" data-details-id="live-thought"><summary>Thinking</summary><pre class="mono">${escapeHtml(appState.streamingThinking)}</pre></details>`
      : "";
    html += `
      <div class="msg assistant">
        ${thinkingHtml}
        ${toolsHtml}
        <div class="bubble markdown-body">${renderMarkdown(appState.streamingText || "Working...")}</div>
      </div>
    `;
  }

  els.chatScroll.innerHTML = html;
  els.chatScroll.querySelectorAll("details[data-details-id]").forEach((d) => { if (details.has(d.dataset.detailsId)) d.open = details.get(d.dataset.detailsId); });
  els.chatScroll.scrollTop = shouldStick ? els.chatScroll.scrollHeight : oldTop;
}

function renderMessageHtml(msg) {
  const documents = (msg.attachments || []).map((doc) => `<button class="attachment-chip" data-message-source="${escapeHtml(doc.id)}">▤ ${escapeHtml(doc.name)}</button>`).join("");
  const disabled = appState.streaming ? "disabled" : "";
  if (msg.role === "user") {
    return `<div class="msg user" data-message-id="${escapeHtml(msg.id)}">${documents ? `<div class="message-sources">${documents}</div>` : ""}<div class="bubble">${escapeHtml(msg.content)}</div><div class="message-actions"><button class="text-action" data-edit-message="${escapeHtml(msg.id)}" ${disabled}>Edit</button></div></div>`;
  }
  const thinkingHtml = msg.thinking
    ? `<details class="thinking-box" data-details-id="thinking-${escapeHtml(msg.id)}"><summary>Thinking</summary><pre class="mono">${escapeHtml(msg.thinking)}</pre></details>` : "";
  const toolsHtml = (msg.tool_calls || []).map(renderToolOrEditHtml).join("");
  const lastUser = [...appState.session.messages].reverse().find((m) => m.role === "user");
  const canRetry = msg === appState.session.messages[appState.session.messages.length - 1] && !(msg.tool_calls?.length) && lastUser;
  return `<div class="msg assistant" data-message-id="${escapeHtml(msg.id)}">${thinkingHtml}${toolsHtml}<div class="bubble markdown-body">${renderMarkdown(msg.content)}</div>
    ${msg.status === "failed" ? '<span class="reply-status">Reply interrupted · partial answer kept</span>' : ""}
    <div class="message-actions"><button class="text-action" data-copy-message="${escapeHtml(msg.id)}">Copy</button>${canRetry ? `<button class="text-action" data-retry-message="${escapeHtml(lastUser.id)}" ${disabled}>Try again</button>` : ""}</div></div>`;
}

function handleAgentEvent(ev) {
  if (!ev || !ev.type) return;
  if (ev.type === "thinking_delta") {
    appState.streamingThinking += ev.text;
  } else if (ev.type === "content_delta") {
    appState.streamingText += ev.text;
  } else if (ev.type === "tool_call_start") {
    appState.streamingTools.push({
      name: ev.name,
      arguments: ev.arguments || {},
      result: null,
      is_error: false,
      edit_id: null,
    });
  } else if (ev.type === "staged_edit_added") {
    if (appState.session) {
      appState.session.staged_edits.push(ev.edit);
      appState.selectedEditId = ev.edit.id;
      appState.viewerFile = ev.edit.path;
      renderHeader();
    }
  } else if (ev.type === "tool_call_result") {
    const last = [...appState.streamingTools]
      .reverse()
      .find((t) => t.name === ev.name && t.result === null);
    if (last) {
      last.result = ev.result;
      last.is_error = ev.is_error;
      last.edit_id = ev.edit_id || last.edit_id || null;
    }
  } else if (ev.type === "approval_required") {
    renderApprovalCard(ev);
  } else if (ev.type === "turn_finished") {
    els.telemetryLeft.textContent = modelLocation(appState.bootstrap.runtime);
  }
  renderChat();
  if (["staged_edit_added", "turn_finished"].includes(ev.type)) {
    if (!appState.viewerOpen && ev.type === "staged_edit_added") {
      appState.viewerOpen = true;
      appState.viewerTab = modeForPreset(appState.session?.app_preset) === "code" ? "diff" : "preview";
    }
    if (appState.viewerOpen) renderViewerPane();
  }
}

function renderApprovalCard(req) {
  const cliApproval = appState.session?.app_preset === "code" && appState.bootstrap.config.harness_mode === "keel-cli";
  els.approvalSlot.innerHTML = `
    <div class="approval-card">
      <div><strong>Approval required:</strong> ${escapeHtml(req.summary)}</div>
      <div class="approval-actions">
        <button id="appr-once" class="btn btn-primary btn-sm">Allow once</button>
        ${cliApproval ? "" : '<button id="appr-always" class="btn btn-sm">Allow for this chat</button>'}
        <button id="appr-deny" class="btn btn-rust btn-sm">Deny</button>
      </div>
    </div>
  `;
  const decide = async (decision) => {
    els.approvalSlot.innerHTML = "";
    await invoke("respond_approval", { id: req.id, decision });
  };
  document.getElementById("appr-once").onclick = () => decide("allow_once");
  document.getElementById("appr-always")?.addEventListener("click", () => decide("allow_always"));
  document.getElementById("appr-deny").onclick = () => decide("deny");
}

async function sendCurrentPrompt() {
  if (appState.streaming || appState.choosingWorkspace || !appState.session) return;
  const prompt = els.promptInput.value.trim() || (appState.pendingDocuments.length ? "Summarize the attached documents and highlight the key points." : "");
  if (!prompt) return;
  if (!appState.workspacePath && modeForPreset(appState.session.app_preset) !== "chat") {
    if (await openWorkspace({ carryDraft: true })) await sendCurrentPrompt();
    return;
  }
  const harnessMode = appState.bootstrap.config.harness_mode;
  const blocked = promptBlockReason(appState.session, appState.bootstrap.runtime,
    appState.workspacePath, harnessMode);
  if (blocked) {
    // A stopped model is not a dead end: start it (or finish setup) and send afterwards.
    const otherReason = promptBlockReason(appState.session, { state: "running" }, appState.workspacePath, harnessMode);
    if (otherReason) showNotice(otherReason);
    else waitForAssistant();
    return;
  }
  els.approvalSlot.innerHTML = "";

  const documents = appState.pendingDocuments;
  appState.pendingDocuments = [];
  els.promptInput.value = "";
  saveDraft();
  renderAttachmentChips();
  autoSizePromptInput();
  appState.session.messages.push({
    id: `local-${Date.now()}`,
    role: "user",
    content: prompt,
    attachments: documents,
    thinking: null,
    tool_calls: [],
    created_at: Math.floor(Date.now() / 1000),
  });

  appState.streaming = true;
  appState.streamingText = "";
  appState.streamingThinking = "";
  appState.streamingTools = [];
  els.sendBtn.classList.add("hidden");
  els.stopBtn.classList.remove("hidden");
  renderChat();

  try {
    appState.session = await invoke("send_message", {
      input: {
        session_id: appState.session.id,
        prompt,
        enable_thinking: els.thinkingToggle.checked,
        attachments: documents,
        selected_file: modeForPreset(appState.session.app_preset) !== "chat" && appState.viewerOpen ? appState.viewerFile : null,
        workspace_path: appState.workspacePath,
      },
    });
    await refreshBootstrap();
    if (appState.session.staged_edits?.length > 0) {
      const latest = appState.session.staged_edits[appState.session.staged_edits.length - 1];
      appState.selectedEditId = latest.id;
      appState.viewerFile = latest.path;
    }
  } catch (err) {
    els.promptInput.value = prompt;
    appState.pendingDocuments = documents;
    saveDraft(); renderAttachmentChips();
    appState.session.messages.push({
      id: `err-${Date.now()}`,
      role: "assistant",
      content: `Runtime error: ${String(err)}`,
      thinking: null,
      tool_calls: [],
      created_at: Math.floor(Date.now() / 1000),
    });
  } finally {
    appState.streaming = false;
    els.sendBtn.classList.remove("hidden");
    els.stopBtn.classList.add("hidden");
    els.approvalSlot.innerHTML = "";
    renderAll();
  }
}

function renderViewerPane() {
  els.viewerPane.classList.toggle("hidden", !appState.viewerOpen || appState.activeView === "chat");
  document.querySelectorAll("[data-viewer-tab]").forEach((tab) => {
    tab.classList.toggle("active", tab.getAttribute("data-viewer-tab") === appState.viewerTab);
  });
  if (!appState.viewerOpen) return;
  renderViewerOptions();
  loadViewerContent();
}

function renderViewerOptions() {
  const files = appState.workspaceFiles.filter((f) => !f.endsWith("/"));
  if (!appState.viewerFile || !files.includes(appState.viewerFile)) {
    appState.viewerFile =
      detectViewerForPreset(appState.session?.app_preset || "general", files) || files[0] || null;
  }
  els.viewerFileSelect.innerHTML = files
    .map(
      (f) =>
        `<option value="${escapeHtml(f)}" ${f === appState.viewerFile ? "selected" : ""}>${escapeHtml(f)}</option>`,
    )
    .join("");
}

async function loadViewerContent() {
  if (appState.viewerTab === "diff") {
    const edits = appState.session?.staged_edits || [];
    const activeEdit =
      edits.find((e) => e.id === appState.selectedEditId) ||
      edits.find((e) => e.path === appState.viewerFile) ||
      edits[edits.length - 1] ||
      null;
    renderDiffView(els.viewerContent, activeEdit);
    return;
  }

  if (!appState.workspacePath || !appState.viewerFile) {
    renderViewer(els.viewerContent, null, "");
    return;
  }
  try {
    if (appState.viewerFile.toLowerCase().endsWith(".png")) {
      const src = await invoke("read_workspace_png", { workspacePath: appState.workspacePath, relPath: appState.viewerFile });
      els.viewerContent.innerHTML = `<img class="generated-image" alt="Workspace image" src="${src}" />`;
      return;
    }
    let content = await invoke("read_workspace_file_raw", {
      workspacePath: appState.workspacePath,
      relPath: appState.viewerFile,
    });
    if (appState.viewerTab === "code") {
      renderCodeView(els.viewerContent, appState.viewerFile, content);
    } else {
      if (/\.html?$/i.test(appState.viewerFile)) {
        const workspace = appState.workspacePath;
        content = await inlineWorkspaceImages(content, appState.viewerFile, (relPath) =>
          invoke("read_workspace_png", { workspacePath: workspace, relPath }));
      }
      renderViewer(els.viewerContent, appState.viewerFile, content);
    }
  } catch (err) {
    els.viewerContent.innerHTML = `<div class="viewer-empty">${escapeHtml(String(err))}</div>`;
  }
}

function renderModelsWorkspace() {
  const b = appState.bootstrap;
  if (!b) return;
  const rt = b.runtime;
  const filtered = filterModels(b.models, appState.modelSearchQuery, appState.modelFilter);
  const inspected =
    b.models.find((m) => m.id === appState.selectedModelInspectorId) ||
    b.models.find((m) => m.id === rt.active_model_id) ||
    b.models.find((m) => m.id === b.config.selected_model_id) ||
    b.models[0];

  const rowsHtml = filtered
    .map((m) => {
      const isRunning = rt.active_model_id === m.id && rt.state === "running";
      const isSelected = inspected?.id === m.id;
      return `
        <div class="model-clean-row ${isSelected ? "active" : ""}" data-select-model="${escapeHtml(m.id)}">
          <div>
            <div class="model-clean-title">
              <span>${escapeHtml(m.name)}</span>
              
            </div>
            <div class="model-clean-sub">
              ${escapeHtml(m.detail)} · ${formatBytes(m.bytes)}
              <span id="dl-prog-${escapeHtml(m.id)}" style="color:var(--accent);margin-left:6px"></span>
            </div>
          </div>
          ${
            m.installed
              ? `<button class="pill-action-use" data-start-model="${escapeHtml(m.id)}">${isRunning ? "In use" : "Use"}</button>`
              : `<button class="pill-action-get" data-dl-model="${escapeHtml(m.id)}">Download</button>`
          }
        </div>
      `;
    })
    .join("");

  const isRunningExternal = rt.state === "external";
  const isRunningLocal = rt.state === "running";

  els.viewModels.innerHTML = `
    <div class="models-main-col">
      <div class="model-setup-row"><div><strong>Not sure which one?</strong><p class="muted">Keel picks the right model for this computer and sets everything up.</p></div><button id="models-setup-btn" class="btn ${isRunningLocal || isRunningExternal ? "" : "btn-primary "}btn-sm">Set it up for me</button></div>
      <div class="models-search-bar">
        <input id="models-search-in" class="input grow" value="${escapeHtml(appState.modelSearchQuery)}" placeholder="Search models" />
        <div class="models-filter-pills">
          ${["all", "installed"]
            .map(
              (f) =>
                `<button class="viewer-tab ${appState.modelFilter === f ? "active" : ""}" data-mfilter="${f}">${f === "all" ? "All" : "Downloaded"}</button>`,
            )
            .join("")}
        </div>
      </div>

      <div class="model-list-clean">
        ${rowsHtml || `<div class="card">No models match "${escapeHtml(appState.modelSearchQuery)}".</div>`}
      </div>

      <details class="card">
        <summary class="import-gguf-toggle">Advanced: add your own model or use Ollama / LM Studio</summary>
        <div class="row" style="margin-top:8px">
          <input id="ws-hf-url" class="input grow" placeholder="Hugging Face URL or repo:owner/repo" />
          <button id="ws-hf-dl" class="btn btn-sm">Download</button>
        </div>
        <div class="row">
          <input id="ws-local-gguf" class="input grow mono" placeholder="/path/to/model.gguf" />
          <button id="ws-local-imp" class="btn btn-sm">Import</button>
        </div>
        <div class="row">
          <input id="ws-ext-url" class="input grow mono" value="${escapeHtml(b.config.external_base_url || "")}" placeholder="http://127.0.0.1:8080/v1" />
          <input id="ws-ext-name" class="input" style="max-width:160px" value="${escapeHtml(b.config.external_model_name || "")}" placeholder="Model name" />
          <button id="ws-ext-use" class="btn btn-primary btn-sm">Connect</button>
        </div>
        <p class="muted" id="ws-ext-notice">${serverDataNotice(b.config.external_base_url || "")}</p>
      </details>
    </div>

    <aside class="models-inspector-col">
      <div>
        <div class="sidebar-section-label">SELECTED</div>
        <h3 style="margin:6px 0 2px;font-size:17px">${escapeHtml(inspected ? inspected.name : "Local Runtime")}</h3>
        <div style="font-size:12px;color:var(--muted)">${inspected ? `${formatBytes(inspected.bytes)} download${inspected.installed ? " · on this computer" : ""}` : ""}</div>
      </div>

      <p class="muted">${inspected ? escapeHtml(inspected.detail) : ""}</p>
      <p class="muted">Chat, Work, and Code all use the model you pick here.</p>
      <div class="card">
        
        <div class="row" style="font-size:12px;color:var(--accent)">
          <span class="status-dot ${rt.state}"></span>
          <span>${isRunningLocal || isRunningExternal ? modelLocation(rt) : "Off"}</span>
        </div>
        <div class="row" style="margin-top:4px">
          ${
            isRunningLocal || isRunningExternal
              ? `<button id="insp-stop-rt" class="btn btn-sm grow">Turn off</button>`
              : inspected?.installed
                ? `<button id="insp-start-rt" class="btn btn-primary btn-sm grow">Start</button>`
                : ""
          }
        </div>
      </div>
    </aside>
  `;

  const modelsSetupBtn = document.getElementById("models-setup-btn");
  if (modelsSetupBtn) modelsSetupBtn.onclick = () => welcome.show();
  const searchIn = document.getElementById("models-search-in");
  searchIn.oninput = () => {
    appState.modelSearchQuery = searchIn.value;
    renderModelsWorkspace();
    const reFocus = document.getElementById("models-search-in");
    reFocus?.focus();
    reFocus?.setSelectionRange(reFocus.value.length, reFocus.value.length);
  };
  els.viewModels.querySelectorAll("[data-mfilter]").forEach((btn) => {
    btn.onclick = () => {
      appState.modelFilter = btn.getAttribute("data-mfilter");
      renderModelsWorkspace();
    };
  });
  els.viewModels.querySelectorAll("[data-select-model]").forEach((row) => {
    row.onclick = (e) => {
      if (e.target.closest("button")) return;
      appState.selectedModelInspectorId = row.getAttribute("data-select-model");
      renderModelsWorkspace();
    };
  });
  els.viewModels.querySelectorAll("[data-start-model]").forEach((btn) => {
    btn.onclick = async () => {
      await invoke("start_model", { modelId: btn.getAttribute("data-start-model") });
      await refreshBootstrap();
      renderAll();
    };
  });
  els.viewModels.querySelectorAll("[data-dl-model]").forEach((btn) => {
    btn.onclick = async () => {
      await invoke("download_catalog_model", { modelId: btn.getAttribute("data-dl-model") });
      await refreshBootstrap();
      renderAll();
    };
  });

  document.getElementById("insp-stop-rt")?.addEventListener("click", async () => {
    await invoke("stop_runtime");
    await refreshBootstrap();
    renderAll();
  });
  document.getElementById("insp-start-rt")?.addEventListener("click", async () => {
    if (!inspected) return;
    await invoke("start_model", { modelId: inspected.id });
    await refreshBootstrap();
    renderAll();
  });

  document.getElementById("ws-hf-dl").onclick = async () => {
    await invoke("download_huggingface_gguf", { url: document.getElementById("ws-hf-url").value });
    await refreshBootstrap();
    renderAll();
  };
  document.getElementById("ws-local-imp").onclick = async () => {
    await invoke("import_local_gguf", { path: document.getElementById("ws-local-gguf").value });
    await refreshBootstrap();
    renderAll();
  };
  document.getElementById("ws-ext-use").onclick = async () => {
    await invoke("set_external_endpoint", {
      baseUrl: document.getElementById("ws-ext-url").value,
      modelName: document.getElementById("ws-ext-name").value || null,
    });
    await refreshBootstrap();
    renderAll();
  };
  document.getElementById("ws-ext-url").oninput = (event) => {
    document.getElementById("ws-ext-notice").textContent = serverDataNotice(event.target.value);
  };
}

function renderConnectorsWorkspace() {
  const b = appState.bootstrap;
  if (!b) return;
  const serversHtml = (b.mcp_servers || [])
    .map(
      (srv) => `
      <div class="card">
        <div class="row">
          <strong class="grow">${escapeHtml(srv.name)}</strong>
          <span class="mono" style="font-size:12px;color:var(--muted)">${escapeHtml(srv.command)} ${escapeHtml(srv.args.join(" "))}</span>
          <button class="btn btn-ghost btn-sm" data-rm-mcp="${escapeHtml(srv.name)}">Remove</button>
        </div>
      </div>
    `,
    )
    .join("");

  els.viewConnectors.innerHTML = `
    <div class="connectors-inner">
      <div class="card">
        <div class="row">
          <strong class="grow">Your folder</strong>
          <span class="status-dot ${appState.workspacePath ? "ready" : ""}"></span>
          <button id="conn-pick-ws" class="btn btn-sm">Change folder</button>
        </div>
        <div class="mono" style="font-size:12.5px;color:var(--muted)">${escapeHtml(appState.workspacePath || "No folder chosen. Work and Code can only read and change files inside the folder you pick.")}</div>
      </div>

      <div class="card">
        <div class="row">
          <strong class="grow">Spreadsheets</strong>
          <span class="status-dot ready"></span>
          <span style="font-size:12px;color:var(--accent)">On</span>
        </div>
        <div style="font-size:12.5px;color:var(--muted)">Keel can read and analyse the CSV files in your folder.</div>
      </div>

      <div class="card">
        <strong>Advanced: add a tool server (MCP)</strong>
        <div class="row">
          <input id="mcp-name" class="input" placeholder="name (e.g. filesystem)" />
          <input id="mcp-cmd" class="input mono" placeholder="command (e.g. npx)" />
        </div>
        <input id="mcp-args" class="input mono" placeholder="space-separated args" />
        <div class="row">
          <select id="mcp-scope" class="select-field grow">
            <option value="global">Global (~/.config/keel-desktop/mcp.json)</option>
            <option value="workspace">Workspace (.keel/mcp.json)</option>
          </select>
          <button id="mcp-add-btn" class="btn btn-primary btn-sm">Add</button>
        </div>
      </div>

      ${serversHtml}
    </div>
  `;

  document.getElementById("conn-pick-ws").onclick = () => els.workspacePillBtn.click();
  document.getElementById("mcp-add-btn").onclick = async () => {
    const args = document.getElementById("mcp-args").value.trim().split(/\s+/).filter(Boolean);
    await invoke("save_mcp_server", {
      input: {
        name: document.getElementById("mcp-name").value,
        command: document.getElementById("mcp-cmd").value,
        args,
        scope: document.getElementById("mcp-scope").value,
        workspace_path: appState.workspacePath,
      },
    });
    await refreshBootstrap();
    renderConnectorsWorkspace();
  };
  els.viewConnectors.querySelectorAll("[data-rm-mcp]").forEach((btn) => {
    btn.onclick = async () => {
      await invoke("remove_mcp_server", {
        name: btn.getAttribute("data-rm-mcp"),
        workspacePath: appState.workspacePath,
      });
      await refreshBootstrap();
      renderConnectorsWorkspace();
    };
  });
}

let modalReturnFocus;
function openModal(title, builder) {
  modalReturnFocus = document.activeElement;
  els.modalTitle.textContent = title;
  els.modalBody.innerHTML = "";
  builder(els.modalBody);
  els.modalBackdrop.classList.remove("hidden");
  els.modalBody.querySelector("input, textarea, select, button")?.focus();
}

function closeModal() {
  els.modalBackdrop.classList.add("hidden");
  if (modalReturnFocus?.isConnected) modalReturnFocus.focus();
}

function openDiffsModal() {
  const edits = appState.session?.staged_edits ?? [];
  openModal("Workspace File Changes", (body) => {
    const top = document.createElement("div");
    top.className = "row";
    top.innerHTML = `
      <span class="grow" style="color:var(--muted)">Review proposed edits before writing them to disk.</span>
      <button id="apply-all-btn" class="btn btn-primary btn-sm">Approve all</button>
    `;
    body.appendChild(top);
    top.querySelector("#apply-all-btn").onclick = async () => {
      appState.session = await invoke("apply_all_staged_edits", { sessionId: appState.session.id });
      await refreshWorkspaceFiles();
      renderAll();
      openDiffsModal();
    };

    if (edits.length === 0) {
      const empty = document.createElement("div");
      empty.className = "card";
      empty.textContent = "No edits staged in this session yet.";
      body.appendChild(empty);
      return;
    }

    for (const edit of edits) {
      const card = document.createElement("div");
      card.className = "card";
      const counts = `${edit.added_lines ? `+${edit.added_lines}` : ""} ${edit.removed_lines ? `-${edit.removed_lines}` : ""}`.trim();
      card.innerHTML = `
        <div class="row">
          <strong class="grow mono">${escapeHtml(edit.summary)}</strong>
          ${counts ? `<span class="mono" style="font-size:12px;color:var(--accent)">${escapeHtml(counts)}</span>` : ""}
          <span class="pill-btn">${escapeHtml(edit.status)}</span>
        </div>
        <div class="inline-edit-diff-wrap">${renderDiffHtml(edit.diff)}</div>
      `;
      if (edit.status === "pending") {
        const actions = document.createElement("div");
        actions.className = "row";
        const discBtn = document.createElement("button");
        discBtn.className = "btn btn-ghost btn-sm";
        discBtn.textContent = "Reject";
        discBtn.onclick = async () => {
          appState.session = await invoke("discard_staged_edit_cmd", {
            sessionId: appState.session.id,
            editId: edit.id,
          });
          renderAll();
          openDiffsModal();
        };
        const applyBtn = document.createElement("button");
        applyBtn.className = "btn btn-primary btn-sm";
        applyBtn.textContent = "Approve";
        applyBtn.onclick = async () => {
          appState.session = await invoke("apply_staged_edit_cmd", {
            sessionId: appState.session.id,
            editId: edit.id,
          });
          await refreshWorkspaceFiles();
          renderAll();
          openDiffsModal();
        };
        actions.appendChild(discBtn);
        actions.appendChild(applyBtn);
        card.appendChild(actions);
      }
      body.appendChild(card);
    }
  });
}

function openCustomizeModal() {
  const b = appState.bootstrap;
  openModal("Settings", (body) => {
    const ap = appState.appearance;
    const appearanceCard = document.createElement("div");
    appearanceCard.className = "card";

    const accentButtons = ACCENT_MODES.map(
      (a) =>
        `<button type="button" class="seg-btn ${ap.accent === a ? "active" : ""}" data-accent-pick="${a}">${a[0].toUpperCase() + a.slice(1)}</button>`,
    ).join("");
    const toneButtons = TONE_MODES.map(
      (t) =>
        `<button type="button" class="seg-btn ${ap.tone === t ? "active" : ""}" data-tone-pick="${t}">${t[0].toUpperCase() + t.slice(1)}</button>`,
    ).join("");
    const densityButtons = DENSITY_MODES.map(
      (d) =>
        `<button type="button" class="seg-btn ${ap.density === d ? "active" : ""}" data-density-pick="${d}">${d[0].toUpperCase() + d.slice(1)}</button>`,
    ).join("");
    const textButtons = TEXT_SIZES.map(
      (sz) =>
        `<button type="button" class="seg-btn ${ap.textSize === sz ? "active" : ""}" data-size-pick="${sz}">${sz[0].toUpperCase() + sz.slice(1)}</button>`,
    ).join("");

    appearanceCard.innerHTML = `
      <strong>Accent</strong>
      <div class="seg-group">${accentButtons}</div>
      <strong>Tone</strong>
      <div class="seg-group cols-2">${toneButtons}</div>
      <strong>Density</strong>
      <div class="seg-group cols-2">${densityButtons}</div>
      <strong>Sidebar</strong>
      <div class="seg-group cols-2">
        <button type="button" class="seg-btn ${ap.sidebar ? "active" : ""}" data-sidebar-pick="true">On</button>
        <button type="button" class="seg-btn ${!ap.sidebar ? "active" : ""}" data-sidebar-pick="false">Off</button>
      </div>
      <strong>Text size</strong>
      <div class="seg-group">${textButtons}</div>
    `;

    appearanceCard.querySelectorAll("[data-accent-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, accent: btn.getAttribute("data-accent-pick") });
        openCustomizeModal();
      };
    });
    appearanceCard.querySelectorAll("[data-tone-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, tone: btn.getAttribute("data-tone-pick") });
        openCustomizeModal();
      };
    });
    appearanceCard.querySelectorAll("[data-density-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, density: btn.getAttribute("data-density-pick") });
        openCustomizeModal();
      };
    });
    appearanceCard.querySelectorAll("[data-sidebar-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({
          ...appState.appearance,
          sidebar: btn.getAttribute("data-sidebar-pick") === "true",
        });
        openCustomizeModal();
      };
    });
    appearanceCard.querySelectorAll("[data-size-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, textSize: btn.getAttribute("data-size-pick") });
        openCustomizeModal();
      };
    });
    body.appendChild(appearanceCard);

    const harnessOptions = b.harnesses
      .map(
        (h) =>
          `<option value="${escapeHtml(h.id)}" ${h.id === b.config.harness_mode ? "selected" : ""}>${escapeHtml(h.name)}${h.available ? "" : " (not installed)"}</option>`,
      )
      .join("");
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <label><strong>Engine for Code mode</strong></label>
      <select id="cust-harness" class="select-field">${harnessOptions}</select>
      <label><strong>Things Keel should always know</strong></label>
      <textarea id="cust-inst" class="textarea-field" rows="3">${escapeHtml(b.config.custom_instructions)}</textarea>
      <label class="row"><input id="cust-auto" type="checkbox" ${b.config.auto_apply_edits ? "checked" : ""} /> Apply file changes without asking me first</label>
      <label><strong>Advanced: use your own llama-server program</strong></label>
      <input id="cust-llama" class="input mono" value="${escapeHtml(b.config.llama_server_path || "")}" placeholder="/path/to/llama-server" />
      <div class="row">
        <button id="cust-reset-welcome" type="button" class="btn btn-ghost btn-sm">Show the welcome screen again</button>
        <div class="spacer"></div>
        <button id="cust-save" class="btn btn-primary btn-sm">Save settings</button>
      </div>
    `;
    card.querySelector("#cust-reset-welcome").onclick = () => {
      try {
        localStorage.removeItem(WELCOME_STORAGE_KEY);
      } catch {
        // Ignore storage errors.
      }
      closeModal();
      welcome.show();
    };
    card.querySelector("#cust-save").onclick = async () => {
      await invoke("save_customize_settings", {
        update: {
          custom_instructions: card.querySelector("#cust-inst").value,
          auto_apply_edits: card.querySelector("#cust-auto").checked,
          harness_mode: card.querySelector("#cust-harness").value,
          llama_server_path: card.querySelector("#cust-llama").value || null,
        },
      });
      await refreshBootstrap();
      renderHeader();
      closeModal();
    };
    body.appendChild(card);
  });
}

let toastTimer;
function showToast(message) {
  const toast = document.getElementById("global-notice");
  toast.textContent = message;
  toast.classList.remove("hidden");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.add("hidden"), 8000);
}

function showNotice(message) {
  els.approvalSlot.innerHTML = `<div class="approval-card" role="status">${escapeHtml(message)}</div>`;
}

function autoSizePromptInput() {
  els.promptInput.style.height = "auto";
  els.promptInput.style.height = `${Math.min(els.promptInput.scrollHeight, 140)}px`;
}

function wireUi() {
  els.toggleSidebarBtn.onclick = () => {
    applyAppearance({ ...appState.appearance, sidebar: !appState.appearance.sidebar });
  };
  els.newChatBtn.onclick = () => createNewSession(MODES[appState.activeView]?.preset || "chat");
  els.newAgentBtn.onclick = els.newChatBtn.onclick;
  document.getElementById("thread-settings-btn").onclick = everyday.threadSettings;
  document.getElementById("new-project-btn").onclick = () => everyday.editProject();
  document.getElementById("memory-btn").onclick = () => everyday.preferences();
  let searchTimer;
  document.getElementById("thread-search").oninput = (event) => { clearTimeout(searchTimer); searchTimer = setTimeout(() => everyday.search(event.target.value), 200); };
  document.getElementById("attach-docs-btn").onclick = async () => {
    if (appState.streaming) return;
    const documents = await invoke("pick_documents");
    if (appState.pendingDocuments.length + documents.length > 8) { showToast("Attach up to 8 documents per message."); return; }
    appState.pendingDocuments.push(...documents); saveDraft(); renderAttachmentChips();
  };
  document.addEventListener("click", (event) => {
    const link = event.target.closest("[data-external-link]");
    if (link) { event.preventDefault(); invoke("open_external_link", { url: link.dataset.externalLink }).catch((error) => showToast(String(error))); }
  });
  document.addEventListener("keydown", (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") { event.preventDefault(); document.getElementById("thread-search").focus(); }
    if (event.key === "Escape") closeModal();
    if (event.key === "Tab" && !els.modalBackdrop.classList.contains("hidden")) {
      const focusable = [...els.modalBackdrop.querySelectorAll("button, input, textarea, select, a[href]")].filter((el) => !el.disabled && el.getClientRects().length);
      const first = focusable[0], last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  });

  document.querySelectorAll(".mode-nav-item").forEach((btn) => {
    btn.onclick = () => setActiveView(btn.getAttribute("data-view"));
  });

  document.querySelectorAll("[data-viewer-tab]").forEach((tab) => {
    tab.onclick = () => {
      appState.viewerTab = tab.getAttribute("data-viewer-tab");
      renderViewerPane();
    };
  });

  els.modelPillBtn.onclick = () => {
    const rt = appState.bootstrap.runtime;
    if (welcome.status().paused) welcome.show();
    else if (isRuntimeLive(rt) || appState.waking) setActiveView("models");
    else if (modelToWake(appState.bootstrap)) wakeAssistant();
    else if (!appState.bootstrap.models.some((m) => m.installed)) welcome.show();
    else setActiveView("models");
  };
  document.getElementById("open-customize-btn").onclick = openCustomizeModal;
  els.harnessPillBtn.onclick = openCustomizeModal;
  els.diffsPillBtn.onclick = openDiffsModal;
  document.getElementById("modal-close-btn").onclick = closeModal;

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && welcome.isOpen()) welcome.handleEscape();
  });

  els.chatScroll.addEventListener("click", async (e) => {
    const copyCode = e.target.closest("[data-copy-code]");
    if (copyCode) { await navigator.clipboard.writeText(copyCode.closest(".code-block").querySelector("code").textContent); copyCode.textContent = "Copied"; return; }
    const copyMessage = e.target.closest("[data-copy-message]");
    if (copyMessage) { const msg = appState.session.messages.find((m) => m.id === copyMessage.dataset.copyMessage); await navigator.clipboard.writeText(msg.content); copyMessage.textContent = "Copied"; return; }
    const source = e.target.closest("[data-message-source]");
    if (source) { const doc = appState.session.messages.flatMap((m) => m.attachments || []).find((d) => d.id === source.dataset.messageSource); if (doc) everyday.previewSource(doc); return; }
    const editMessage = e.target.closest("[data-edit-message], [data-retry-message]");
    if (editMessage && !appState.streaming) {
      const id = editMessage.dataset.editMessage || editMessage.dataset.retryMessage;
      const message = appState.session.messages.find((m) => m.id === id);
      const fork = await invoke("fork_thread", { id: appState.session.id, beforeMessageId: id });
      await refreshBootstrap(); await loadSession(fork.id);
      els.promptInput.value = message.content; appState.pendingDocuments = message.attachments || []; saveDraft(); renderAttachmentChips(); autoSizePromptInput();
      showToast("Editing a copy. Your original conversation is kept.");
      if (editMessage.dataset.retryMessage) await sendCurrentPrompt(); else els.promptInput.focus();
      return;
    }
    const approveBtn = e.target.closest("[data-edit-approve]");
    const rejectBtn = e.target.closest("[data-edit-reject]");
    if (!approveBtn && !rejectBtn) return;
    e.preventDefault();
    e.stopPropagation();
    if (!appState.session) return;

    if (approveBtn) {
      const editId = approveBtn.getAttribute("data-edit-approve");
      appState.session = await invoke("apply_staged_edit_cmd", {
        sessionId: appState.session.id,
        editId,
      });
      await refreshWorkspaceFiles();
      renderAll();
    } else if (rejectBtn) {
      const editId = rejectBtn.getAttribute("data-edit-reject");
      appState.session = await invoke("discard_staged_edit_cmd", {
        sessionId: appState.session.id,
        editId,
      });
      renderAll();
    }
  });

  els.workspacePillBtn.onclick = () => openWorkspace();
  els.composerWsChip.onclick = () => openWorkspace();

  els.toggleViewerBtn.onclick = () => {
    appState.viewerOpen = !appState.viewerOpen;
    if (appState.viewerOpen && !MODES[appState.activeView]) {
      appState.activeView = "code";
    }
    renderAll();
  };
  els.closeViewerBtn.onclick = () => {
    appState.viewerOpen = false;

    renderAll();
  };
  els.refreshViewerBtn.onclick = () => refreshWorkspaceFiles().then(loadViewerContent);
  els.viewerFileSelect.onchange = () => {
    appState.viewerFile = els.viewerFileSelect.value;
    loadViewerContent();
  };

  els.sendBtn.onclick = sendCurrentPrompt;
  els.stopBtn.onclick = () => invoke("stop_generation");
  els.promptInput.addEventListener("input", () => { autoSizePromptInput(); saveDraft(); });
  els.promptInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendCurrentPrompt();
    }
  });
}

window.addEventListener("unhandledrejection", (event) => {
  showToast(String(event.reason));
  event.preventDefault();
});

init().catch((err) => {
  window.__KEEL_STARTUP_ERROR__ = String(err);
  els.chatScroll.innerHTML = `<div class="empty-state"><h1>Keel Desktop</h1><p>${escapeHtml(String(err))}</p></div>`;
});
