import { createEveryday } from "./everyday.js";
import { createImageWorkspace } from "./images.js";
import {
  AUDIO_TRY_STT_INPUT_HELP,
  AUDIO_TRY_TTS_INPUT_HELP,
  AUDIO_TRY_TTS_SAMPLE,
  formatAudioTrySttSuccess,
  formatAudioTryTtsSuccess,
} from "./audio.js";
import { assistantStatus, connectedExternalLocation, friendlyRuntimeError, friendlySetupError, isFirstRun, isRemoteModelServer, modelLocation, modelTierLabel, modelToWake, serverDataNotice, speedSettingsCopy, answerTimePhrase, modelCheckIntro } from "./onboarding.js";
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
  estimateSessionTokens,
  filterModels,
  findReusableEmptySession,
  formatBytes,
  formatContextMeter,
  formatRelativeTime,
  formatTokenCount,
  modelFitLabel,
  normalizeAppearance,
  shortPath,
} from "./state.js";
import { renderCodeView, renderDiffHtml, renderDiffView, renderViewer, inlineWorkspaceImages } from "./viewers.js";
import { createWelcome } from "./welcome.js";

const tauri = window.__TAURI__?.core;
const events = window.__TAURI__?.event;

const APPEARANCE_STORAGE_KEY = "keel_appearance_v2";
const WELCOME_STORAGE_KEY = "keel_welcome_done";
const THINKING_STORAGE_KEY = "keel_thinking_v1";

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
  modelFilter: "all", // "all" | "installed" | "discover"
  discoverModels: [],
  discoverQuery: "",
  discoverLoading: false,
  discoverError: null,
  discoverOnlyFitting: true,
  discoverLoaded: false,
  streaming: false,
  streamingText: "",
  streamingThinking: "",
  streamingTools: [],
  downloadProgress: {},
  busyModelId: null,
  busyModelAction: null,
  modelSwitcherOpen: false,
  waking: false,
  sendWhenReady: false,
  lastAnswerSeconds: 0,
  lastContextTokens: 0,
  appUpdate: null,
  appUpdateDismissed: false,
  appUpdateBusy: false,
  appUpdateProgress: null,
  appearance: loadAppearance(),
};

const els = {
  sidebar: document.getElementById("sidebar"),
  appsList: document.getElementById("apps-list"),
  sessionsList: document.getElementById("sessions-list"),
  updateBanner: document.getElementById("update-banner"),
  sidebarRuntimeBox: document.getElementById("sidebar-runtime-box"),
  newChatBtn: document.getElementById("new-chat-btn"),
  newAgentBtn: document.getElementById("new-agent-btn"),
  toggleSidebarBtn: document.getElementById("toggle-sidebar-btn"),
  modelPillBtn: document.getElementById("model-pill-btn"),
  modelSwitcherMenu: document.getElementById("model-switcher-menu"),
  runtimeDot: document.getElementById("runtime-dot"),
  modelPillLabel: document.getElementById("model-pill-label"),
  workspacePillBtn: document.getElementById("workspace-pill-btn"),
  workspacePillLabel: document.getElementById("workspace-pill-label"),
  harnessPillBtn: document.getElementById("harness-pill-btn"),
  harnessPillLabel: document.getElementById("harness-pill-label"),
  diffsPillBtn: document.getElementById("diffs-pill-btn"),
  publishPrBtn: document.getElementById("publish-pr-btn"),
  diffsPillLabel: document.getElementById("diffs-pill-label"),
  toggleViewerBtn: document.getElementById("toggle-viewer-btn"),
  viewChatCode: document.getElementById("view-chat-code"),
  viewModels: document.getElementById("view-models"),
  viewTools: document.getElementById("view-tools"),
  viewLibrary: document.getElementById("view-library"),
  viewConnectors: document.getElementById("view-connectors"),
  chatScroll: document.getElementById("chat-scroll"),
  noticeSlot: document.getElementById("notice-slot"),
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
  contextMeterPill: document.getElementById("context-meter-pill"),
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
    const searchInput = document.getElementById("thread-search");
    if (searchInput) searchInput.value = "";
    const project = appState.bootstrap.projects?.find((p) => p.id === id);
    if (project?.workspace_path) appState.workspacePath = project.workspace_path;
    await createNewSession(project?.workspace_path ? "general" : "chat");
  },
  openArtifact: async (item) => {
    const preset = item.kind === "html" ? "websites" : item.kind === "csv" ? "sheets" : item.kind === "md" ? "docs" : "general";
    const previewKinds = new Set(["html", "htm", "csv", "md", "png", "jpg", "jpeg", "webp"]);
    appState.workspacePath = item.workspace_path;
    appState.activeProjectId = item.project_id;
    const existing = (appState.bootstrap?.sessions || []).find(
      (s) => s.workspace_path === item.workspace_path && modeForPreset(s.app_preset) === "work",
    );
    if (existing) {
      await loadSession(existing.id);
    } else {
      await createNewSession(preset);
    }
    appState.viewerFile = item.path;
    appState.viewerOpen = true;
    appState.viewerTab = previewKinds.has((item.kind || "").toLowerCase()) ? "preview" : "code";
    renderAll();
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
    await invoke("auto_start_model");
    appState.waking = false;
    await refreshBootstrap();
    renderAll();
    await sendQueuedPrompt();
    return true;
  } catch (err) {
    appState.waking = false;
    appState.sendWhenReady = false;
    renderAll();
    showNotice(`Your AI didn't start: ${friendlyRuntimeError(err)}`);
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
  web_search: "Search web",
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
  if (MODES[view]) {
    const currentPreset = appState.session?.app_preset;
    const returningFromWorkSubApp = view === "work" && appState.activeView === "work" && currentPreset && currentPreset !== "general";
    if (modeForPreset(currentPreset) !== view || returningFromWorkSubApp) {
      const recent = appState.bootstrap.sessions.find((s) =>
        returningFromWorkSubApp
          ? s.app_preset === "general" && (s.project_id || null) === appState.activeProjectId
          : modeForPreset(s.app_preset) === view && (s.project_id || null) === appState.activeProjectId,
      );
      if (recent) await loadSession(recent.id);
      else await createNewSession(MODES[view].preset);
    }
  }
  appState.activeView = view;
  if (view === "models") {
    await refreshBootstrap();
  }
  if (view === "connectors" && appState.bootstrap) {
    try {
      appState.bootstrap.mcp_servers = await invoke("refresh_mcp_servers", {
        workspacePath: appState.workspacePath,
      });
    } catch {
      // Keep existing list if probe fails transiently.
    }
  }
  if (view === "code") {
    appState.viewerOpen = hasViewerContent();
    appState.viewerTab = (appState.session?.staged_edits?.length || 0) > 0 || appState.session?.code_checkout ? "diff" : "code";
  }
  renderAll();
}

async function init() {
  applyAppearance(appState.appearance);

  if (events) {
    await events.listen("keel://preference-saved", () => showToast("Saved to preferences. You can edit or remove it in the sidebar."));
    await events.listen("keel://setup-stage", (event) => welcome.stage(event.payload));
    await events.listen("keel://model-fallback", (event) => {
      const p = event.payload;
      if (p?.from_name && p?.to_name) {
        showToast(`${p.from_name} couldn't start on this computer, so Keel switched to ${p.to_name} automatically.`);
      }
    });
    await events.listen("keel://image-job", () => images.onJob());
    await events.listen("keel://agent-event", (ev) => handleAgentEvent(ev.payload));
    await events.listen("tauri://drag-enter", () => {
      document.querySelector(".models-drop-zone")?.classList.add("drop-active");
      document.querySelector(".chat-column")?.classList.add("drop-active");
    });
    await events.listen("tauri://drag-leave", () => {
      document.querySelector(".models-drop-zone")?.classList.remove("drop-active");
      document.querySelector(".chat-column")?.classList.remove("drop-active");
    });
    await events.listen("tauri://drag-drop", async (ev) => {
      document.querySelector(".models-drop-zone")?.classList.remove("drop-active");
      document.querySelector(".chat-column")?.classList.remove("drop-active");
      const paths = ev.payload?.paths || [];
      if (!paths.length) return;
      const ggufs = paths.filter((p) => String(p).toLowerCase().endsWith(".gguf"));
      const others = paths.filter((p) => !String(p).toLowerCase().endsWith(".gguf"));
      if (ggufs.length) {
        let lastId = null;
        let lastName = "";
        for (const path of ggufs) {
          try {
            await invoke("import_local_gguf", { path });
            const base = String(path).split(/[\\/]/).pop() || "model.gguf";
            lastId = `imported:${base}`;
            lastName = base.replace(/\.gguf$/i, "");
          } catch (err) {
            showToast(friendlyRuntimeError(err));
          }
        }
        if (lastId) {
          await refreshBootstrap();
          appState.selectedModelInspectorId = lastId;
          appState.modelFilter = "all";
          setActiveView("models");
          renderAll();
          showToast(
            ggufs.length === 1
              ? `Added ${lastName}. Click Use to start it.`
              : `Added ${ggufs.length} local models.`,
          );
        }
      }
      if (others.length) {
        try {
          const documents = await invoke("import_documents_from_paths", { paths: others });
          appState.pendingDocuments.push(...documents);
          saveDraft();
          renderAttachmentChips();
        } catch (err) {
          showToast(friendlyRuntimeError(err));
        }
      }
    });
    await events.listen("keel://app-update-progress", (ev) => {
      appState.appUpdateProgress = ev.payload;
      renderUpdateBanner();
    });
    setTimeout(() => {
      checkAppUpdate({ silent: true });
    }, 2500);
    await events.listen("keel://download-progress", (ev) => {
      const p = ev.payload;
      appState.downloadProgress[p.model_id] = p;
      welcome.download(p);
      const pct = p.total_bytes ? Math.round((p.downloaded_bytes / p.total_bytes) * 100) : 0;
      const bar = document.getElementById(`dl-prog-${p.model_id}`);
      if (bar) {
        bar.textContent =
          p.done
            ? "Downloaded"
            : `${formatBytes(p.downloaded_bytes)}${pct ? ` (${pct}%)` : ""}`;
      }
      const fill = document.getElementById(`dl-bar-${p.model_id}`);
      if (fill) {
        fill.style.width = `${p.done ? 100 : pct}%`;
      }
      if (p.model_id?.startsWith("local:")) {
        const hfProg = document.getElementById("ws-hf-progress");
        if (hfProg) {
          hfProg.textContent = p.done
            ? "Downloaded"
            : `Downloading: ${formatBytes(p.downloaded_bytes)}${pct ? ` (${pct}%)` : ""}`;
        }
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
  const reusableId = findReusableEmptySession(appState.bootstrap?.sessions || [], {
    currentSession: appState.session,
    preset,
    workspacePath,
    projectId: appState.activeProjectId,
  });
  if (reusableId) {
    if (appState.session?.id !== reusableId) {
      await loadSession(reusableId);
      return;
    }
    appState.lastContextTokens = 0;
    appState.activeView = modeForPreset(preset);
    appState.viewerOpen = hasViewerContent();
    appState.viewerTab = appState.activeView === "code" ? "code" : "preview";
    renderAll();
    els.promptInput?.focus();
    return;
  }
  appState.lastContextTokens = 0;
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

function activeWorkspacePath() {
  if (appState.session?.app_preset === "code" && appState.session?.code_checkout?.isolated) {
    return appState.session.code_checkout.isolated;
  }
  return appState.workspacePath;
}

async function loadSession(id) {
  if (appState.streaming) return;
  saveDraft();
  appState.lastContextTokens = 0;
  appState.session = await invoke("load_session_cmd", { id });
  appState.activeProjectId = appState.session.project_id || null;
  appState.searchResults = null;
  const searchInput = document.getElementById("thread-search");
  if (searchInput) searchInput.value = "";
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
  const ws = activeWorkspacePath();
  if (!ws) {
    appState.workspaceFiles = [];
    return;
  }
  try {
    appState.workspaceFiles = await invoke("list_workspace_files_cmd", {
      workspacePath: ws,
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

async function revealKeelSessionLog() {
  const id = appState.session?.id;
  if (!id) return;
  try {
    const path = await invoke("reveal_keel_session_log", { sessionId: id });
    try {
      await navigator.clipboard.writeText(path);
      showToast(`Opened folder. Path copied: ${path}`);
    } catch {
      showToast(`Opened journal folder for ${path.split(/[/\\]/).pop() || "session"}`);
    }
  } catch (error) {
    showNotice(String(error));
  }
}

async function exportKeelSessionLog() {
  const id = appState.session?.id;
  if (!id) return;
  try {
    const path = await invoke("export_keel_session_log", { sessionId: id });
    showToast(`Exported journal to ${path}`);
  } catch (error) {
    showNotice(String(error));
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
    const searchInput = document.getElementById("thread-search");
    if (searchInput) searchInput.value = "";
    const recent = b.sessions.find((session) => (session.project_id || null) === appState.activeProjectId);
    if (recent) await loadSession(recent.id);
    else {
      const project = b.projects?.find((p) => p.id === appState.activeProjectId);
      if (project?.workspace_path) appState.workspacePath = project.workspace_path;
      await createNewSession(project?.workspace_path ? "general" : "chat");
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
        const sameProject = remaining.find((item) => (item.project_id || null) === appState.activeProjectId);
        if (sameProject) {
          await loadSession(sameProject.id);
        } else if (!appState.activeProjectId && remaining.length > 0) {
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
  const runningLabel =
    rt.state === "external"
      ? externalConnectedLabel(b, rt)
      : rt.state === "running" ? activeModel?.name || "Local model" : status.key === "setup" ? "Setting up your AI" : "No model loaded";
  const isLive = isRuntimeLive(rt);
  const fill = status.key === "setup" ? ` style="width:${Math.round(setup.fraction * 100)}%"` : "";
  const newThreadLabel = els.newAgentBtn?.querySelector("span");
  if (newThreadLabel) {
    newThreadLabel.textContent =
      appState.activeView === "code" ? "New code" : appState.activeView === "work" ? "New work" : "New chat";
  }
  const canOpenSetup = status.key === "setup" || status.key === "paused";
  els.sidebarRuntimeBox.classList.add("clickable");
  els.sidebarRuntimeBox.title = canOpenSetup ? "Open guided setup" : "Open Models";
  els.sidebarRuntimeBox.onclick = () => (canOpenSetup ? welcome.show() : setActiveView("models"));
  els.sidebarRuntimeBox.innerHTML = `
    <span class="status-dot ${status.key === "setup" || status.key === "waking" ? "starting" : rt.state}"></span>
    <div class="sidebar-runtime-meta">
      <div class="sidebar-runtime-name">${escapeHtml(runningLabel)}</div>
      ${status.key === "setup" ? `<div class="sidebar-runtime-bar"><div class="sidebar-runtime-fill active"${fill}></div></div>` : ""}
    </div>
    <span class="sidebar-runtime-state">${escapeHtml(status.label)}</span>
  `;
  renderUpdateBanner();
}

function updateProgressLabel() {
  const p = appState.appUpdateProgress;
  if (!p) return "Downloading update…";
  if (p.done) return "Installing & restarting…";
  if (p.total_bytes > 0) {
    const pct = Math.min(100, Math.round((p.downloaded_bytes / p.total_bytes) * 100));
    return `Downloading… ${pct}%`;
  }
  return `Downloading… ${formatBytes(p.downloaded_bytes || 0)}`;
}

async function checkAppUpdate({ silent = false } = {}) {
  try {
    const info = await invoke("check_app_update");
    appState.appUpdate = info;
    if (info?.available) appState.appUpdateDismissed = false;
    renderUpdateBanner();
    return info;
  } catch (err) {
    if (!silent) showToast(friendlyRuntimeError(err));
    return null;
  }
}

async function installOrDownloadAppUpdate() {
  const info = appState.appUpdate;
  if (!info?.available || appState.appUpdateBusy) return;
  if (!info.can_install_in_place) {
    try {
      await invoke("open_external_link", { url: info.release_url });
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
    return;
  }
  appState.appUpdateBusy = true;
  appState.appUpdateProgress = null;
  renderUpdateBanner();
  try {
    await invoke("install_app_update");
  } catch (err) {
    appState.appUpdateBusy = false;
    appState.appUpdateProgress = null;
    renderUpdateBanner();
    showToast(friendlyRuntimeError(err));
  }
}

function renderUpdateBanner() {
  const banner = els.updateBanner;
  if (!banner) return;
  const info = appState.appUpdate;
  if (!info?.available || appState.appUpdateDismissed) {
    banner.classList.add("hidden");
    banner.innerHTML = "";
    return;
  }
  const versionLabel = `v${String(info.version || "").replace(/^v/i, "")}`;
  const actionLabel = appState.appUpdateBusy
    ? updateProgressLabel()
    : info.can_install_in_place
      ? "Update & restart"
      : "Download update";
  banner.classList.remove("hidden");
  banner.innerHTML = `
    <div class="update-banner-head">
      <strong>Keel ${escapeHtml(versionLabel)} available</strong>
      ${appState.appUpdateBusy ? "" : '<button type="button" id="update-banner-dismiss" class="text-action" aria-label="Dismiss update notice">&times;</button>'}
    </div>
    <div class="update-banner-actions">
      <button type="button" id="update-banner-action" class="btn btn-primary btn-sm grow" ${appState.appUpdateBusy ? "disabled" : ""}>${escapeHtml(actionLabel)}</button>
    </div>
  `;
  banner.querySelector("#update-banner-action")?.addEventListener("click", installOrDownloadAppUpdate);
  banner.querySelector("#update-banner-dismiss")?.addEventListener("click", () => {
    appState.appUpdateDismissed = true;
    renderUpdateBanner();
  });
}

function renderHeader() {
  const b = appState.bootstrap;
  if (!b) return;
  const rt = b.runtime;
  const isBusy = appState.waking || appState.busyModelAction === "starting";
  els.runtimeDot.className = `status-dot ${isBusy ? "starting" : rt.state}`;
  const selected = b.models.find(
    (m) => m.id === (rt.active_model_id || b.config.selected_model_id),
  );
  const setup = welcome.status();
  const location = rt.state === "external" ? connectedExternalLocation(rt) : modelLocation(rt);
  els.modelPillBtn.title = isRuntimeLive(rt) ? `${location} · Click to switch AI` : "Click to choose or start your AI";
  els.modelPillBtn.setAttribute("aria-expanded", String(appState.modelSwitcherOpen));
  els.modelPillLabel.textContent = setup.busy || setup.paused ? assistantStatus({ runtime: rt, setup }).label : isBusy ? "Waking up your AI…" : rt.state === "stopped" ? (b.models.some((m) => m.installed && !m.damaged) ? "Start your AI" : b.models.some((m) => m.damaged) ? "Repair your AI" : "Set up your AI") : rt.state === "error" ? "Model needs attention" :
    rt.state === "external"
      ? externalConnectedLabel(b, rt)
      : selected
        ? selected.name
        : b.config.selected_model_id || "Select model";
  renderModelSwitcher();

  const shortWs = shortPath(appState.workspacePath);
  const activeProj = b.projects?.find((p) => p.id === appState.activeProjectId);
  els.workspacePillLabel.textContent = appState.workspacePath
    ? activeProj ? `${activeProj.name} · ${shortWs}` : shortWs
    : activeProj ? activeProj.name : "Open folder";
  const attachBtn = document.getElementById("attach-docs-btn");
  if (attachBtn) {
    attachBtn.title = b.ocr_available
      ? "Attach or drop/paste PDF, DOCX, text, CSV, code, or image (Chat can see image pixels; OCR ready as fallback)"
      : "Attach or drop/paste PDF, DOCX, text, CSV, code, or image (Chat can see image pixels; install Tesseract for OCR captions)";
  }

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
  renderContextMeter();
  if (!appState.streaming) {
    const took = answerTimePhrase(appState.lastAnswerSeconds);
    const live = `${location} · ${appState.session?.messages.length || 0} messages${took ? ` · ${took}` : ""}`;
    els.telemetryLeft.textContent = appState.sendWhenReady ? "Your message sends as soon as your AI is ready" : isBusy ? "Waking up your AI. This takes a few seconds." : isRuntimeLive(rt) ? live : "Your AI is off. Send a message to start it.";
  }
  const mode = modeForPreset(appState.session?.app_preset);
  els.promptInput.placeholder = MODES[mode].placeholder;
  els.composerWsChip.textContent = appState.workspacePath ? shortWs : "Open folder";
  els.composerWsChip.classList.toggle("hidden", mode === "chat");
  els.workspacePillBtn.classList.toggle("hidden", mode === "chat" && appState.activeView !== "tools");
  els.harnessPillBtn.classList.toggle("hidden", mode !== "code");
  els.toggleViewerBtn.classList.toggle("hidden", mode === "chat");
  els.diffsPillBtn.classList.toggle("hidden", mode === "chat" || counts.pending + counts.applied === 0);
  els.publishPrBtn.classList.toggle("hidden", mode !== "code" || !appState.workspacePath);
  els.newChatBtn.disabled = appState.streaming;
  els.newAgentBtn.disabled = appState.streaming;
}

function activeContextBudget(b) {
  if (!b) return 0;
  const rt = b.runtime;
  if (rt?.active_context_size > 0) return rt.active_context_size;
  const selected = b.models?.find((m) => m.id === (rt?.active_model_id || b.config?.selected_model_id));
  const modelMax = selected?.context_size || 8192;
  const configured = b.config?.context_window || 0;
  const desired = configured > 0 ? configured : (b.system_ram_gb >= 16 ? 16384 : 8192);
  return Math.min(desired, Math.max(modelMax, 2048));
}

function renderContextMeter() {
  const pill = els.contextMeterPill;
  const b = appState.bootstrap;
  if (!pill || !b) return;
  const used = appState.lastContextTokens || estimateSessionTokens(appState.session?.messages || []);
  const meter = formatContextMeter(used, activeContextBudget(b));
  if (!meter) {
    pill.textContent = "";
    pill.classList.add("hidden");
    pill.classList.remove("is-high");
    return;
  }
  pill.textContent = meter.label;
  pill.classList.remove("hidden");
  pill.classList.toggle("is-high", meter.isHigh);
  pill.title = meter.isHigh
    ? `Conversation memory is ${meter.percent}% full. Older turns summarize automatically when needed.`
    : `Conversation memory used (${meter.percent}%). Older turns summarize automatically when full.`;
}

function closeModelSwitcher() {
  if (!appState.modelSwitcherOpen) return;
  appState.modelSwitcherOpen = false;
  els.modelSwitcherMenu?.classList.add("hidden");
  els.modelPillBtn?.setAttribute("aria-expanded", "false");
}

async function switchToModel(modelId) {
  if (appState.busyModelId || appState.waking) return;
  closeModelSwitcher();
  appState.busyModelId = modelId;
  appState.busyModelAction = "starting";
  renderAll();
  try {
    await invoke("start_model", { modelId });
    await refreshBootstrap();
    const chosen = appState.bootstrap?.models?.find((m) => m.id === modelId);
    if (chosen) showToast(`${chosen.name} is ready.`);
  } catch (err) {
    showToast(friendlyRuntimeError(err));
  } finally {
    appState.busyModelId = null;
    appState.busyModelAction = null;
    renderAll();
  }
}

async function downloadCatalogModel(modelId, { repair = false } = {}) {
  if (appState.busyModelId) return;
  appState.busyModelId = modelId;
  appState.busyModelAction = repair ? "repairing" : "downloading";
  renderModelsWorkspace();
  try {
    await invoke("download_catalog_model", { modelId });
    await refreshBootstrap();
    const wasLive = isRuntimeLive(appState.bootstrap?.runtime);
    if (!wasLive) {
      appState.busyModelAction = "starting";
      renderAll();
      try {
        await invoke("start_model", { modelId });
        await refreshBootstrap();
        const chosen = appState.bootstrap?.models?.find((m) => m.id === modelId);
        showToast(`${chosen?.name || "Your AI"} is downloaded and ready.`);
      } catch (startErr) {
        showToast(`Downloaded, but couldn't start yet: ${friendlyRuntimeError(startErr)}`);
      }
    } else {
      showToast(repair ? "Model repaired and ready to use." : "Model downloaded.");
    }
  } catch (err) {
    showToast(friendlyRuntimeError(err));
  } finally {
    appState.busyModelId = null;
    appState.busyModelAction = null;
    renderAll();
  }
}

function confirmAction(title, message, confirmLabel, onConfirm) {
  openModal(title, (body) => {
    body.innerHTML = `
      <p>${escapeHtml(message)}</p>
      <div class="row" style="justify-content:flex-end;margin-top:8px">
        <button id="confirm-cancel-btn" type="button" class="btn btn-ghost btn-sm">Cancel</button>
        <button id="confirm-ok-btn" type="button" class="btn btn-rust btn-sm">${escapeHtml(confirmLabel)}</button>
      </div>
    `;
    body.querySelector("#confirm-cancel-btn").onclick = closeModal;
    body.querySelector("#confirm-ok-btn").onclick = async () => {
      closeModal();
      await onConfirm();
    };
  });
}

async function removeLocalModel(modelId) {
  if (appState.busyModelId) return;
  const model = appState.bootstrap?.models?.find((m) => m.id === modelId);
  const label = model ? `${model.name} (${formatBytes(model.bytes)})` : modelId;
  confirmAction(
    "Remove model from this computer?",
    `This deletes ${label} from disk. You can download it again later.`,
    "Remove model",
    async () => {
      if (appState.busyModelId) return;
      appState.busyModelId = modelId;
      appState.busyModelAction = "removing";
      renderModelsWorkspace();
      try {
        await invoke("delete_model", { modelId });
        await refreshBootstrap();
        showToast("Model removed from this computer.");
      } catch (err) {
        showToast(friendlyRuntimeError(err));
      } finally {
        appState.busyModelId = null;
        appState.busyModelAction = null;
        renderAll();
      }
    },
  );
}

/** Connected external pill/sidebar label: remote keeps cost language; loopback stays local-only. */
function externalConnectedLabel(b, rt) {
  const location = connectedExternalLocation({
    state: "external",
    base_url: rt?.base_url || b.config.external_base_url || "",
  });
  const name = b.config.external_model_name;
  const remote = isRemoteModelServer(rt?.base_url || b.config.external_base_url || "");
  if (remote) return name ? `${name} · ${location}` : location;
  return name || location;
}

function tierLabel(model) {
  const b = appState.bootstrap;
  const failed = b?.model_checks?.[model.id]?.passed === false;
  return modelTierLabel(failed ? { ...model, checkPassed: false } : model, b?.offered_model_id);
}

function checkResultHtml(report) {
  if (!report) return `<p class="muted">${escapeHtml(modelCheckIntro())}</p>`;
  const items = (report.items || [])
    .map((item) => `<li>${escapeHtml(item.detail || item.name || "")}</li>`)
    .join("");
  return `<p>${escapeHtml(report.summary || "")}</p>${items ? `<ul class="check-list">${items}</ul>` : ""}`;
}

function renderModelSwitcher() {
  const menu = els.modelSwitcherMenu;
  const b = appState.bootstrap;
  if (!menu || !b) return;
  menu.classList.toggle("hidden", !appState.modelSwitcherOpen);
  if (!appState.modelSwitcherOpen) return;

  const rt = b.runtime;
  const healthy = b.models.filter((m) => m.installed && !m.damaged);
  const damaged = b.models.filter((m) => m.damaged);
  const busy = Boolean(appState.busyModelId || appState.waking);

  let html = "";
  if (rt.state === "external") {
    const extUrl = rt.base_url || b.config.external_base_url || "";
    const extName = b.config.external_model_name || connectedExternalLocation({ state: "external", base_url: extUrl });
    html += `
      <div class="switcher-section-label">CONNECTED APP</div>
      <div class="switcher-item active">
        <span class="switcher-item-main">
          <strong>${escapeHtml(extName)}</strong>
          <span class="switcher-item-sub">${escapeHtml(serverDataNotice(extUrl))}</span>
        </span>
        <span class="switcher-check">✓</span>
      </div>
      <button type="button" class="switcher-item" id="switcher-use-builtin" ${busy ? "disabled" : ""}>
        <span class="switcher-item-main">
          <strong>Switch back to built-in AI</strong>
          <span class="switcher-item-sub">Run locally inside Keel</span>
        </span>
      </button>
      <div class="switcher-divider"></div>
    `;
  }

  if (healthy.length > 0) {
    html += `<div class="switcher-section-label">ON THIS COMPUTER</div>`;
    for (const m of healthy) {
      const isActive = rt.state === "running" && rt.active_model_id === m.id;
      const tier = tierLabel(m);
      const ctxLabel = isActive && rt.active_context_size ? ` · ${formatTokenCount(rt.active_context_size)} context` : "";
      html += `
        <button type="button" class="switcher-item ${isActive ? "active" : ""}" data-switch-model="${escapeHtml(m.id)}" role="menuitem" ${busy ? "disabled" : ""}>
          <span class="switcher-item-main">
            <strong>${escapeHtml(m.name)}</strong>
            <span class="switcher-item-sub">${tier ? `${escapeHtml(tier)} · ` : ""}${formatBytes(m.bytes)}${escapeHtml(ctxLabel)}</span>
          </span>
          <span class="switcher-check">${isActive ? "✓" : ""}</span>
        </button>
      `;
    }
  }

  if (damaged.length > 0) {
    html += `<div class="switcher-section-label">NEEDS ATTENTION</div>`;
    for (const m of damaged) {
      html += `
        <button type="button" class="switcher-item damaged" data-inspect-damaged="${escapeHtml(m.id)}" role="menuitem">
          <span class="switcher-item-main">
            <strong>${escapeHtml(m.name)}</strong>
            <span class="switcher-item-sub">Damaged download · click to repair</span>
          </span>
          <span class="model-badge warn">Repair</span>
        </button>
      `;
    }
  }

  html += `<div class="switcher-divider"></div>`;
  if (isRuntimeLive(rt)) {
    html += `<button type="button" class="switcher-item" id="switcher-stop" role="menuitem"><span>Turn AI off</span></button>`;
  }
  html += `<button type="button" class="switcher-item" id="switcher-discover" role="menuitem"><span>Discover or add .gguf…</span></button>`;
  html += `<button type="button" class="switcher-item" id="switcher-browse" role="menuitem"><span>Browse &amp; manage models…</span></button>`;

  menu.innerHTML = html;
  menu.querySelectorAll("[data-switch-model]").forEach((btn) => {
    btn.onclick = () => switchToModel(btn.dataset.switchModel);
  });
  menu.querySelectorAll("[data-inspect-damaged]").forEach((btn) => {
    btn.onclick = () => {
      closeModelSwitcher();
      appState.selectedModelInspectorId = btn.dataset.inspectDamaged;
      setActiveView("models");
    };
  });
  menu.querySelector("#switcher-use-builtin")?.addEventListener("click", async () => {
    closeModelSwitcher();
    try {
      await invoke("clear_external_endpoint");
      await refreshBootstrap();
      renderAll();
      if (modelToWake(appState.bootstrap)) await wakeAssistant();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  });
  menu.querySelector("#switcher-stop")?.addEventListener("click", async () => {
    closeModelSwitcher();
    try {
      await invoke("stop_runtime");
      await refreshBootstrap();
      renderAll();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  });
  menu.querySelector("#switcher-discover")?.addEventListener("click", () => {
    closeModelSwitcher();
    appState.modelFilter = "discover";
    setActiveView("models");
    if (!appState.discoverLoaded) loadDiscoverModels(appState.discoverQuery);
  });
  menu.querySelector("#switcher-browse")?.addEventListener("click", () => {
    closeModelSwitcher();
    setActiveView("models");
  });
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
    const journalBtns =
      preset.id === "code" && s.keel_session_log
        ? '<button class="btn btn-ghost" id="empty-reveal-cli-journal">Show journal</button><button class="btn btn-ghost" id="empty-export-cli-journal">Export journal</button>'
        : "";
    const boundaryNotice =
      preset.id === "code" && b.code_execution?.notice
        ? `<p id="code-boundary-notice" class="empty-status" role="status">${escapeHtml(b.code_execution.notice)}</p>`
        : "";
    const heading = preset.id === "chat" && !project ? "What can I help with?" : preset.title;
    els.chatScroll.innerHTML = `
      <div class="empty-state">
        ${project ? `<span class="eyebrow">${escapeHtml(project.name)}</span>` : ""}
        <h1>${escapeHtml(heading)}</h1>
        <p>${escapeHtml(preset.summary)}</p>
        ${action}${openCli}${journalBtns}${status}${boundaryNotice}
        <div class="starters-grid">${starters}</div>
      </div>
    `;
    els.chatScroll.querySelector("#chat-setup-btn")?.addEventListener("click", () => (canWake && !setupBusy && !setup.paused ? wakeAssistant() : welcome.show()));
    els.chatScroll.querySelector("#empty-open-folder")?.addEventListener("click", () => openWorkspace());
    els.chatScroll.querySelector("#empty-sample-folder")?.addEventListener("click", () => openWorkspace({ sample: true }));
    els.chatScroll.querySelector("#empty-open-cli-session")?.addEventListener("click", importKeelSession);
    els.chatScroll.querySelector("#empty-reveal-cli-journal")?.addEventListener("click", () => revealKeelSessionLog());
    els.chatScroll.querySelector("#empty-export-cli-journal")?.addEventListener("click", () => exportKeelSessionLog());
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
    html += `<div id="streaming-msg" class="msg assistant">${renderStreamingInnerHtml()}</div>`;
  }

  els.chatScroll.innerHTML = html;
  els.chatScroll.querySelectorAll("details[data-details-id]").forEach((d) => { if (details.has(d.dataset.detailsId)) d.open = details.get(d.dataset.detailsId); });
  els.chatScroll.scrollTop = shouldStick ? els.chatScroll.scrollHeight : oldTop;
}

let streamingRafId = null;
let streamingStartedAt = 0;

function renderStreamingInnerHtml() {
  const toolsHtml = appState.streamingTools.map(renderToolOrEditHtml).join("");
  const elapsed = streamingStartedAt ? Math.max(1, Math.round((Date.now() - streamingStartedAt) / 1000)) : 0;
  const thinkingSummary = appState.streamingText
    ? "Thinking"
    : `Thinking…${elapsed ? ` (${elapsed}s)` : ""}`;
  const thinkingHtml = appState.streamingThinking
    ? `<details class="thinking-box" data-details-id="live-thought"><summary>${thinkingSummary}</summary><pre class="mono">${escapeHtml(appState.streamingThinking)}</pre></details>`
    : "";
  const bodyContent = appState.streamingText
    ? renderMarkdown(appState.streamingText)
    : appState.streamingThinking
      ? `<p class="muted">Reasoning locally…</p>`
      : renderMarkdown("Working...");
  return `${thinkingHtml}${toolsHtml}<div class="bubble markdown-body">${bodyContent}</div>`;
}

function updateStreamingNode() {
  streamingRafId = null;
  if (!appState.streaming) return;
  const node = document.getElementById("streaming-msg");
  if (!node) {
    renderChat();
    return;
  }
  const shouldStick = els.chatScroll.scrollHeight - els.chatScroll.scrollTop - els.chatScroll.clientHeight < 100;
  const oldTop = els.chatScroll.scrollTop;
  const details = new Map([...node.querySelectorAll("details[data-details-id]")].map((d) => [d.dataset.detailsId, d.open]));
  node.innerHTML = renderStreamingInnerHtml();
  node.querySelectorAll("details[data-details-id]").forEach((d) => { if (details.has(d.dataset.detailsId)) d.open = details.get(d.dataset.detailsId); });
  els.chatScroll.scrollTop = shouldStick ? els.chatScroll.scrollHeight : oldTop;
}

function scheduleStreamingUpdate() {
  if (streamingRafId !== null) return;
  if (typeof requestAnimationFrame === "function") {
    streamingRafId = requestAnimationFrame(updateStreamingNode);
  } else {
    updateStreamingNode();
  }
}

function renderMessageHtml(msg) {
  const documents = (msg.attachments || []).map((doc) => `<button class="attachment-chip" data-message-source="${escapeHtml(doc.id)}">▤ ${escapeHtml(doc.name)}</button>`).join("");
  const disabled = appState.streaming ? "disabled" : "";
  if (msg.role === "user") {
    const editBtn = msg.unsent
      ? ""
      : `<div class="message-actions"><button class="text-action" data-edit-message="${escapeHtml(msg.id)}" ${disabled}>Edit</button></div>`;
    return `<div class="msg user" data-message-id="${escapeHtml(msg.id)}">${documents ? `<div class="message-sources">${documents}</div>` : ""}<div class="bubble">${escapeHtml(msg.content)}</div>${editBtn}</div>`;
  }
  const thinkingHtml = msg.thinking
    ? `<details class="thinking-box" data-details-id="thinking-${escapeHtml(msg.id)}"><summary>Thinking</summary><pre class="mono">${escapeHtml(msg.thinking)}</pre></details>` : "";
  const toolsHtml = (msg.tool_calls || []).map(renderToolOrEditHtml).join("");
  const lastUser = [...appState.session.messages].reverse().find((m) => m.role === "user");
  const isLastMsg = msg === appState.session.messages[appState.session.messages.length - 1];
  const canRetry = !msg.unsent && isLastMsg && !(msg.tool_calls?.length) && lastUser && !lastUser.unsent;
  const canContinue = !msg.unsent && isLastMsg && msg.status === "failed";
  const retrySendBtn = msg.unsent
    ? `<button class="text-action" data-retry-unsent ${disabled}>Retry send</button>`
    : "";
  const continueBtn = canContinue
    ? `<button class="text-action" data-continue-message ${disabled}>Continue</button>`
    : "";
  return `<div class="msg assistant" data-message-id="${escapeHtml(msg.id)}">${thinkingHtml}${toolsHtml}<div class="bubble markdown-body">${renderMarkdown(msg.content)}</div>
    ${msg.status === "failed" ? '<span class="reply-status">Reply interrupted · partial answer kept</span>' : ""}
    <div class="message-actions"><button class="text-action" data-copy-message="${escapeHtml(msg.id)}">Copy</button>${continueBtn}${retrySendBtn}${canRetry ? `<button class="text-action" data-retry-message="${escapeHtml(lastUser.id)}" ${disabled}>Try again</button>` : ""}</div></div>`;
}

function handleAgentEvent(ev) {
  if (!ev || !ev.type) return;
  if (ev.type === "thinking_delta") {
    appState.streamingThinking += ev.text;
    scheduleStreamingUpdate();
    return;
  } else if (ev.type === "content_delta") {
    appState.streamingText += ev.text;
    scheduleStreamingUpdate();
    return;
  } else if (ev.type === "compacted") {
    if (ev.note) showToast(ev.note);
    return;
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
    els.telemetryLeft.textContent = "Waiting for approval";
    renderApprovalCard(ev);
  } else if (ev.type === "turn_finished") {
    const fromServer = Number(ev.elapsed_secs);
    const fromClock = streamingStartedAt ? (Date.now() - streamingStartedAt) / 1000 : 0;
    appState.lastAnswerSeconds = fromServer > 0 ? fromServer : fromClock;
    const totalUsed = Number(ev.prompt_tokens || 0) + Number(ev.completion_tokens || 0);
    if (totalUsed > 0) appState.lastContextTokens = totalUsed;
    renderContextMeter();
    const tps = Number(ev.tokens_per_sec) > 0 ? ` · ${ev.tokens_per_sec.toFixed(1)} tok/s` : "";
    const toks = Number(ev.completion_tokens) > 0 ? ` · ${ev.completion_tokens} tokens` : "";
    const took = answerTimePhrase(appState.lastAnswerSeconds);
    els.telemetryLeft.textContent = `${modelLocation(appState.bootstrap.runtime)}${took ? ` · ${took}` : ""}${tps}${toks || " · Finished"}`;
  }
  updateStreamingNode();
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
  clearNotice();
  els.approvalSlot.innerHTML = "";

  const documents = appState.pendingDocuments;
  appState.pendingDocuments = [];
  els.promptInput.value = "";
  saveDraft();
  renderAttachmentChips();
  autoSizePromptInput();
  // Remove any previous failed-to-send placeholder pair before appending the fresh attempt.
  appState.session.messages = appState.session.messages.filter((m) => !m.unsent);
  appState.session.messages.push({
    id: `local-${Date.now()}`,
    role: "user",
    content: prompt,
    attachments: documents,
    thinking: null,
    tool_calls: [],
    unsent: true,
    created_at: Math.floor(Date.now() / 1000),
  });

  appState.streaming = true;
  streamingStartedAt = Date.now();
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
    saveDraft(); renderAttachmentChips(); autoSizePromptInput();
    appState.session.messages.push({
      id: `err-${Date.now()}`,
      role: "assistant",
      content: friendlyRuntimeError(err),
      thinking: null,
      tool_calls: [],
      unsent: true,
      created_at: Math.floor(Date.now() / 1000),
    });
  } finally {
    appState.streaming = false;
    streamingStartedAt = 0;
    if (streamingRafId !== null && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(streamingRafId);
      streamingRafId = null;
    }
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

function diffViewCallbacks() {
  const edits = appState.session?.staged_edits || [];
  return {
    edits,
    onSelect: (editId) => {
      appState.selectedEditId = editId;
      loadViewerContent();
    },
    onApprove: async (editId, btn) => {
      if (btn) btn.disabled = true;
      try {
        appState.session = await invoke("apply_staged_edit_cmd", { sessionId: appState.session.id, editId });
        await refreshWorkspaceFiles();
        renderAll();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        if (btn) btn.disabled = false;
      }
    },
    onReject: async (editId, btn) => {
      if (btn) btn.disabled = true;
      try {
        appState.session = await invoke("discard_staged_edit_cmd", { sessionId: appState.session.id, editId });
        renderAll();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        if (btn) btn.disabled = false;
      }
    },
  };
}

async function loadViewerContent() {
  if (appState.viewerTab === "diff") {
    const edits = appState.session?.staged_edits || [];
    const activeEdit =
      edits.find((e) => e.id === appState.selectedEditId) ||
      edits.find((e) => e.path === appState.viewerFile) ||
      edits[edits.length - 1] ||
      null;
    if (modeForPreset(appState.session?.app_preset) === "code") {
      await renderCodeWorktreePane(els.viewerContent, activeEdit);
      return;
    }
    renderDiffView(els.viewerContent, activeEdit, diffViewCallbacks());
    return;
  }

  const ws = activeWorkspacePath();
  if (!ws || !appState.viewerFile) {
    renderViewer(els.viewerContent, null, "");
    return;
  }
  try {
    if (appState.viewerFile.toLowerCase().endsWith(".png")) {
      const src = await invoke("read_workspace_png", { workspacePath: ws, relPath: appState.viewerFile });
      if (typeof src === "string" && src.startsWith("data:image/")) {
        els.viewerContent.innerHTML = `<img class="generated-image" alt="${escapeHtml(appState.viewerFile)}" src="${escapeHtml(src)}" />`;
      } else {
        els.viewerContent.innerHTML = `<div class="viewer-empty">Could not preview this image.</div>`;
      }
      return;
    }
    let content = await invoke("read_workspace_file_raw", {
      workspacePath: ws,
      relPath: appState.viewerFile,
    });
    if (appState.viewerTab === "code") {
      renderCodeView(els.viewerContent, appState.viewerFile, content);
    } else {
      if (/\.html?$/i.test(appState.viewerFile)) {
        content = await inlineWorkspaceImages(content, appState.viewerFile, (relPath) =>
          invoke("read_workspace_png", { workspacePath: ws, relPath }));
      }
      renderViewer(els.viewerContent, appState.viewerFile, content, {
        onExportHtml: async (name, html) => {
          try {
            const saved = await invoke("export_html_file", { fileName: name, html });
            if (saved) showToast(`Saved to ${saved}`);
          } catch (err) {
            showToast(friendlyRuntimeError(err));
          }
        },
      });
    }
  } catch (err) {
    els.viewerContent.innerHTML = `<div class="viewer-empty">${escapeHtml(String(err))}</div>`;
  }
}

async function renderCodeWorktreePane(container, activeEdit) {
  const session = appState.session;
  const checkout = session?.code_checkout;
  const wrap = document.createElement("div");
  wrap.className = "viewer-scroll diff-pane-wrap";

  const card = document.createElement("div");
  card.id = "code-worktree-card";
  card.className = "worktree-card";

  if (!checkout) {
    card.innerHTML = `
      <div class="worktree-head">
        <strong>Isolated checkout</strong>
        <span class="muted">Protects your working tree</span>
      </div>
      <p class="muted">Run Code edits and tests in a detached Git worktree from HEAD, review the full patch, and apply or undo cleanly.</p>
      <div class="worktree-actions">
        <button id="code-worktree-create" class="btn btn-primary btn-sm" ${appState.workspacePath ? "" : "disabled"}>Start isolated checkout</button>
      </div>
    `;
    card.querySelector("#code-worktree-create").onclick = async (event) => {
      event.target.disabled = true;
      try {
        appState.session = await invoke("create_code_worktree", { sessionId: session.id });
        showToast("Isolated checkout ready.");
        await refreshWorkspaceFiles();
        await loadViewerContent();
      } catch (err) {
        showNotice(String(err));
        event.target.disabled = false;
      }
    };
  } else {
    let patch = null;
    let patchError = null;
    try {
      patch = await invoke("review_code_worktree", { sessionId: session.id });
    } catch (err) {
      patchError = String(err);
    }
    const testRun = session.last_code_test;
    const applied = session.applied_code_patch;
    const baseShort = (checkout.base || "").slice(0, 8);
    card.innerHTML = `
      <div class="worktree-head">
        <div>
          <strong>Isolated checkout</strong>
          <span class="worktree-badge mono">base ${escapeHtml(baseShort)}</span>
          ${applied ? '<span class="worktree-badge worktree-badge-ok">Applied</span>' : ""}
        </div>
        <button id="code-worktree-remove" class="btn btn-ghost btn-sm">Discard checkout</button>
      </div>
      <div class="worktree-test-row">
        <input id="code-test-cmd" class="input grow mono" value="${escapeHtml(testRun?.command || "")}" placeholder="Test command (e.g. pytest -q or cargo test)" />
        <button id="code-test-run" class="btn btn-sm">Run test</button>
      </div>
      ${
        testRun
          ? `<div class="worktree-test-result ${testRun.passed ? "passed" : "failed"}">
              <div class="worktree-test-meta">
                <strong>${testRun.passed ? "Passed" : "Failed"}</strong>
                <span class="mono">exit ${testRun.exit_code} · ${escapeHtml(testRun.command)}</span>
              </div>
              ${testRun.output ? `<pre class="worktree-test-out mono">${escapeHtml(testRun.output)}</pre>` : ""}
            </div>`
          : ""
      }
      <div class="worktree-patch-head">
        <span>${
          patchError
            ? escapeHtml(patchError)
            : patch?.paths?.length
              ? `Full patch (${patch.paths.length} file${patch.paths.length === 1 ? "" : "s"}): ${escapeHtml(patch.paths.join(", "))}`
              : "No changes in isolated checkout yet."
        }</span>
        <div class="worktree-actions">
          <button id="code-patch-apply" class="btn btn-primary btn-sm" ${patch?.paths?.length ? "" : "disabled"}>Apply patch to repo</button>
          ${applied ? '<button id="code-patch-undo" class="btn btn-rust btn-sm">Undo applied patch</button>' : ""}
        </div>
      </div>
      ${patch?.diff ? `<div class="worktree-diff-wrap">${renderDiffHtml(patch.diff)}</div>` : ""}
    `;
    card.querySelector("#code-test-cmd")?.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        card.querySelector("#code-test-run")?.click();
      }
    });
    card.querySelector("#code-test-run").onclick = async () => {
      const cmd = card.querySelector("#code-test-cmd").value.trim();
      if (!cmd) return;
      card.querySelector("#code-test-run").disabled = true;
      try {
        appState.session = await invoke("run_code_test", { sessionId: session.id, command: cmd });
        await loadViewerContent();
      } catch (err) {
        showNotice(String(err));
        card.querySelector("#code-test-run").disabled = false;
      }
    };
    card.querySelector("#code-patch-apply").onclick = async (event) => {
      if (!patch) return;
      event.target.disabled = true;
      try {
        appState.session = await invoke("apply_code_worktree", { sessionId: session.id, patch });
        showToast("Reviewed patch applied to your repo.");
        await refreshWorkspaceFiles();
        await loadViewerContent();
      } catch (err) {
        showNotice(String(err));
        event.target.disabled = false;
      }
    };
    card.querySelector("#code-patch-undo")?.addEventListener("click", async (event) => {
      event.target.disabled = true;
      try {
        appState.session = await invoke("undo_code_worktree", { sessionId: session.id });
        showToast("Reverted Keel's applied patch.");
        await refreshWorkspaceFiles();
        await loadViewerContent();
      } catch (err) {
        showNotice(String(err));
        event.target.disabled = false;
      }
    });
    card.querySelector("#code-worktree-remove").onclick = (event) => {
      confirmAction(
        "Discard isolated checkout?",
        "This removes the temporary Git worktree and any unapplied changes inside it. Your original repository folder is not touched.",
        "Discard checkout",
        async () => {
          event.target.disabled = true;
          try {
            appState.session = await invoke("remove_code_worktree", { sessionId: session.id });
            showToast("Isolated checkout removed.");
            await refreshWorkspaceFiles();
            await loadViewerContent();
          } catch (err) {
            showNotice(String(err));
            event.target.disabled = false;
          }
        },
      );
    };
  }

  wrap.appendChild(card);
  if (activeEdit) {
    const stagedWrap = document.createElement("div");
    renderDiffView(stagedWrap, activeEdit, diffViewCallbacks());
    wrap.appendChild(stagedWrap);
  }
  container.innerHTML = "";
  container.appendChild(wrap);
}

async function loadDiscoverModels(query = appState.discoverQuery) {
  if (appState.discoverLoading) return;
  appState.discoverLoading = true;
  appState.discoverError = null;
  appState.discoverLoaded = true;
  renderModelsWorkspace();
  try {
    const trimmed = (query || "").trim();
    const list = await invoke("discover_models_cmd", {
      query: trimmed || null,
      limit: 24,
    });
    appState.discoverModels = Array.isArray(list) ? list : [];
  } catch (err) {
    appState.discoverError = friendlyRuntimeError(err);
  } finally {
    appState.discoverLoading = false;
    if (appState.activeView === "models") renderModelsWorkspace();
  }
}

async function downloadHuggingFaceModel(url, label = "model") {
  if (appState.busyModelId) return;
  const trimmed = (url || "").trim();
  if (!trimmed) {
    showToast("Paste a Hugging Face model link or owner/repo name first.");
    return;
  }
  appState.busyModelId = "hf-custom";
  appState.busyModelAction = "downloading";
  renderModelsWorkspace();
  const progEl = document.getElementById("ws-hf-progress");
  if (progEl) progEl.textContent = `Resolving ${label}…`;
  try {
    const beforeIds = new Set((appState.bootstrap?.models || []).map((m) => m.id));
    await invoke("download_huggingface_gguf", { url: trimmed });
    await refreshBootstrap();
    const added = (appState.bootstrap?.models || []).find((m) => !beforeIds.has(m.id));
    if (added) {
      appState.selectedModelInspectorId = added.id;
      appState.modelFilter = "all";
      if (!isRuntimeLive(appState.bootstrap?.runtime)) {
        appState.busyModelAction = "starting";
        renderAll();
        try {
          await invoke("start_model", { modelId: added.id });
          await refreshBootstrap();
          showToast(`${added.name} is downloaded and ready.`);
        } catch (startErr) {
          showToast(`Downloaded, but couldn't start yet: ${friendlyRuntimeError(startErr)}`);
        }
      } else {
        showToast(`${added.name} downloaded. Click Use to switch to it.`);
      }
    } else {
      showToast("Model downloaded. Click Use to start it.");
    }
  } catch (err) {
    showToast(friendlyRuntimeError(err));
  } finally {
    appState.busyModelId = null;
    appState.busyModelAction = null;
    renderAll();
  }
}

function renderModelsWorkspace() {
  const b = appState.bootstrap;
  if (!b) return;
  const rt = b.runtime;
  const isDiscover = appState.modelFilter === "discover";
  const filtered = isDiscover ? [] : filterModels(b.models, appState.modelSearchQuery, appState.modelFilter);
  const inspected =
    b.models.find((m) => m.id === appState.selectedModelInspectorId) ||
    b.models.find((m) => m.id === rt.active_model_id) ||
    b.models.find((m) => m.id === b.config.selected_model_id) ||
    b.models[0];
  const anyBusy = Boolean(appState.busyModelId || appState.waking);
  const installedFileNames = new Set(
    b.models.filter((m) => m.installed).map((m) => (m.file_name || "").toLowerCase()),
  );

  const rowsHtml = filtered
    .map((m) => {
      const isRunning = rt.active_model_id === m.id && rt.state === "running";
      const isSelected = inspected?.id === m.id;
      const isThisBusy = appState.busyModelId === m.id;
      const tier = tierLabel(m);
      const badgeClass = m.damaged ? "model-badge warn" : isRunning ? "model-badge live" : "model-badge";
      const badgeText = m.damaged ? "Needs repair" : isRunning ? "In use" : tier;
      const fit = !m.damaged
        ? modelFitLabel(m.bytes, b.system_ram_gb, b.computer?.vram_total_mib || 0)
        : "";
      const subText = m.damaged
        ? m.damage_reason || "This download looks incomplete or damaged."
        : `${m.detail} · ${formatBytes(m.bytes)}${fit ? ` · ${fit}` : ""}`;
      const isDownloading = isThisBusy && ["downloading", "repairing"].includes(appState.busyModelAction);
      let actionHtml = "";
      if (m.damaged) {
        actionHtml = `
          <div class="model-row-actions">
            <button class="pill-action-get" data-repair-model="${escapeHtml(m.id)}" ${anyBusy ? "disabled" : ""}>${isThisBusy && appState.busyModelAction === "repairing" ? "Repairing…" : "Repair"}</button>
            <button class="btn btn-ghost btn-sm" data-del-model="${escapeHtml(m.id)}" ${anyBusy ? "disabled" : ""}>Remove</button>
          </div>
        `;
      } else if (m.installed) {
        const label = isThisBusy && appState.busyModelAction === "starting" ? "Starting…" : isRunning ? "In use" : "Use";
        actionHtml = `<button class="pill-action-use" data-start-model="${escapeHtml(m.id)}" ${anyBusy || isRunning ? "disabled" : ""}>${label}</button>`;
      } else {
        const label = isThisBusy && appState.busyModelAction === "downloading" ? "Downloading…" : "Download";
        actionHtml = `<button class="pill-action-get" data-dl-model="${escapeHtml(m.id)}" ${anyBusy ? "disabled" : ""}>${label}</button>`;
      }
      const prog = appState.downloadProgress[m.id];
      const progPct = prog?.total_bytes ? Math.round((prog.downloaded_bytes / prog.total_bytes) * 100) : 0;
      const progLabel =
        isDownloading && prog
          ? prog.done
            ? "Downloaded"
            : `${formatBytes(prog.downloaded_bytes)}${progPct ? ` (${progPct}%)` : ""}`
          : "";
      const progWidth = isDownloading && prog ? (prog.done ? 100 : progPct) : 0;
      return `
        <div class="model-clean-row ${isSelected ? "active" : ""} ${m.damaged ? "damaged" : ""}" data-select-model="${escapeHtml(m.id)}">
          <div class="grow">
            <div class="model-clean-title">
              <span>${escapeHtml(m.name)}</span>
              ${badgeText ? `<span class="${badgeClass}">${escapeHtml(badgeText)}</span>` : ""}
            </div>
            <div class="model-clean-sub">
              ${escapeHtml(subText)}
              <span id="dl-prog-${escapeHtml(m.id)}" style="color:var(--accent);margin-left:6px">${escapeHtml(progLabel)}</span>
            </div>
            ${
              isDownloading
                ? `<div class="model-dl-track"><div id="dl-bar-${escapeHtml(m.id)}" class="model-dl-fill" style="width:${progWidth}%"></div></div>`
                : ""
            }
          </div>
          ${actionHtml}
        </div>
      `;
    })
    .join("");

  let discoverHtml = "";
  if (isDiscover) {
    const visibleDiscover = appState.discoverModels.filter((item) => {
      if (!appState.discoverOnlyFitting) return true;
      if (!item.bytes) return true;
      const fit = modelFitLabel(item.bytes, b.system_ram_gb, b.computer?.vram_total_mib || 0);
      return fit !== "Heavy for your RAM";
    });
    if (appState.discoverLoading) {
      discoverHtml = `<div class="card"><span class="muted">Checking trending GGUF models on Hugging Face…</span></div>`;
    } else if (appState.discoverError) {
      discoverHtml = `<div class="card"><div class="muted">${escapeHtml(appState.discoverError)}</div><div><button id="discover-retry-btn" class="btn btn-sm">Try again</button></div></div>`;
    } else if (!visibleDiscover.length) {
      discoverHtml = `<div class="card"><span class="muted">${
        appState.discoverOnlyFitting && appState.discoverModels.length
          ? "No trending models fit this computer's memory with the fit filter on."
          : "No GGUF models found. Try another search."
      }</span></div>`;
    } else {
      discoverHtml = visibleDiscover
        .map((item) => {
          const fit = item.bytes
            ? modelFitLabel(item.bytes, b.system_ram_gb, b.computer?.vram_total_mib || 0)
            : "";
          const already = installedFileNames.has((item.file_name || "").toLowerCase());
          const sizeText = item.bytes ? formatBytes(item.bytes) : "Size unknown";
          const metaParts = [
            item.author,
            item.quantization,
            sizeText,
            fit || null,
            item.likes ? `${item.likes} likes` : null,
          ].filter(Boolean);
          return `
            <div class="model-clean-row">
              <div class="grow">
                <div class="model-clean-title">
                  <span>${escapeHtml(item.name)}</span>
                  <span class="model-badge">${escapeHtml(item.quantization)}</span>
                </div>
                <div class="model-clean-sub">${escapeHtml(metaParts.join(" · "))}</div>
                <div class="model-clean-sub mono" style="font-size:11px;opacity:.75">${escapeHtml(item.file_name)}</div>
              </div>
              ${
                already
                  ? `<span class="model-badge live">Installed</span>`
                  : `<button class="pill-action-get" data-discover-dl="${escapeHtml(item.resolve_url)}" data-discover-name="${escapeHtml(item.name)}" ${anyBusy ? "disabled" : ""}>${appState.busyModelId === "hf-custom" ? "Downloading…" : "Download"}</button>`
              }
            </div>
          `;
        })
        .join("");
    }
  }

  const isRunningExternal = rt.state === "external";
  const isRunningLocal = rt.state === "running";
  const inspectedStatus = !inspected
    ? ""
    : inspected.damaged
      ? "Needs repair"
      : inspected.installed
        ? `${formatBytes(inspected.bytes)} · on this computer`
        : `${formatBytes(inspected.bytes)} download`;

  const totalDownloadedBytes = b.models
    .filter((m) => m.installed)
    .reduce((acc, m) => acc + (m.bytes || 0), 0);
  const storageText = [
    b.system_ram_gb ? `${b.system_ram_gb} GB RAM detected` : null,
    totalDownloadedBytes > 0 ? `${formatBytes(totalDownloadedBytes)} on disk` : "No models downloaded yet",
  ]
    .filter(Boolean)
    .join(" · ");

  els.viewModels.innerHTML = `
    <div class="models-main-col">
      <div class="model-setup-row"><div><strong>Not sure which one?</strong><p class="muted">Keel picks the right model for this computer and sets everything up automatically.</p></div><button id="models-setup-btn" class="btn ${isRunningLocal || isRunningExternal ? "" : "btn-primary "}btn-sm">Set it up for me</button></div>

      <div class="card models-drop-zone">
        <div class="row" style="justify-content:space-between;flex-wrap:wrap">
          <div>
            <strong>Bring your own model (.gguf)</strong>
            <div class="muted" style="font-size:12px">Drop a .gguf file anywhere on this window, scan ~/Models, or paste a Hugging Face link or owner/repo.</div>
          </div>
          <div class="row">
            <button id="ws-local-pick" type="button" class="btn btn-primary btn-sm">Choose .gguf…</button>
            <button id="ws-scan-models" type="button" class="btn btn-sm">Scan ~/Models</button>
          </div>
        </div>
        <div class="row">
          <input id="ws-hf-url" class="input grow" placeholder="Paste Hugging Face link or repo (e.g. prism-ml/Ternary-Bonsai-2.5-27B-GGUF)" />
          <button id="ws-hf-dl" class="btn btn-sm" ${anyBusy ? "disabled" : ""}>${appState.busyModelId === "hf-custom" ? "Downloading…" : "Download"}</button>
        </div>
        <p id="ws-hf-progress" class="muted" style="margin:0;min-height:14px;font-size:12px"></p>
      </div>

      <div class="models-storage-bar">
        <span>${escapeHtml(storageText)}</span>
        <button id="models-open-folder" type="button" class="btn btn-ghost btn-sm">Open folder</button>
      </div>
      <div class="models-search-bar">
        ${
          isDiscover
            ? `<input id="discover-search-in" class="input grow" value="${escapeHtml(appState.discoverQuery)}" placeholder="Search Hugging Face GGUF models (e.g. bonsai 2.5, inkling, qwen)…" />
               <button id="discover-search-btn" type="button" class="btn btn-sm" ${appState.discoverLoading ? "disabled" : ""}>Search</button>`
            : `<input id="models-search-in" class="input grow" value="${escapeHtml(appState.modelSearchQuery)}" placeholder="Search models" />`
        }
        <div class="models-filter-pills">
          ${[
            ["all", "All"],
            ["installed", "Downloaded"],
            ["discover", "Discover"],
          ]
            .map(
              ([f, label]) =>
                `<button class="viewer-tab ${appState.modelFilter === f ? "active" : ""}" data-mfilter="${f}">${label}</button>`,
            )
            .join("")}
        </div>
      </div>

      ${
        isDiscover
          ? `<div class="row" style="justify-content:space-between;font-size:12.5px">
               <label class="row" style="gap:6px;cursor:pointer;color:var(--muted)">
                 <input id="discover-only-fit" type="checkbox" ${appState.discoverOnlyFitting ? "checked" : ""} />
                 <span>Only show models that fit this computer</span>
               </label>
               <button id="discover-refresh-btn" type="button" class="btn btn-ghost btn-sm" ${appState.discoverLoading ? "disabled" : ""}>Refresh trending</button>
             </div>`
          : ""
      }

      <div class="model-list-clean">
        ${
          isDiscover
            ? discoverHtml
            : rowsHtml || `<div class="card">No models match "${escapeHtml(appState.modelSearchQuery)}".</div>`
        }
      </div>

      <details class="card">
        <summary class="import-gguf-toggle">Connect an external server (Ollama, LM Studio, or local file path)</summary>
        <div class="row" style="margin-top:8px">
          <span class="muted">Quick fill:</span>
          <button type="button" class="btn btn-sm" data-ext-preset="http://127.0.0.1:11434/v1">Ollama</button>
          <button type="button" class="btn btn-sm" data-ext-preset="http://127.0.0.1:1234/v1">LM Studio</button>
          ${isRunningExternal ? '<button type="button" id="ws-ext-clear" class="btn btn-ghost btn-sm">Switch back to built-in AI</button>' : ""}
        </div>
        <div class="row">
          <input id="ws-ext-url" class="input grow mono" value="${escapeHtml(b.config.external_base_url || "")}" placeholder="http://127.0.0.1:11434/v1" />
          <input id="ws-ext-name" class="input" style="max-width:160px" value="${escapeHtml(b.config.external_model_name || "")}" placeholder="Model name (optional)" />
          <button id="ws-ext-use" class="btn btn-primary btn-sm">Connect</button>
        </div>
        <div class="row" style="margin-top:6px">
          <input id="ws-ext-key" class="input grow mono" type="password" autocomplete="off" placeholder="API key (optional)" />
        </div>
        <label class="row" style="margin-top:6px;align-items:flex-start;gap:8px;font-size:12.5px;color:var(--muted)">
          <input id="ws-ext-remote" type="checkbox" style="margin-top:2px" />
          <span>This server is not on this computer</span>
        </label>
        <p class="muted" id="ws-ext-notice">${serverDataNotice(b.config.external_base_url || "")}</p>
        <div class="row">
          <input id="ws-local-gguf" class="input grow mono" placeholder="Or paste a local .gguf file path" />
          <button id="ws-local-imp" class="btn btn-sm">Import path</button>
        </div>
      </details>
    </div>

    <aside class="models-inspector-col">
      <div>
        <div class="sidebar-section-label">SELECTED</div>
        <h3 style="margin:6px 0 2px;font-size:17px">${escapeHtml(inspected ? inspected.name : "Built-in AI")}</h3>
        <div style="font-size:12px;color:var(--muted)">${escapeHtml(inspectedStatus)}</div>
      </div>

      <p class="muted">${inspected ? escapeHtml(inspected.damaged ? inspected.damage_reason || "This model file is incomplete or damaged. Click Repair to download a clean copy." : inspected.detail) : ""}</p>
      <p class="muted">Chat, Work, and Code all use the model you pick here.</p>
      <div class="card">
        <div class="row" style="font-size:12px;color:var(--accent)">
          <span class="status-dot ${rt.state}"></span>
          <span>${
            isRunningExternal
              ? escapeHtml(serverDataNotice(rt.base_url || b.config.external_base_url || ""))
              : isRunningLocal
                ? escapeHtml(modelLocation(rt))
                : rt.state === "error" && rt.error
                  ? escapeHtml(rt.error)
                  : "Off"
          }</span>
        </div>
        ${
          rt.state === "error" && rt.error_details
            ? `<details style="margin-top:6px">
                 <summary class="import-gguf-toggle" style="font-size:12px">Engine log details</summary>
                 <pre class="mono" style="margin:6px 0 0;padding:8px;border-radius:8px;background:var(--ink);font-size:11px;white-space:pre-wrap;max-height:180px;overflow:auto">${escapeHtml(rt.error_details)}</pre>
               </details>`
            : ""
        }
        <div class="row" style="margin-top:4px">
          ${
            isRunningLocal || isRunningExternal
              ? `<button id="insp-stop-rt" class="btn btn-sm grow" ${anyBusy ? "disabled" : ""}>Turn off</button>`
              : inspected?.damaged
                ? `<button id="insp-repair-rt" class="btn btn-primary btn-sm grow" ${anyBusy ? "disabled" : ""}>Repair download</button>`
                : inspected?.installed
                  ? `<button id="insp-start-rt" class="btn btn-primary btn-sm grow" ${anyBusy ? "disabled" : ""}>Start</button>`
                  : ""
          }
        </div>
        ${
          inspected && (inspected.installed || inspected.damaged)
            ? `<button id="insp-delete-model" class="btn btn-ghost btn-sm" ${anyBusy ? "disabled" : ""}>Remove from this computer</button>`
            : ""
        }
      </div>
    </aside>
  `;

  const modelsSetupBtn = document.getElementById("models-setup-btn");
  if (modelsSetupBtn) modelsSetupBtn.onclick = () => welcome.show();
  document.getElementById("models-open-folder")?.addEventListener("click", async () => {
    try { await invoke("open_models_folder"); }
    catch (err) { showToast(friendlyRuntimeError(err)); }
  });
  const searchIn = document.getElementById("models-search-in");
  if (searchIn) {
    searchIn.oninput = () => {
      const start = searchIn.selectionStart ?? searchIn.value.length;
      const end = searchIn.selectionEnd ?? start;
      appState.modelSearchQuery = searchIn.value;
      renderModelsWorkspace();
      const reFocus = document.getElementById("models-search-in");
      reFocus?.focus();
      reFocus?.setSelectionRange(start, end);
    };
  }
  const discoverSearchIn = document.getElementById("discover-search-in");
  if (discoverSearchIn) {
    discoverSearchIn.oninput = () => {
      appState.discoverQuery = discoverSearchIn.value;
    };
    discoverSearchIn.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        loadDiscoverModels(discoverSearchIn.value);
      }
    };
  }
  document.getElementById("discover-search-btn")?.addEventListener("click", () => {
    loadDiscoverModels(appState.discoverQuery);
  });
  document.getElementById("discover-refresh-btn")?.addEventListener("click", () => {
    loadDiscoverModels(appState.discoverQuery);
  });
  document.getElementById("discover-retry-btn")?.addEventListener("click", () => {
    loadDiscoverModels(appState.discoverQuery);
  });
  document.getElementById("discover-only-fit")?.addEventListener("change", (e) => {
    appState.discoverOnlyFitting = Boolean(e.target.checked);
    renderModelsWorkspace();
  });
  els.viewModels.querySelectorAll("[data-discover-dl]").forEach((btn) => {
    btn.onclick = () =>
      downloadHuggingFaceModel(
        btn.getAttribute("data-discover-dl"),
        btn.getAttribute("data-discover-name") || "model",
      );
  });
  els.viewModels.querySelectorAll("[data-mfilter]").forEach((btn) => {
    btn.onclick = () => {
      appState.modelFilter = btn.getAttribute("data-mfilter");
      if (appState.modelFilter === "discover" && !appState.discoverLoaded) {
        loadDiscoverModels(appState.discoverQuery);
      } else {
        renderModelsWorkspace();
      }
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
    btn.onclick = () => switchToModel(btn.getAttribute("data-start-model"));
  });
  els.viewModels.querySelectorAll("[data-dl-model]").forEach((btn) => {
    btn.onclick = () => downloadCatalogModel(btn.getAttribute("data-dl-model"));
  });
  els.viewModels.querySelectorAll("[data-repair-model]").forEach((btn) => {
    btn.onclick = () => downloadCatalogModel(btn.getAttribute("data-repair-model"), { repair: true });
  });
  els.viewModels.querySelectorAll("[data-del-model]").forEach((btn) => {
    btn.onclick = () => removeLocalModel(btn.getAttribute("data-del-model"));
  });
  els.viewModels.querySelectorAll("[data-ext-preset]").forEach((btn) => {
    btn.onclick = () => {
      const urlInput = document.getElementById("ws-ext-url");
      urlInput.value = btn.getAttribute("data-ext-preset");
      document.getElementById("ws-ext-notice").textContent = serverDataNotice(urlInput.value);
      const remote = document.getElementById("ws-ext-remote");
      if (remote) remote.checked = isRemoteModelServer(urlInput.value);
    };
  });

  document.getElementById("insp-stop-rt")?.addEventListener("click", async () => {
    try {
      await invoke("stop_runtime");
      await refreshBootstrap();
      renderAll();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  });
  document.getElementById("insp-start-rt")?.addEventListener("click", () => {
    if (inspected) switchToModel(inspected.id);
  });
  document.getElementById("insp-repair-rt")?.addEventListener("click", () => {
    if (inspected) downloadCatalogModel(inspected.id, { repair: true });
  });
  document.getElementById("insp-delete-model")?.addEventListener("click", () => {
    if (inspected) removeLocalModel(inspected.id);
  });
  document.getElementById("ws-ext-clear")?.addEventListener("click", async () => {
    try {
      await invoke("clear_external_endpoint");
      await refreshBootstrap();
      renderAll();
      if (modelToWake(appState.bootstrap)) await wakeAssistant();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  });

  document.getElementById("ws-hf-dl").onclick = async () => {
    const url = document.getElementById("ws-hf-url").value.trim();
    await downloadHuggingFaceModel(url, "model");
  };
  document.getElementById("ws-scan-models").onclick = async () => {
    try {
      const res = await invoke("scan_user_models_folder");
      await refreshBootstrap();
      if (res.added_ids?.length) {
        appState.selectedModelInspectorId = res.added_ids[0];
        appState.modelFilter = "all";
        renderAll();
        showToast(`Added ${res.added_ids.length} model${res.added_ids.length === 1 ? "" : "s"} from ${res.folder}.`);
      } else {
        showToast(`No new .gguf models found in ${res.folder}.`);
      }
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  };
  document.getElementById("ws-local-pick").onclick = async () => {
    try {
      const picked = await invoke("pick_model_file");
      if (!picked) return;
      await invoke("import_local_gguf", { path: picked });
      const base = String(picked).split(/[\\/]/).pop() || "model.gguf";
      await refreshBootstrap();
      appState.selectedModelInspectorId = `imported:${base}`;
      appState.modelFilter = "all";
      renderAll();
      showToast(`Added ${base.replace(/\.gguf$/i, "")}. Click Use to start it.`);
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  };
  document.getElementById("ws-local-imp").onclick = async () => {
    try {
      const rawPath = document.getElementById("ws-local-gguf").value.trim();
      await invoke("import_local_gguf", { path: rawPath });
      const base = rawPath.split(/[\\/]/).pop() || "model.gguf";
      await refreshBootstrap();
      appState.selectedModelInspectorId = `imported:${base}`;
      appState.modelFilter = "all";
      renderAll();
      showToast(`Added ${base.replace(/\.gguf$/i, "")}. Click Use to start it.`);
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  };
  document.getElementById("ws-ext-use").onclick = async () => {
    try {
      const url = document.getElementById("ws-ext-url").value;
      const keyInput = document.getElementById("ws-ext-key");
      const apiKey = keyInput?.value ?? "";
      await invoke("set_external_endpoint", {
        baseUrl: url,
        modelName: document.getElementById("ws-ext-name").value || null,
        confirmRemote: !!document.getElementById("ws-ext-remote")?.checked,
        apiKey: apiKey.length ? apiKey : null,
      });
      if (keyInput) keyInput.value = "";
      await refreshBootstrap();
      renderAll();
      showToast("Connected.");
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  };
  document.getElementById("ws-ext-url").oninput = (event) => {
    document.getElementById("ws-ext-notice").textContent = serverDataNotice(event.target.value);
    const remote = document.getElementById("ws-ext-remote");
    if (remote && isRemoteModelServer(event.target.value)) remote.checked = true;
  };
}

function renderConnectorsWorkspace() {
  const b = appState.bootstrap;
  if (!b) return;
  const servers = b.mcp_servers || [];
  const hasUntrustedWorkspace = servers.some((srv) => srv.scope === "workspace" && srv.trusted === false);
  const hasTrustedWorkspace = servers.some((srv) => srv.scope === "workspace" && srv.trusted !== false);

  const trustBannerHtml =
    appState.workspacePath && hasUntrustedWorkspace
      ? `
      <div class="card mcp-trust-banner">
        <div class="row">
          <strong class="grow">Review folder MCP tools before running</strong>
          <button id="mcp-trust-ws-btn" class="btn btn-primary btn-sm">Trust this folder</button>
        </div>
        <div style="font-size:12.5px;color:var(--muted);margin-top:4px">
          This folder defines custom tools in <code>.keel/mcp.json</code>. Keel will not launch them until you inspect the commands below and trust this folder.
        </div>
      </div>
    `
      : appState.workspacePath && hasTrustedWorkspace
        ? `
      <div class="card">
        <div class="row">
          <strong class="grow">Folder MCP tools trusted</strong>
          <button id="mcp-revoke-ws-btn" class="btn btn-ghost btn-sm">Revoke trust</button>
        </div>
        <div style="font-size:12.5px;color:var(--muted);margin-top:4px">
          Tools in <code>.keel/mcp.json</code> are allowed to run for this folder.
        </div>
      </div>
    `
        : "";

  const serversHtml = servers
    .map((srv) => {
      const scope = srv.scope === "workspace" ? "workspace" : "global";
      const scopeLabel = scope === "workspace" ? "Folder" : "Global";
      const untrusted = scope === "workspace" && srv.trusted === false;
      const dotClass = untrusted ? "starting" : srv.error ? "error" : srv.enabled ? "ready" : "";
      const tools = Array.isArray(srv.tools) ? srv.tools : [];
      const toolsHtml = tools.length
        ? `
          <details class="mcp-tools-list">
            <summary>${tools.length} ${tools.length === 1 ? "tool" : "tools"} discovered</summary>
            <ul>
              ${tools
                .map(
                  (t) =>
                    `<li><span class="mono">${escapeHtml(t.name)}</span>${t.description ? ` — ${escapeHtml(t.description)}` : ""}</li>`,
                )
                .join("")}
            </ul>
          </details>
        `
        : "";
      const errorHtml = srv.error
        ? `<div class="mcp-error">${escapeHtml(srv.error)}</div>`
        : "";
      return `
      <div class="card">
        <div class="row">
          <span class="status-dot ${dotClass}"></span>
          <strong class="grow">${escapeHtml(srv.name)}</strong>
          <span class="mcp-badge ${untrusted ? "warn" : ""}">${escapeHtml(untrusted ? "Folder · Untrusted" : scopeLabel)}</span>
          <span class="mono" style="font-size:12px;color:var(--muted)">${escapeHtml(srv.command)} ${escapeHtml((srv.args || []).join(" "))}</span>
          <button class="btn btn-ghost btn-sm" data-rm-mcp="${escapeHtml(srv.name)}" data-rm-scope="${scope}">Remove</button>
        </div>
        ${errorHtml}
        ${toolsHtml}
      </div>
    `;
    })
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

      ${trustBannerHtml}

      <details class="card">
        <summary class="import-gguf-toggle">Developer: add a custom tool (MCP)</summary>
        <div class="row" style="margin-top:8px">
          <input id="mcp-name" class="input" placeholder="name (e.g. filesystem)" />
          <input id="mcp-cmd" class="input mono" placeholder="command (e.g. npx)" />
        </div>
        <input id="mcp-args" class="input mono" placeholder="space-separated args" />
        <div class="row">
          <select id="mcp-scope" class="select-field grow">
            <option value="global">Global (app config mcp.json)</option>
            <option value="workspace" ${appState.workspacePath ? "" : "disabled"}>Workspace (.keel/mcp.json)${appState.workspacePath ? "" : " — pick a folder first"}</option>
          </select>
          <button id="mcp-refresh-btn" class="btn btn-ghost btn-sm" type="button">Refresh</button>
          <button id="mcp-add-btn" class="btn btn-primary btn-sm" type="button">Add</button>
        </div>
      </details>

      ${serversHtml}
    </div>
  `;

  document.getElementById("conn-pick-ws").onclick = () => els.workspacePillBtn.click();
  const trustBtn = document.getElementById("mcp-trust-ws-btn");
  if (trustBtn) {
    trustBtn.onclick = async () => {
      try {
        trustBtn.disabled = true;
        const updated = await invoke("set_workspace_mcp_trust", {
          workspacePath: appState.workspacePath,
          trusted: true,
        });
        appState.bootstrap.mcp_servers = updated;
        renderConnectorsWorkspace();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        renderConnectorsWorkspace();
      }
    };
  }
  const revokeBtn = document.getElementById("mcp-revoke-ws-btn");
  if (revokeBtn) {
    revokeBtn.onclick = async () => {
      try {
        revokeBtn.disabled = true;
        const updated = await invoke("set_workspace_mcp_trust", {
          workspacePath: appState.workspacePath,
          trusted: false,
        });
        appState.bootstrap.mcp_servers = updated;
        renderConnectorsWorkspace();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        renderConnectorsWorkspace();
      }
    };
  }
  document.getElementById("mcp-refresh-btn").onclick = async () => {
    const btn = document.getElementById("mcp-refresh-btn");
    try {
      btn.disabled = true;
      const updated = await invoke("refresh_mcp_servers", {
        workspacePath: appState.workspacePath,
      });
      appState.bootstrap.mcp_servers = updated;
      renderConnectorsWorkspace();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
      renderConnectorsWorkspace();
    }
  };
  document.getElementById("mcp-add-btn").onclick = async () => {
    const btn = document.getElementById("mcp-add-btn");
    try {
      btn.disabled = true;
      const args = document.getElementById("mcp-args").value.trim().split(/\s+/).filter(Boolean);
      const updated = await invoke("save_mcp_server", {
        input: {
          name: document.getElementById("mcp-name").value,
          command: document.getElementById("mcp-cmd").value,
          args,
          scope: document.getElementById("mcp-scope").value,
          workspace_path: appState.workspacePath,
        },
        workspacePath: appState.workspacePath,
      });
      appState.bootstrap.mcp_servers = updated;
      renderConnectorsWorkspace();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
      btn.disabled = false;
    }
  };
  els.viewConnectors.querySelectorAll("[data-rm-mcp]").forEach((btn) => {
    btn.onclick = async () => {
      try {
        btn.disabled = true;
        const updated = await invoke("remove_mcp_server", {
          name: btn.getAttribute("data-rm-mcp"),
          scope: btn.getAttribute("data-rm-scope") || "global",
          workspacePath: appState.workspacePath,
        });
        appState.bootstrap.mcp_servers = updated;
        renderConnectorsWorkspace();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        btn.disabled = false;
      }
    };
  });
}

let modalReturnFocus;
function openModal(title, builder) {
  if (els.modalBackdrop.classList.contains("hidden")) {
    modalReturnFocus = document.activeElement;
  }
  els.modalTitle.textContent = title;
  els.modalBody.innerHTML = "";
  builder(els.modalBody);
  els.modalBackdrop.classList.remove("hidden");
  els.modalBody.querySelector("input, textarea, select, button")?.focus();
}

function closeModal() {
  els.modalBackdrop.classList.add("hidden");
  if (modalReturnFocus?.isConnected) modalReturnFocus.focus();
  modalReturnFocus = null;
}

async function openPublishModal() {
  if (!appState.session || !appState.workspacePath) return;
  let status;
  try {
    status = await invoke("inspect_publish_status", { sessionId: appState.session.id });
  } catch (err) {
    showNotice(String(err));
    return;
  }
  const testRun = appState.session.last_code_test || null;
  const testPassed = testRun ? Boolean(testRun.passed) : null;
  const suggestedBranch =
    status.branch && !["main", "master", "HEAD"].includes(status.branch)
      ? status.branch
      : `keel/${(appState.session.title || "changes").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 32) || "update"}`;
  openModal("Publish changes to GitHub", (body) => {
    body.innerHTML = `
      <p class="muted">Branch: <span class="mono">${escapeHtml(status.branch)}</span> (${escapeHtml(status.head)})${status.remote_url ? ` · Remote: <span class="mono">${escapeHtml(status.remote_url)}</span>` : " · No remote configured"}</p>
      <p class="muted">${status.dirty_files.length ? `${status.dirty_files.length} changed file(s): <span class="mono">${escapeHtml(status.dirty_files.join(", "))}</span>` : "Working tree has no uncommitted changes."}</p>
      ${
        testRun
          ? `<p class="${testRun.passed ? "muted" : "welcome-error"}">Latest test check: <strong>${testRun.passed ? "Passed" : "Failed"}</strong> (exit ${testRun.exit_code}) — <span class="mono">${escapeHtml(testRun.command)}</span></p>`
          : ""
      }
      <label for="pub-branch">Feature branch</label>
      <input id="pub-branch" class="input mono" value="${escapeHtml(suggestedBranch)}" />
      <label for="pub-commit">Commit message</label>
      <input id="pub-commit" class="input" value="${escapeHtml(appState.session.title || "")}" placeholder="Describe your changes" />
      <label for="pub-title">Pull request title</label>
      <input id="pub-title" class="input" value="${escapeHtml(appState.session.title || "")}" />
      <label for="pub-body">Pull request description</label>
      <textarea id="pub-body" class="input" rows="4" placeholder="## Summary&#10;&#10;## Proof"></textarea>
      <label class="row"><input id="pub-push" type="checkbox" ${status.remote_name ? "checked" : "disabled"} /> Push branch to ${escapeHtml(status.remote_name || "remote")}</label>
      <label class="row"><input id="pub-open-pr" type="checkbox" ${status.remote_name && status.gh_authenticated ? "checked" : "disabled"} /> Open pull request with GitHub CLI (${status.gh_authenticated ? "signed in" : status.gh_available ? "run gh auth login" : "gh not installed"})</label>
      <p id="pub-status" class="muted" role="status"></p>
      <div class="row">
        <button id="pub-submit" class="btn btn-primary" ${testPassed === false ? "disabled" : ""}>Commit &amp; publish</button>
      </div>
    `;
    body.querySelector("#pub-submit").onclick = async (event) => {
      const statusEl = body.querySelector("#pub-status");
      event.target.disabled = true;
      statusEl.textContent = "Publishing…";
      try {
        const res = await invoke("publish_code_changes", {
          sessionId: appState.session.id,
          plan: {
            branch_name: body.querySelector("#pub-branch").value,
            commit_message: body.querySelector("#pub-commit").value,
            pr_title: body.querySelector("#pub-title").value,
            pr_body: body.querySelector("#pub-body").value,
            push_remote: body.querySelector("#pub-push").checked,
            open_pr: body.querySelector("#pub-open-pr").checked,
          },
          testPassed,
        });
        statusEl.innerHTML = res.pr_url
          ? `Published <span class="mono">${escapeHtml(res.branch)}</span> (${escapeHtml(res.commit_sha)}) · PR: <span class="mono">${escapeHtml(res.pr_url)}</span>`
          : `Committed <span class="mono">${escapeHtml(res.branch)}</span> (${escapeHtml(res.commit_sha)})${res.pushed ? " and pushed" : ""}.${res.next_command ? ` Next: <span class="mono">${escapeHtml(res.next_command)}</span>` : ""}`;
        showToast("Changes published.");
      } catch (err) {
        statusEl.textContent = String(err);
        event.target.disabled = false;
      }
    };
  });
}

function openDiffsModal() {
  const edits = appState.session?.staged_edits ?? [];
  const pendingCount = edits.filter((e) => e.status === "pending").length;
  openModal("Workspace File Changes", (body) => {
    const top = document.createElement("div");
    top.className = "row";
    top.innerHTML = `
      <span class="grow" style="color:var(--muted)">Review proposed edits before writing them to disk.</span>
      <button id="reject-all-btn" class="btn btn-ghost btn-sm" ${pendingCount === 0 ? "disabled" : ""}>Reject all</button>
      <button id="apply-all-btn" class="btn btn-primary btn-sm" ${pendingCount === 0 ? "disabled" : ""}>Approve all${pendingCount ? ` (${pendingCount})` : ""}</button>
    `;
    body.appendChild(top);
    top.querySelector("#reject-all-btn").onclick = async (ev) => {
      ev.target.disabled = true;
      try {
        const pendingIds = (appState.session?.staged_edits || []).filter((e) => e.status === "pending").map((e) => e.id);
        for (const editId of pendingIds) {
          appState.session = await invoke("discard_staged_edit_cmd", { sessionId: appState.session.id, editId });
        }
        renderAll();
        openDiffsModal();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        ev.target.disabled = false;
      }
    };
    top.querySelector("#apply-all-btn").onclick = async (ev) => {
      ev.target.disabled = true;
      try {
        appState.session = await invoke("apply_all_staged_edits", { sessionId: appState.session.id });
        await refreshWorkspaceFiles();
        renderAll();
        openDiffsModal();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        ev.target.disabled = false;
      }
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
          discBtn.disabled = true;
          try {
            appState.session = await invoke("discard_staged_edit_cmd", {
              sessionId: appState.session.id,
              editId: edit.id,
            });
            renderAll();
            openDiffsModal();
          } catch (err) {
            showToast(friendlyRuntimeError(err));
            discBtn.disabled = false;
          }
        };
        const applyBtn = document.createElement("button");
        applyBtn.className = "btn btn-primary btn-sm";
        applyBtn.textContent = "Approve";
        applyBtn.onclick = async () => {
          applyBtn.disabled = true;
          try {
            appState.session = await invoke("apply_staged_edit_cmd", {
              sessionId: appState.session.id,
              editId: edit.id,
            });
            await refreshWorkspaceFiles();
            renderAll();
            openDiffsModal();
          } catch (err) {
            showToast(friendlyRuntimeError(err));
            applyBtn.disabled = false;
          }
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
    const speedCopy = speedSettingsCopy();
    let runWhere = b.config.run_where || "auto";
    let graphicsShare = b.config.graphics_share || "auto";
    const speedCard = document.createElement("div");
    speedCard.className = "card";
    const segButtons = (options, current, attr) =>
      options
        .map(
          ([id, label]) =>
            `<button type="button" class="seg-btn ${current === id ? "active" : ""}" data-${attr}="${id}">${label}</button>`,
        )
        .join("");
    speedCard.innerHTML = `
      <strong>${speedCopy.where}</strong>
      <div class="seg-group">${segButtons(speedCopy.whereOptions, runWhere, "run-where")}</div>
      <strong>${speedCopy.share}</strong>
      <div class="seg-group">${segButtons(speedCopy.shareOptions, graphicsShare, "graphics-share")}</div>
      <p class="muted" id="speed-summary">${escapeHtml(b.computer?.summary || "Automatic chooses where the model runs on this computer.")}</p>
      <p class="muted">${speedCopy.later}</p>
    `;
    const paintSpeed = () => {
      speedCard.querySelectorAll("[data-run-where]").forEach((btn) => {
        btn.classList.toggle("active", btn.getAttribute("data-run-where") === runWhere);
      });
      const onProcessor = runWhere === "processor";
      speedCard.querySelectorAll("[data-graphics-share]").forEach((btn) => {
        btn.disabled = onProcessor;
        btn.classList.toggle("active", !onProcessor && btn.getAttribute("data-graphics-share") === graphicsShare);
      });
    };
    speedCard.querySelectorAll("[data-run-where]").forEach((btn) => {
      btn.onclick = () => {
        runWhere = btn.getAttribute("data-run-where");
        paintSpeed();
      };
    });
    speedCard.querySelectorAll("[data-graphics-share]").forEach((btn) => {
      btn.onclick = () => {
        if (runWhere === "processor") return;
        graphicsShare = btn.getAttribute("data-graphics-share");
        paintSpeed();
      };
    });
    paintSpeed();
    body.appendChild(speedCard);

    const checkCard = document.createElement("div");
    checkCard.className = "card";
    const activeId = b.runtime?.active_model_id || b.config?.selected_model_id;
    const known = b.model_checks?.[activeId];
    checkCard.innerHTML = `
      <strong>Check this model</strong>
      <div id="model-check-result">${known ? checkResultHtml(known) : `<p class="muted">${escapeHtml(modelCheckIntro())}</p>`}</div>
      <button id="check-model" type="button" class="btn btn-sm">Check this model</button>
    `;
    checkCard.querySelector("#check-model").onclick = async () => {
      const button = checkCard.querySelector("#check-model");
      const slot = checkCard.querySelector("#model-check-result");
      button.disabled = true;
      slot.innerHTML = `<p class="muted">Checking… this usually takes under a minute.</p>`;
      try {
        const report = await invoke("check_current_model");
        await refreshBootstrap();
        slot.innerHTML = checkResultHtml(report);
      } catch (err) {
        slot.innerHTML = `<p class="muted">${escapeHtml(friendlyRuntimeError(err))}</p>`;
      } finally {
        button.disabled = false;
      }
    };
    body.appendChild(checkCard);

    const packsCard = document.createElement("div");
    packsCard.className = "card";
    packsCard.innerHTML = `
      <strong>Capability packs</strong>
      <p class="muted">Optional local packs (speech and demos). Downloads are checksummed and stored on this computer.</p>
      <div id="capability-packs-list"><p class="muted">Loading…</p></div>
    `;
    body.appendChild(packsCard);
    const renderPacksList = async () => {
      const slot = packsCard.querySelector("#capability-packs-list");
      try {
        const packs = await invoke("list_capability_packs");
        if (!packs?.length) {
          slot.innerHTML = `<p class="muted">No capability packs listed yet.</p>`;
          return;
        }
        slot.innerHTML = packs
          .map((pack) => {
            const id = escapeHtml(pack.capability_id || "");
            const name = escapeHtml(pack.display_name || pack.capability_id || "Pack");
            const detail = escapeHtml(pack.detail || "");
            const vram =
              pack.min_vram_mib > 0
                ? `${Number(pack.min_vram_mib).toLocaleString()} MiB graphics`
                : "No graphics required";
            const status = pack.installed ? "Installed" : "Not installed";
            const canDownload = Boolean(pack.download_url) && !pack.installed;
            const actions = pack.installed
              ? `<button type="button" class="btn btn-ghost btn-sm" data-remove-pack="${id}">Remove</button>`
              : canDownload
                ? `<button type="button" class="btn btn-primary btn-sm" data-download-pack="${id}">Download</button>`
                : `<button type="button" class="btn btn-ghost btn-sm" disabled title="Download URL not pinned yet">Coming soon</button>`;
            return `<div class="row" style="align-items:flex-start;gap:0.75rem;margin:0.65rem 0;">
              <div class="grow">
                <div><strong>${name}</strong></div>
                <div class="muted">${detail}</div>
                <div class="muted">${escapeHtml(status)} · ${escapeHtml(vram)}</div>
              </div>
              <div>${actions}</div>
            </div>`;
          })
          .join("");
        slot.querySelectorAll("[data-download-pack]").forEach((btn) => {
          btn.onclick = async () => {
            const packId = btn.getAttribute("data-download-pack");
            btn.disabled = true;
            btn.textContent = "Downloading…";
            try {
              await invoke("download_capability_pack", { capabilityId: packId });
              showToast("Pack downloaded.");
              await renderPacksList();
            } catch (err) {
              showToast(friendlyRuntimeError(err));
              btn.disabled = false;
              btn.textContent = "Download";
            }
          };
        });
        slot.querySelectorAll("[data-remove-pack]").forEach((btn) => {
          btn.onclick = () => {
            const packId = btn.getAttribute("data-remove-pack");
            confirmAction(
              "Remove pack?",
              "This deletes the pack files from this computer. You can download again later.",
              "Remove pack",
              async () => {
                try {
                  await invoke("remove_capability_pack", { capabilityId: packId });
                  showToast("Pack removed.");
                  await renderPacksList();
                } catch (err) {
                  showToast(friendlyRuntimeError(err));
                }
              },
            );
          };
        });
      } catch (err) {
        slot.innerHTML = `<p class="muted">${escapeHtml(friendlyRuntimeError(err))}</p>`;
      }
    };
    renderPacksList();

    const audioCard = document.createElement("div");
    audioCard.className = "card";
    audioCard.innerHTML = `
      <strong>Speech packs</strong>
      <p class="muted">Download Whisper or Piper model files, then register the matching local runner. You can also register a runner for a model you already have.</p>
      <div id="audio-packs-list"><p class="muted">Loading…</p></div>
      <strong style="display:block;margin-top:14px">Try STT / TTS</strong>
      <p class="muted">${escapeHtml(AUDIO_TRY_STT_INPUT_HELP)} ${escapeHtml(AUDIO_TRY_TTS_INPUT_HELP)} Register a local runner before trying speech to text or text to speech.</p>
      <div class="row" style="gap:6px;margin-top:8px;flex-wrap:wrap">
        <button type="button" class="btn btn-sm btn-ghost" id="audio-try-stt">Try STT</button>
        <button type="button" class="btn btn-sm btn-ghost" id="audio-try-tts">Try TTS</button>
      </div>
      <p id="audio-try-status" class="muted" role="status" style="margin-top:8px"></p>
      <strong style="display:block;margin-top:14px">Recent audio artifacts</strong>
      <p class="muted">Local transcripts and WAV files from successful runs.</p>
      <div id="audio-artifacts-list"><p class="muted">Loading…</p></div>
      <div class="row" style="gap:6px;margin-top:8px;flex-wrap:wrap">
        <button type="button" class="btn btn-sm btn-ghost" id="audio-artifacts-refresh">Refresh</button>
        <button type="button" class="btn btn-sm btn-ghost" id="audio-artifacts-clear">Clear</button>
      </div>
    `;
    body.appendChild(audioCard);

    const audioTryStatus = audioCard.querySelector("#audio-try-status");
    const setAudioTryStatus = (message, { ok = false } = {}) => {
      audioTryStatus.textContent = message || "";
      audioTryStatus.style.color = ok ? "var(--ok, inherit)" : "";
    };
    const wireAudioTryControls = () => {
      audioCard.querySelector("#audio-try-stt").addEventListener("click", async () => {
        const btn = audioCard.querySelector("#audio-try-stt");
        btn.disabled = true;
        setAudioTryStatus("Choose a local audio clip…");
        try {
          const picked = await invoke("pick_audio_file");
          if (!picked) {
            setAudioTryStatus("Try STT cancelled — no file selected.");
            return;
          }
          setAudioTryStatus(`Running STT on ${picked}…`);
          const transcript = await invoke("run_audio_stt", { audioPath: picked });
          const ok = formatAudioTrySttSuccess(transcript);
          setAudioTryStatus(ok, { ok: true });
          showToast(ok);
          await renderAudioArtifacts();
        } catch (err) {
          // Preserve core pack/runner messages (do not rewrite into model-runtime copy).
          const msg = String(err ?? "").replace(/^Error:\s*/i, "").trim() || "STT failed.";
          setAudioTryStatus(msg);
          showToast(msg);
        } finally {
          btn.disabled = false;
        }
      });
      audioCard.querySelector("#audio-try-tts").addEventListener("click", async () => {
        const btn = audioCard.querySelector("#audio-try-tts");
        btn.disabled = true;
        setAudioTryStatus(`Running TTS (“${AUDIO_TRY_TTS_SAMPLE}”)…`);
        try {
          const outputPath = await invoke("run_audio_tts", {
            text: AUDIO_TRY_TTS_SAMPLE,
            outputPath: null,
          });
          const ok = formatAudioTryTtsSuccess(outputPath);
          setAudioTryStatus(ok, { ok: true });
          showToast(ok);
          await renderAudioArtifacts();
        } catch (err) {
          const msg = String(err ?? "").replace(/^Error:\s*/i, "").trim() || "TTS failed.";
          setAudioTryStatus(msg);
          showToast(msg);
        } finally {
          btn.disabled = false;
        }
      });
    };

    const renderAudioArtifacts = async () => {
      const slot = audioCard.querySelector("#audio-artifacts-list");
      try {
        const artifacts = await invoke("list_audio_artifacts");
        if (!artifacts?.length) {
          slot.innerHTML = `<p class="muted">No audio artifacts yet. Run STT or TTS to create one.</p>`;
          return;
        }
        const recent = artifacts.slice(0, 12);
        slot.innerHTML = recent
          .map((art) => {
            const kind =
              art.kind === "stt_transcript"
                ? "STT transcript"
                : art.kind === "tts_output"
                  ? "TTS output"
                  : String(art.kind || "Artifact");
            const preview = art.text_preview
              ? `<div class="muted" style="font-size:12px">${escapeHtml(art.text_preview)}</div>`
              : "";
            return `
              <div class="row" style="align-items:flex-start;gap:10px;margin-bottom:8px">
                <div class="grow">
                  <div class="muted" style="font-size:12px">${escapeHtml(kind)}</div>
                  <div class="mono" style="font-size:12px;word-break:break-all">${escapeHtml(art.path || "")}</div>
                  ${preview}
                </div>
              </div>
            `;
          })
          .join("");
      } catch (err) {
        slot.innerHTML = `<p class="muted">${escapeHtml(friendlyRuntimeError(err))}</p>`;
      }
    };
    audioCard.querySelector("#audio-artifacts-refresh").addEventListener("click", () => {
      renderAudioArtifacts();
    });
    audioCard.querySelector("#audio-artifacts-clear").addEventListener("click", async () => {
      if (!window.confirm("Clear all local audio artifacts from disk?")) return;
      try {
        await invoke("clear_audio_artifacts");
        showToast("Audio artifacts cleared.");
        await renderAudioArtifacts();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
      }
    });
    renderAudioArtifacts();
    wireAudioTryControls();
    const renderAudioPacks = async () => {
      const slot = audioCard.querySelector("#audio-packs-list");
      try {
        const packs = await invoke("list_audio_packs");
        if (!packs?.length) {
          slot.innerHTML = `<p class="muted">No audio packs listed yet.</p>`;
          return;
        }
        slot.innerHTML = packs
          .filter((pack) => pack.id !== "stt-demo-fixture")
          .map((pack) => {
            const kind =
              pack.capability === "stt"
                ? "Speech to text"
                : pack.capability === "tts"
                  ? "Text to speech"
                  : String(pack.capability || "Audio");
            const status = pack.runner_path
              ? `Runner configured · ${pack.runner_path}`
              : "Runner required";
            const downloadable = Boolean(
              pack.sha256 && pack.download_url && !String(pack.download_url).startsWith("keel-pack:unpinned/"),
            );
            const id = escapeHtml(pack.id);
            return `
              <div class="row" style="align-items:flex-start;gap:10px;margin-bottom:10px" data-audio-pack="${id}">
                <div class="grow">
                  <div>${escapeHtml(pack.display_name || pack.id)}</div>
                  <div class="muted" style="font-size:12px">${escapeHtml(kind)} · ${escapeHtml(status)}</div>
                </div>
                <div class="row" style="gap:6px;flex-wrap:wrap;justify-content:flex-end">
                  ${
                    pack.installed
                      ? `<button type="button" class="btn btn-sm btn-ghost" data-audio-remove="${id}">Remove</button>`
                      : downloadable
                        ? `<button type="button" class="btn btn-sm btn-ghost" data-audio-download="${id}">Download model</button>`
                        : ""
                  }
                  <button type="button" class="btn btn-sm btn-ghost" data-audio-runner="${id}">Register runner…</button>
                  ${
                    pack.capability === "stt"
                      ? `<button type="button" class="btn btn-sm btn-ghost" data-audio-whisper="${id}">Whisper.cpp…</button>`
                      : pack.capability === "tts"
                        ? `<button type="button" class="btn btn-sm btn-ghost" data-audio-piper="${id}">Piper…</button>`
                        : ""
                  }
                </div>
              </div>
            `;
          })
          .join("");

        slot.querySelectorAll("[data-audio-download]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            btn.disabled = true;
            try {
              await invoke("download_audio_pack", { packId: btn.getAttribute("data-audio-download") });
              showToast("Audio pack downloaded.");
              await renderAudioPacks();
            } catch (err) {
              showToast(friendlyRuntimeError(err));
              btn.disabled = false;
            }
          });
        });
        slot.querySelectorAll("[data-audio-remove]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            if (!window.confirm("Remove this installed audio pack from disk?")) return;
            btn.disabled = true;
            try {
              await invoke("remove_audio_pack", { packId: btn.getAttribute("data-audio-remove") });
              showToast("Audio pack removed.");
              await renderAudioPacks();
            } catch (err) {
              showToast(friendlyRuntimeError(err));
              btn.disabled = false;
            }
          });
        });
        slot.querySelectorAll("[data-audio-runner]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            const packId = btn.getAttribute("data-audio-runner");
            const command = window.prompt(
              "Local binary or script for this pack (absolute path or PATH name).\nSTT: command <audio_path> → transcript on stdout.\nTTS: command --text <text> --output <path>.\nFor Whisper.cpp / Piper use those dedicated buttons instead.",
              "",
            );
            if (command == null) return;
            const trimmed = command.trim();
            if (!trimmed) {
              showToast("Provide a runner path.");
              return;
            }
            btn.disabled = true;
            try {
              await invoke("register_audio_pack_runner", { packId, command: trimmed });
              showToast("Runner registered.");
              await renderAudioPacks();
            } catch (err) {
              showToast(friendlyRuntimeError(err));
              btn.disabled = false;
            }
          });
        });
        slot.querySelectorAll("[data-audio-whisper]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            const packId = btn.getAttribute("data-audio-whisper");
            const command = window.prompt(
              "Whisper.cpp binary (absolute path or PATH name, e.g. whisper-cli).\nInvokes: command …args… -m <model> -f <audio_path> → transcript on stdout.",
              "whisper-cli",
            );
            if (command == null) return;
            const trimmedCmd = command.trim();
            if (!trimmedCmd) {
              showToast("Provide a Whisper.cpp binary.");
              return;
            }
            const model = window.prompt(
              "Path to a local Whisper model file. Leave blank to use the model downloaded for this pack.",
              "",
            );
            if (model == null) return;
            const trimmedModel = model.trim();
            btn.disabled = true;
            try {
              await invoke("register_whisper_cpp_runner", {
                packId,
                command: trimmedCmd,
                model: trimmedModel,
              });
              showToast("Whisper.cpp runner registered.");
              await renderAudioPacks();
            } catch (err) {
              showToast(friendlyRuntimeError(err));
              btn.disabled = false;
            }
          });
        });
        slot.querySelectorAll("[data-audio-piper]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            const packId = btn.getAttribute("data-audio-piper");
            const command = window.prompt(
              "Piper binary (absolute path or PATH name, e.g. piper).\nInvokes: command …args… --model <onnx> -f <path> with text on stdin.",
              "piper",
            );
            if (command == null) return;
            const trimmedCmd = command.trim();
            if (!trimmedCmd) {
              showToast("Provide a Piper binary.");
              return;
            }
            const model = window.prompt(
              "Path to a local Piper ONNX voice with its .onnx.json file. Leave blank to use the voice downloaded for this pack.",
              "",
            );
            if (model == null) return;
            const trimmedModel = model.trim();
            btn.disabled = true;
            try {
              await invoke("register_piper_runner", {
                packId,
                command: trimmedCmd,
                model: trimmedModel,
              });
              showToast("Piper runner registered.");
              await renderAudioPacks();
            } catch (err) {
              showToast(friendlyRuntimeError(err));
              btn.disabled = false;
            }
          });
        });
      } catch (err) {
        slot.innerHTML = `<p class="muted">${escapeHtml(friendlyRuntimeError(err))}</p>`;
      }
    };
    renderAudioPacks();

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
      <div class="seg-group">${toneButtons}</div>
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

    const syncAppearanceButtons = () => {
      const cur = appState.appearance;
      appearanceCard.querySelectorAll("[data-accent-pick]").forEach((b) => b.classList.toggle("active", b.getAttribute("data-accent-pick") === cur.accent));
      appearanceCard.querySelectorAll("[data-tone-pick]").forEach((b) => b.classList.toggle("active", b.getAttribute("data-tone-pick") === cur.tone));
      appearanceCard.querySelectorAll("[data-density-pick]").forEach((b) => b.classList.toggle("active", b.getAttribute("data-density-pick") === cur.density));
      appearanceCard.querySelectorAll("[data-sidebar-pick]").forEach((b) => b.classList.toggle("active", (b.getAttribute("data-sidebar-pick") === "true") === cur.sidebar));
      appearanceCard.querySelectorAll("[data-size-pick]").forEach((b) => b.classList.toggle("active", b.getAttribute("data-size-pick") === cur.textSize));
    };

    appearanceCard.querySelectorAll("[data-accent-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, accent: btn.getAttribute("data-accent-pick") });
        syncAppearanceButtons();
      };
    });
    appearanceCard.querySelectorAll("[data-tone-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, tone: btn.getAttribute("data-tone-pick") });
        syncAppearanceButtons();
      };
    });
    appearanceCard.querySelectorAll("[data-density-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, density: btn.getAttribute("data-density-pick") });
        syncAppearanceButtons();
      };
    });
    appearanceCard.querySelectorAll("[data-sidebar-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({
          ...appState.appearance,
          sidebar: btn.getAttribute("data-sidebar-pick") === "true",
        });
        syncAppearanceButtons();
      };
    });
    appearanceCard.querySelectorAll("[data-size-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, textSize: btn.getAttribute("data-size-pick") });
        syncAppearanceButtons();
      };
    });
    body.appendChild(appearanceCard);

    const harnessOptions = b.harnesses
      .map((h) => {
        const recommended = h.id === "keel" ? " — default" : "";
        const missing = h.available ? "" : " (not installed)";
        return `<option value="${escapeHtml(h.id)}" ${h.id === b.config.harness_mode ? "selected" : ""}>${escapeHtml(h.name)}${recommended}${missing}</option>`;
      })
      .join("");
    const harnessHint = b.config.harness_mode === "keel-cli"
      ? `<p class="muted">Code uses Keel Code (<code>keel</code>). Finish the current turn before resuming the same journal in a terminal.</p>`
      : `<p class="muted">Select Keel Code (CLI) to share Code journals with a current <code>keel</code> binary installed on PATH.</p>`;
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <label><strong>Things Keel should always know</strong></label>
      <textarea id="cust-inst" class="textarea-field" rows="3">${escapeHtml(b.config.custom_instructions)}</textarea>
      <label class="row"><input id="cust-auto" type="checkbox" ${b.config.auto_apply_edits ? "checked" : ""} /> Apply file changes without asking me first</label>
      <details class="advanced-drawer">
        <summary class="import-gguf-toggle">Developer options</summary>
        <div class="advanced-drawer-body">
          <label><strong>Context window</strong></label>
          <select id="cust-ctx" class="select-field">
            <option value="0" ${!b.config.context_window ? "selected" : ""}>Auto (${b.system_ram_gb >= 16 ? "16K" : "8K"} for this computer)</option>
            <option value="4096" ${b.config.context_window === 4096 ? "selected" : ""}>4,096 tokens (smallest RAM, coolest)</option>
            <option value="8192" ${b.config.context_window === 8192 ? "selected" : ""}>8,192 tokens (lighter RAM)</option>
            <option value="16384" ${b.config.context_window === 16384 ? "selected" : ""}>16,384 tokens (longer docs)</option>
            <option value="32768" ${b.config.context_window === 32768 ? "selected" : ""}>32,768 tokens (high RAM)</option>
          </select>
          <label><strong>Engine for Code mode</strong></label>
          <select id="cust-harness" class="select-field">${harnessOptions}</select>
          ${harnessHint}
          <label><strong>Custom local engine path</strong></label>
          <input id="cust-llama" class="input mono" value="${escapeHtml(b.config.llama_server_path || "")}" placeholder="Leave blank to use Keel's built-in engine" />
        </div>
      </details>
      <details class="advanced-drawer">
        <summary class="import-gguf-toggle">Keyboard shortcuts</summary>
        <div class="shortcuts-grid">
          <kbd>Ctrl/Cmd + N</kbd><span>New conversation</span>
          <kbd>Ctrl/Cmd + K</kbd><span>Search conversations</span>
          <kbd>Ctrl/Cmd + B</kbd><span>Toggle sidebar</span>
          <kbd>Ctrl/Cmd + 1 / 2 / 3</kbd><span>Switch Chat / Work / Code</span>
          <kbd>Ctrl/Cmd + ,</kbd><span>Open Settings</span>
          <kbd>Esc</kbd><span>Stop streaming or close dialog</span>
        </div>
      </details>
      <div class="row">
        <button id="cust-reset-welcome" type="button" class="btn btn-ghost btn-sm">Show the welcome screen again</button>
        <div class="spacer"></div>
        <button id="cust-save" class="btn btn-primary btn-sm">Save settings</button>
      </div>
    `;
    const updateCard = document.createElement("div");
    updateCard.className = "card";
    const renderSettingsUpdate = () => {
      const info = appState.appUpdate;
      const statusHtml = !info
        ? `<p class="muted">You are running Keel Desktop v${escapeHtml(b.version)}.</p>`
        : info.available
          ? `<p class="muted">Keel Desktop <strong>v${escapeHtml(String(info.version || "").replace(/^v/i, ""))}</strong> is available (current: v${escapeHtml(info.current_version)}). ${
              info.can_install_in_place
                ? "Updates download, verify, and restart right inside Keel."
                : "Installed via system package — open the release page to download the updated package."
            }</p>`
          : `<p class="muted">Keel Desktop v${escapeHtml(info.current_version)} is up to date.</p>`;
      const installBtn = info?.available
        ? `<button id="install-app-update-btn" type="button" class="btn btn-primary btn-sm" ${appState.appUpdateBusy ? "disabled" : ""}>${
            appState.appUpdateBusy
              ? escapeHtml(updateProgressLabel())
              : info.can_install_in_place
                ? "Update & restart"
                : "Download installer"
          }</button>`
        : "";
      updateCard.innerHTML = `
        <strong>App updates</strong>
        ${statusHtml}
        <div class="row" style="gap:8px;flex-wrap:wrap">
          <button id="check-app-update-btn" type="button" class="btn btn-sm" ${appState.appUpdateBusy ? "disabled" : ""}>Check for updates</button>
          ${installBtn}
          ${info?.available && info.release_url ? `<a class="btn btn-ghost btn-sm" href="#" data-external-link="${escapeHtml(info.release_url)}">Release notes</a>` : ""}
        </div>
      `;
      updateCard.querySelector("#check-app-update-btn")?.addEventListener("click", async (ev) => {
        ev.target.disabled = true;
        ev.target.textContent = "Checking…";
        await checkAppUpdate({ silent: false });
        renderSettingsUpdate();
      });
      updateCard.querySelector("#install-app-update-btn")?.addEventListener("click", async () => {
        await installOrDownloadAppUpdate();
        renderSettingsUpdate();
      });
    };
    renderSettingsUpdate();
    body.appendChild(updateCard);
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
      const prevCtx = b.config.context_window || 0;
      const nextCtx = Number(card.querySelector("#cust-ctx")?.value || 0);
      const prevWhere = b.config.run_where || "auto";
      const prevShare = b.config.graphics_share || "auto";
      try {
        await invoke("save_customize_settings", {
          update: {
            custom_instructions: card.querySelector("#cust-inst").value,
            auto_apply_edits: card.querySelector("#cust-auto").checked,
            harness_mode: card.querySelector("#cust-harness").value,
            llama_server_path: card.querySelector("#cust-llama").value || null,
            context_window: nextCtx,
            run_where: runWhere,
            graphics_share: graphicsShare,
          },
        });
        await refreshBootstrap();
        renderHeader();
        closeModal();
        const running = appState.bootstrap?.runtime?.state === "running";
        const changed = prevCtx !== nextCtx || prevWhere !== runWhere || prevShare !== graphicsShare;
        showToast(
          running && changed
            ? "Settings saved. The next message uses this."
            : "Settings saved.",
        );
      } catch (err) {
        showToast(friendlyRuntimeError(err));
      }
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

function clearNotice() {
  if (els.noticeSlot) els.noticeSlot.innerHTML = "";
}

function showNotice(message) {
  const slot = els.noticeSlot || els.approvalSlot;
  slot.innerHTML = `<div class="approval-card notice-card" role="status"><span class="notice-text">${escapeHtml(message)}</span><button type="button" class="icon-btn notice-dismiss" aria-label="Dismiss notice">&times;</button></div>`;
  slot.querySelector(".notice-dismiss")?.addEventListener("click", clearNotice);
}

function autoSizePromptInput() {
  els.promptInput.style.height = "auto";
  els.promptInput.style.height = `${Math.min(els.promptInput.scrollHeight, 140)}px`;
}

async function importBrowserFiles(fileList) {
  if (appState.streaming || !fileList?.length) return;
  const files = [...fileList];
  if (appState.pendingDocuments.length + files.length > 8) {
    showToast("Attach up to 8 documents per message.");
    return;
  }
  const readBase64 = (file) =>
    new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const res = String(reader.result || "");
        const comma = res.indexOf(",");
        resolve(comma >= 0 ? res.slice(comma + 1) : res);
      };
      reader.onerror = () => reject(new Error(`Could not read ${file.name}`));
      reader.readAsDataURL(file);
    });
  const MAX_DROP_BYTES = 20 * 1024 * 1024;
  for (const file of files) {
    if (file.size > MAX_DROP_BYTES) {
      showToast(`${file.name || "File"} is larger than the 20 MB attachment limit.`);
      continue;
    }
    try {
      const base64Data = await readBase64(file);
      const name = file.name || `pasted-${Date.now()}.${(file.type || "image/png").split("/")[1] || "png"}`;
      const doc = await invoke("import_document_from_bytes", { name, base64Data });
      appState.pendingDocuments.push(doc);
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  }
  saveDraft();
  renderAttachmentChips();
}

function wireUi() {
  try {
    const savedThink = localStorage.getItem(THINKING_STORAGE_KEY);
    if (savedThink !== null) els.thinkingToggle.checked = savedThink === "1";
  } catch {
    // Ignore storage errors.
  }
  els.thinkingToggle.addEventListener("change", () => {
    try { localStorage.setItem(THINKING_STORAGE_KEY, els.thinkingToggle.checked ? "1" : "0"); }
    catch { /* Ignore storage errors. */ }
  });

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
    try {
      const documents = await invoke("pick_documents");
      if (appState.pendingDocuments.length + documents.length > 8) { showToast("Attach up to 8 documents per message."); return; }
      appState.pendingDocuments.push(...documents); saveDraft(); renderAttachmentChips();
    } catch (err) {
      showToast(friendlyRuntimeError(err));
    }
  };
  const chatCol = document.querySelector(".chat-column");
  if (chatCol) {
    let dragDepth = 0;
    chatCol.addEventListener("dragenter", (e) => {
      if (!e.dataTransfer?.types?.includes("Files")) return;
      e.preventDefault();
      dragDepth += 1;
      chatCol.classList.add("drop-active");
    });
    chatCol.addEventListener("dragover", (e) => {
      if (!e.dataTransfer?.types?.includes("Files")) return;
      e.preventDefault();
    });
    chatCol.addEventListener("dragleave", () => {
      dragDepth = Math.max(0, dragDepth - 1);
      if (dragDepth === 0) chatCol.classList.remove("drop-active");
    });
    chatCol.addEventListener("drop", async (e) => {
      e.preventDefault();
      dragDepth = 0;
      chatCol.classList.remove("drop-active");
      if (e.dataTransfer?.files?.length) await importBrowserFiles(e.dataTransfer.files);
    });
  }
  els.promptInput.addEventListener("paste", async (e) => {
    const files = [...(e.clipboardData?.files || [])];
    if (!files.length) return;
    e.preventDefault();
    await importBrowserFiles(files);
  });
  document.addEventListener("click", (event) => {
    if (appState.modelSwitcherOpen && !event.target.closest(".model-pill-wrap")) {
      closeModelSwitcher();
    }
    const link = event.target.closest("[data-external-link]");
    if (link) { event.preventDefault(); invoke("open_external_link", { url: link.dataset.externalLink }).catch((error) => showToast(String(error))); }
  });
  document.addEventListener("keydown", (event) => {
    const mod = (event.ctrlKey || event.metaKey) && !event.altKey;
    const key = event.key.toLowerCase();
    const modalOpen = !els.modalBackdrop.classList.contains("hidden");
    const overlayOpen = modalOpen || welcome.isOpen();
    if (mod && key === "k" && !overlayOpen) {
      event.preventDefault();
      if (!appState.appearance.sidebar) {
        applyAppearance({ ...appState.appearance, sidebar: true });
      }
      document.getElementById("thread-search")?.focus();
      return;
    }
    if (mod && key === "b" && !overlayOpen) {
      event.preventDefault();
      applyAppearance({ ...appState.appearance, sidebar: !appState.appearance.sidebar });
      return;
    }
    if (mod && key === "n" && !overlayOpen) {
      event.preventDefault();
      createNewSession(MODES[appState.activeView]?.preset || "chat");
      return;
    }
    if (mod && event.key === "," && !overlayOpen) {
      event.preventDefault();
      openCustomizeModal();
      return;
    }
    if (mod && !event.shiftKey && ["1", "2", "3"].includes(event.key) && !overlayOpen) {
      event.preventDefault();
      setActiveView(event.key === "1" ? "chat" : event.key === "2" ? "work" : "code");
      return;
    }
    if (event.key === "Escape") {
      if (appState.modelSwitcherOpen) { closeModelSwitcher(); return; }
      if (modalOpen) { closeModal(); return; }
      if (welcome.isOpen()) return;
      if (appState.streaming) {
        els.stopBtn.click();
      }
    }
    if (event.key === "Tab" && modalOpen) {
      const focusable = [...els.modalBackdrop.querySelectorAll("button, input, textarea, select, a[href]")].filter((el) => !el.disabled && el.getClientRects().length);
      const first = focusable[0], last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  });

  document.querySelectorAll(".mode-nav-item").forEach((btn) => {
    btn.onclick = () => { closeModelSwitcher(); setActiveView(btn.getAttribute("data-view")); };
  });

  document.querySelectorAll("[data-viewer-tab]").forEach((tab) => {
    tab.onclick = () => {
      appState.viewerTab = tab.getAttribute("data-viewer-tab");
      renderViewerPane();
    };
  });

  els.modelPillBtn.onclick = (event) => {
    event.stopPropagation();
    const b = appState.bootstrap;
    if (!b) return;
    if (welcome.status().paused) {
      closeModelSwitcher();
      welcome.show();
      return;
    }
    const hasAnyLocal = b.models.some((m) => m.installed || m.damaged);
    if (!isRuntimeLive(b.runtime) && !hasAnyLocal && !appState.waking) {
      closeModelSwitcher();
      welcome.show();
      return;
    }
    appState.modelSwitcherOpen = !appState.modelSwitcherOpen;
    els.modelPillBtn.setAttribute("aria-expanded", String(appState.modelSwitcherOpen));
    renderModelSwitcher();
  };
  document.getElementById("open-customize-btn").onclick = openCustomizeModal;
  els.harnessPillBtn.onclick = openCustomizeModal;
  els.diffsPillBtn.onclick = openDiffsModal;
  els.publishPrBtn.onclick = openPublishModal;
  document.getElementById("modal-close-btn").onclick = closeModal;
  els.modalBackdrop.addEventListener("click", (event) => {
    if (event.target === els.modalBackdrop) closeModal();
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && welcome.isOpen()) welcome.handleEscape();
  });

  els.chatScroll.addEventListener("click", async (e) => {
    const copyCode = e.target.closest("[data-copy-code]");
    if (copyCode) {
      try {
        await navigator.clipboard.writeText(copyCode.closest(".code-block").querySelector("code").textContent);
        copyCode.textContent = "Copied";
        setTimeout(() => { if (copyCode.isConnected) copyCode.textContent = "Copy code"; }, 1800);
      } catch {
        showToast("Could not access clipboard.");
      }
      return;
    }
    const copyMessage = e.target.closest("[data-copy-message]");
    if (copyMessage) {
      const msg = appState.session.messages.find((m) => m.id === copyMessage.dataset.copyMessage);
      try {
        await navigator.clipboard.writeText(msg?.content || "");
        copyMessage.textContent = "Copied";
        setTimeout(() => { if (copyMessage.isConnected) copyMessage.textContent = "Copy"; }, 1800);
      } catch {
        showToast("Could not access clipboard.");
      }
      return;
    }
    const continueBtn = e.target.closest("[data-continue-message]");
    if (continueBtn && !appState.streaming) {
      els.promptInput.value = "Continue where you left off.";
      saveDraft();
      await sendCurrentPrompt();
      return;
    }
    const retryUnsent = e.target.closest("[data-retry-unsent]");
    if (retryUnsent && !appState.streaming) {
      if (!els.promptInput.value.trim()) {
        const unsentUser = [...(appState.session?.messages || [])]
          .reverse()
          .find((m) => m.role === "user" && m.unsent);
        if (unsentUser) {
          els.promptInput.value = unsentUser.content || "";
          if (!appState.pendingDocuments.length && unsentUser.attachments?.length) {
            appState.pendingDocuments = unsentUser.attachments;
            renderAttachmentChips();
          }
          autoSizePromptInput();
        }
      }
      await sendCurrentPrompt();
      return;
    }
    const source = e.target.closest("[data-message-source]");
    if (source) { const doc = appState.session.messages.flatMap((m) => m.attachments || []).find((d) => d.id === source.dataset.messageSource); if (doc) everyday.previewSource(doc); return; }
    const editMessage = e.target.closest("[data-edit-message], [data-retry-message]");
    if (editMessage && !appState.streaming) {
      const id = editMessage.dataset.editMessage || editMessage.dataset.retryMessage;
      const message = appState.session.messages.find((m) => m.id === id);
      if (!message || message.unsent) return;
      try {
        const fork = await invoke("fork_thread", { id: appState.session.id, beforeMessageId: id });
        await refreshBootstrap(); await loadSession(fork.id);
        els.promptInput.value = message.content; appState.pendingDocuments = message.attachments || []; saveDraft(); renderAttachmentChips(); autoSizePromptInput();
        showToast("Editing a copy. Your original conversation is kept.");
        if (editMessage.dataset.retryMessage) await sendCurrentPrompt(); else els.promptInput.focus();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
      }
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
      approveBtn.disabled = true;
      try {
        appState.session = await invoke("apply_staged_edit_cmd", {
          sessionId: appState.session.id,
          editId,
        });
        await refreshWorkspaceFiles();
        renderAll();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        approveBtn.disabled = false;
      }
    } else if (rejectBtn) {
      const editId = rejectBtn.getAttribute("data-edit-reject");
      rejectBtn.disabled = true;
      try {
        appState.session = await invoke("discard_staged_edit_cmd", {
          sessionId: appState.session.id,
          editId,
        });
        renderAll();
      } catch (err) {
        showToast(friendlyRuntimeError(err));
        rejectBtn.disabled = false;
      }
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
  els.stopBtn.onclick = async () => {
    els.telemetryLeft.textContent = "Stopping…";
    await invoke("stop_generation");
    els.telemetryLeft.textContent = "Stopped";
  };
  els.promptInput.addEventListener("input", () => { autoSizePromptInput(); saveDraft(); });
  els.promptInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendCurrentPrompt();
    }
  });
}

window.addEventListener("unhandledrejection", (event) => {
  showToast(friendlyRuntimeError(event.reason));
  event.preventDefault();
});

init().catch((err) => {
  window.__KEEL_STARTUP_ERROR__ = String(err);
  els.chatScroll.innerHTML = `<div class="empty-state"><h1>Keel Desktop</h1><p>${escapeHtml(String(err))}</p></div>`;
});
