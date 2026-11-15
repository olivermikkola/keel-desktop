import { escapeHtml, renderMarkdown } from "./markdown.js";
import {
  APPEARANCE_DEFAULTS,
  SPACING_MODES,
  TEXT_SIZES,
  THEME_PALETTES,
  computeWelcomeVisible,
  countStagedEdits,
  detectViewerForPreset,
  formatBytes,
  formatRelativeTime,
  normalizeAppearance,
  shortPath,
} from "./state.js";
import { renderViewer } from "./viewers.js";

const tauri = window.__TAURI__?.core;
const events = window.__TAURI__?.event;

const APPEARANCE_STORAGE_KEY = "keel_appearance_v1";
const WELCOME_STORAGE_KEY = "keel_welcome_done";

const appState = {
  bootstrap: null,
  session: null,
  workspacePath: null,
  workspaceFiles: [],
  viewerOpen: false,
  viewerFile: null,
  streaming: false,
  streamingText: "",
  streamingThinking: "",
  streamingTools: [],
  downloadProgress: {},
  appearance: loadAppearance(),
};

const els = {
  sidebar: document.getElementById("sidebar"),
  appsList: document.getElementById("apps-list"),
  sessionsList: document.getElementById("sessions-list"),
  connectedList: document.getElementById("connected-list"),
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
  chatScroll: document.getElementById("chat-scroll"),
  approvalSlot: document.getElementById("approval-slot"),
  promptInput: document.getElementById("prompt-input"),
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
  welcomeStep1: document.getElementById("welcome-step-1"),
  welcomeStep2: document.getElementById("welcome-step-2"),
  welcomeNextBtn: document.getElementById("welcome-next-btn"),
  welcomeSkipBtn: document.getElementById("welcome-skip-btn"),
  welcomeDoneBtn: document.getElementById("welcome-done-btn"),
};

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
  document.documentElement.dataset.palette = appState.appearance.palette;
  document.documentElement.dataset.spacing = appState.appearance.spacing;
  document.documentElement.dataset.textSize = appState.appearance.textSize;
  try {
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(appState.appearance));
  } catch {
    // Ignore storage failures in restricted contexts.
  }
}

function syncWelcomeOverlay(forceShow = false) {
  let doneVal = null;
  try {
    doneVal = localStorage.getItem(WELCOME_STORAGE_KEY);
  } catch {
    doneVal = "1";
  }
  const visible = forceShow || computeWelcomeVisible(doneVal);
  els.welcomeOverlay.classList.toggle("hidden", !visible);
  els.welcomeStep1.classList.remove("hidden");
  els.welcomeStep2.classList.add("hidden");
}

function dismissWelcome() {
  try {
    localStorage.setItem(WELCOME_STORAGE_KEY, "1");
  } catch {
    // Ignore storage errors.
  }
  els.welcomeOverlay.classList.add("hidden");
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
           <button class="inline-edit-btn reject" data-edit-reject="${escapeHtml(staged.id)}">Reject</button>
           <button class="inline-edit-btn approve" data-edit-approve="${escapeHtml(staged.id)}">Approve</button>
         </div>`
      : `<div class="inline-edit-resolved ${staged.status === "applied" ? "applied" : ""}">
           <span>${statusLabel}</span>
           <span>${staged.status === "applied" ? "✓" : "×"}</span>
         </div>`;

    return `
      <details class="inline-edit-card">
        <summary class="inline-edit-head">
          ${renderEditFileSvg()}
          <span class="inline-edit-title">${escapeHtml(staged.summary)}</span>
          <span class="inline-edit-counts">
            ${plus > 0 ? `<span class="diff-plus">+${plus}</span>` : ""}
            ${minus > 0 ? `<span class="diff-minus">-${minus}</span>` : ""}
          </span>
        </summary>
        <pre class="inline-edit-diff">${escapeHtml(staged.diff || "")}</pre>
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

async function init() {
  applyAppearance(appState.appearance);
  syncWelcomeOverlay(false);

  if (events) {
    await events.listen("keel://agent-event", (ev) => handleAgentEvent(ev.payload));
    await events.listen("keel://download-progress", (ev) => {
      const p = ev.payload;
      appState.downloadProgress[p.model_id] = p;
      const bar = document.getElementById(`dl-prog-${p.model_id}`);
      if (bar) {
        const pct = p.total_bytes ? Math.round((p.downloaded_bytes / p.total_bytes) * 100) : 0;
        bar.textContent =
          p.status === "completed"
            ? "Installed"
            : `${formatBytes(p.downloaded_bytes)}${pct ? ` (${pct}%)` : ""}`;
      }
    });
  }

  await refreshBootstrap();

  if (appState.bootstrap?.sessions?.length > 0) {
    await loadSession(appState.bootstrap.sessions[0].id);
  } else {
    await createNewSession("general");
  }

  wireUi();
  renderAll();
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

async function createNewSession(preset = "general") {
  appState.session = await invoke("create_session", {
    preset,
    workspacePath: appState.workspacePath,
  });
  await refreshBootstrap();
  renderAll();
}

async function loadSession(id) {
  appState.session = await invoke("load_session_cmd", { id });
  if (appState.session.workspace_path) {
    appState.workspacePath = appState.session.workspace_path;
    await refreshWorkspaceFiles();
  }
  if (appState.session.app_preset !== "general") {
    appState.viewerOpen = true;
    const picked = detectViewerForPreset(appState.session.app_preset, appState.workspaceFiles);
    if (picked) {
      appState.viewerFile = picked;
    }
  }
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

function renderAll() {
  renderSidebar();
  renderHeader();
  renderChat();
  renderViewerPane();
}

function renderSidebar() {
  const b = appState.bootstrap;
  if (!b) return;

  els.appsList.innerHTML = "";
  const activePreset = appState.session?.app_preset || "general";
  const circleIds = ["websites", "sheets", "docs", "study", "slides"];
  for (const id of circleIds) {
    const preset = b.presets.find((p) => p.id === id);
    if (!preset) continue;
    const btn = document.createElement("button");
    btn.className = `app-circle-btn ${activePreset === preset.id ? "active" : ""}`;
    btn.title = preset.description || preset.title;
    btn.innerHTML = `
      <div class="app-circle-ring">${APP_ICONS[id] || APP_ICONS.docs}</div>
      <span>${escapeHtml(preset.title)}</span>
    `;
    btn.onclick = () => createNewSession(preset.id);
    els.appsList.appendChild(btn);
  }

  els.sessionsList.innerHTML = "";
  for (const s of b.sessions) {
    const row = document.createElement("div");
    row.className = `recent-item ${appState.session?.id === s.id ? "active" : ""}`;
    row.innerHTML = `
      <span class="recent-title">${escapeHtml(s.title)}</span>
      <span class="recent-age">${escapeHtml(formatRelativeTime(s.updated_at))}</span>
      <button class="recent-del" title="Delete chat" aria-label="Delete chat">&times;</button>
    `;
    row.onclick = (e) => {
      if (e.target.closest(".recent-del")) return;
      loadSession(s.id);
    };
    row.querySelector(".recent-del").onclick = async (e) => {
      e.stopPropagation();
      const remaining = await invoke("delete_session_cmd", { id: s.id });
      if (appState.session?.id === s.id) {
        if (remaining.length > 0) {
          await loadSession(remaining[0].id);
        } else {
          await createNewSession("general");
        }
      } else {
        await refreshBootstrap();
        renderSidebar();
      }
    };
    els.sessionsList.appendChild(row);
  }

  const mcpCount = b.mcp_servers?.length || 0;
  els.connectedList.innerHTML = `
    <button id="conn-ws-btn" class="connected-row">
      <svg class="connected-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path></svg>
      <span class="connected-label">Workspace</span>
      <span class="connected-sub mono">${escapeHtml(appState.workspacePath ? shortPath(appState.workspacePath) : "Unset")}</span>
      <span class="status-dot ${appState.workspacePath ? "ready" : ""}"></span>
    </button>
    <button id="conn-mcp-btn" class="connected-row">
      <svg class="connected-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83"></path><path d="M4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83"></path></svg>
      <span class="connected-label">MCP</span>
      <span class="connected-sub">${mcpCount ? `${mcpCount} stdio` : "Ready"}</span>
      <span class="status-dot ${mcpCount ? "ready" : ""}"></span>
    </button>
    <button id="conn-sql-btn" class="connected-row">
      <svg class="connected-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"></ellipse><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path></svg>
      <span class="connected-label">SQL sandbox</span>
      <span class="connected-sub">In-memory</span>
      <span class="status-dot ready"></span>
    </button>
  `;
  document.getElementById("conn-ws-btn").onclick = () => els.workspacePillBtn.click();
  document.getElementById("conn-mcp-btn").onclick = openMcpModal;
  document.getElementById("conn-sql-btn").onclick = () => createNewSession("sheets");
}

function renderHeader() {
  const b = appState.bootstrap;
  if (!b) return;
  const rt = b.runtime;
  els.runtimeDot.className = `status-dot ${rt.state}`;
  const selected = b.models.find(
    (m) => m.id === (rt.active_model_id || b.config.selected_model_id),
  );
  els.modelPillLabel.textContent =
    rt.state === "external"
      ? b.config.external_model_name || "External endpoint"
      : selected
        ? selected.name
        : b.config.selected_model_id || "Select model";

  els.workspacePillLabel.textContent = shortPath(appState.workspacePath);
  const harness = b.harnesses.find((h) => h.id === b.config.harness_mode);
  els.harnessPillLabel.textContent = harness ? harness.name.replace(" Harness", "") : "Keel";

  const counts = countStagedEdits(appState.session?.staged_edits ?? []);
  els.diffsPillLabel.textContent =
    counts.pending > 0
      ? `${counts.pending} pending`
      : counts.applied > 0
        ? `${counts.applied} applied`
        : "0 pending";
  els.diffsPillBtn.classList.toggle("has-pending", counts.pending > 0);
  els.telemetryRight.textContent = `Keel v${b.version}`;
}

function renderChat() {
  const s = appState.session;
  const b = appState.bootstrap;
  if (!s || !b) return;

  if (s.messages.length === 0 && !appState.streaming) {
    const preset = b.presets.find((p) => p.id === s.app_preset) || b.presets[0];
    const starters = preset.starters
      .map(
        (st) =>
          `<button class="starter-card" data-starter="${escapeHtml(st)}">${escapeHtml(st)}</button>`,
      )
      .join("");
    els.chatScroll.innerHTML = `
      <div class="empty-state">
        <h1>${escapeHtml(preset.title)}</h1>
        <p>${escapeHtml(preset.description)}</p>
        <div class="starters-grid">${starters}</div>
      </div>
    `;
    els.chatScroll.querySelectorAll("[data-starter]").forEach((btn) => {
      btn.onclick = () => {
        els.promptInput.value = btn.getAttribute("data-starter");
        sendCurrentPrompt();
      };
    });
    return;
  }

  let html = "";
  for (const msg of s.messages) {
    html += renderMessageHtml(msg);
  }

  if (appState.streaming) {
    const toolsHtml = appState.streamingTools.map(renderToolOrEditHtml).join("");
    const thinkingHtml = appState.streamingThinking
      ? `<details class="thinking-box"><summary>Thought</summary><pre class="mono">${escapeHtml(appState.streamingThinking)}</pre></details>`
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
  els.chatScroll.scrollTop = els.chatScroll.scrollHeight;
}

function renderMessageHtml(msg) {
  if (msg.role === "user") {
    return `<div class="msg user"><div class="bubble">${escapeHtml(msg.content)}</div></div>`;
  }
  const thinkingHtml = msg.thinking
    ? `<details class="thinking-box"><summary>Thought</summary><pre class="mono">${escapeHtml(msg.thinking)}</pre></details>`
    : "";
  const toolsHtml = (msg.tools || []).map(renderToolOrEditHtml).join("");
  return `
    <div class="msg assistant">
      ${thinkingHtml}
      ${toolsHtml}
      <div class="bubble markdown-body">${renderMarkdown(msg.content)}</div>
    </div>
  `;
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
    els.telemetryLeft.textContent = `${ev.prompt_tokens} prompt · ${ev.completion_tokens} completion · ${ev.tokens_per_sec.toFixed(1)} tok/s`;
  }
  renderChat();
}

function renderApprovalCard(req) {
  els.approvalSlot.innerHTML = `
    <div class="approval-card">
      <div><strong>Approval required:</strong> ${escapeHtml(req.summary)}</div>
      <div class="approval-actions">
        <button id="appr-once" class="btn btn-primary btn-sm">Allow once</button>
        <button id="appr-always" class="btn btn-sm">Always allow</button>
        <button id="appr-deny" class="btn btn-rust btn-sm">Deny</button>
      </div>
    </div>
  `;
  const decide = async (decision) => {
    els.approvalSlot.innerHTML = "";
    await invoke("respond_approval", { id: req.id, decision });
  };
  document.getElementById("appr-once").onclick = () => decide("allow_once");
  document.getElementById("appr-always").onclick = () => decide("allow_always");
  document.getElementById("appr-deny").onclick = () => decide("deny");
}

async function sendCurrentPrompt() {
  if (appState.streaming || !appState.session) return;
  const prompt = els.promptInput.value.trim();
  if (!prompt) return;

  els.promptInput.value = "";
  autoSizePromptInput();
  appState.session.messages.push({
    id: `local-${Date.now()}`,
    role: "user",
    content: prompt,
    thinking: null,
    tools: [],
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
        workspace_path: appState.workspacePath,
      },
    });
    await refreshBootstrap();
    if (appState.session.staged_edits?.length > 0 && !appState.viewerFile) {
      const firstFile = appState.session.staged_edits[0].path;
      appState.viewerFile = firstFile;
    }
  } catch (err) {
    appState.session.messages.push({
      id: `err-${Date.now()}`,
      role: "assistant",
      content: `Runtime error: ${String(err)}`,
      thinking: null,
      tools: [],
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
  els.viewerPane.classList.toggle("hidden", !appState.viewerOpen);
  if (!appState.viewerOpen) return;
  renderViewerOptions();
  loadViewerFile();
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

async function loadViewerFile() {
  if (!appState.workspacePath || !appState.viewerFile) {
    renderViewer(els.viewerContent, null, "");
    return;
  }
  try {
    const content = await invoke("read_workspace_file_raw", {
      workspacePath: appState.workspacePath,
      relPath: appState.viewerFile,
    });
    renderViewer(els.viewerContent, appState.viewerFile, content);
  } catch (err) {
    els.viewerContent.innerHTML = `<div class="viewer-empty">${escapeHtml(String(err))}</div>`;
  }
}

function openModal(title, builder) {
  els.modalTitle.textContent = title;
  els.modalBody.innerHTML = "";
  builder(els.modalBody);
  els.modalBackdrop.classList.remove("hidden");
}

function closeModal() {
  els.modalBackdrop.classList.add("hidden");
}

function openModelsModal() {
  const b = appState.bootstrap;
  openModal("Models", (body) => {
    const rt = b.runtime;
    const activeModel = b.models.find((m) => m.id === rt.active_model_id);

    if (rt.state === "ready" || rt.state === "starting" || rt.state === "external") {
      const running = document.createElement("div");
      running.className = "running-now-card";
      const runningTitle =
        rt.state === "external"
          ? b.config.external_model_name || "External OpenAI Endpoint"
          : activeModel?.name || rt.active_model_id || "Local Model";
      const runningMeta =
        rt.state === "external"
          ? rt.base_url || "Custom endpoint"
          : `${activeModel ? `${formatBytes(activeModel.bytes)} · ${activeModel.quantization}` : "GGUF"} · llama.cpp`;
      running.innerHTML = `
        <div>
          <div class="running-eyebrow">${rt.state === "starting" ? "STARTING RUNTIME" : "RUNNING NOW"}</div>
          <div class="running-title">${escapeHtml(runningTitle)}</div>
          <div class="running-meta">${escapeHtml(runningMeta)}</div>
        </div>
        <button id="stop-runtime-btn" class="pill-action-use">Stop</button>
      `;
      running.querySelector("#stop-runtime-btn").onclick = async () => {
        await invoke("stop_runtime");
        await refreshBootstrap();
        renderHeader();
        openModelsModal();
      };
      body.appendChild(running);
    }

    const list = document.createElement("div");
    list.className = "model-list-clean";
    for (const m of b.models) {
      const isRunning = rt.active_model_id === m.id && (rt.state === "ready" || rt.state === "starting");
      const row = document.createElement("div");
      row.className = "model-clean-row";
      row.innerHTML = `
        <div>
          <div class="model-clean-title">
            <span>${escapeHtml(m.name)}</span>
            <span class="model-clean-quant">${escapeHtml(m.quantization)}</span>
          </div>
          <div class="model-clean-sub">
            ${escapeHtml(m.detail)} · ${formatBytes(m.bytes)}
            <span id="dl-prog-${escapeHtml(m.id)}" style="color:var(--accent);margin-left:6px"></span>
          </div>
        </div>
        ${
          m.installed
            ? `<button class="pill-action-use" data-start="${escapeHtml(m.id)}">${isRunning ? "Reload" : "Use"}</button>`
            : `<button class="pill-action-get" data-dl="${escapeHtml(m.id)}">Get</button>`
        }
      `;
      row.querySelector("[data-start]")?.addEventListener("click", async () => {
        await invoke("start_model", { modelId: m.id });
        await refreshBootstrap();
        renderHeader();
        closeModal();
      });
      row.querySelector("[data-dl]")?.addEventListener("click", async () => {
        await invoke("download_catalog_model", { modelId: m.id });
        await refreshBootstrap();
        openModelsModal();
      });
      list.appendChild(row);
    }
    body.appendChild(list);

    const toggleBtn = document.createElement("button");
    toggleBtn.className = "import-gguf-toggle";
    toggleBtn.textContent = "+ Import your own .gguf or external endpoint";
    const impPanel = document.createElement("div");
    impPanel.className = "hidden";
    impPanel.style.display = "flex";
    impPanel.style.flexDirection = "column";
    impPanel.style.gap = "10px";
    impPanel.innerHTML = `
      <div class="card">
        <strong>Import GGUF</strong>
        <div class="row">
          <input id="hf-url" class="input grow" placeholder="Hugging Face URL or repo:owner/repo" />
          <button id="hf-dl-btn" class="btn btn-sm">Download</button>
        </div>
        <div class="row">
          <input id="local-gguf" class="input grow mono" placeholder="/path/to/local-model.gguf" />
          <button id="local-imp-btn" class="btn btn-sm">Import</button>
        </div>
      </div>
      <div class="card">
        <strong>External OpenAI-compatible Endpoint</strong>
        <input id="ext-url" class="input mono" value="${escapeHtml(b.config.external_base_url || "")}" placeholder="http://127.0.0.1:8080/v1" />
        <div class="row">
          <input id="ext-model" class="input grow" value="${escapeHtml(b.config.external_model_name || "")}" placeholder="Model name (optional)" />
          <button id="ext-save-btn" class="btn btn-primary btn-sm">Use Endpoint</button>
        </div>
      </div>
    `;
    toggleBtn.onclick = () => impPanel.classList.toggle("hidden");
    body.appendChild(toggleBtn);
    body.appendChild(impPanel);

    impPanel.querySelector("#hf-dl-btn").onclick = async () => {
      await invoke("download_huggingface_gguf", { url: impPanel.querySelector("#hf-url").value });
      await refreshBootstrap();
      openModelsModal();
    };
    impPanel.querySelector("#local-imp-btn").onclick = async () => {
      await invoke("import_local_gguf", { path: impPanel.querySelector("#local-gguf").value });
      await refreshBootstrap();
      openModelsModal();
    };
    impPanel.querySelector("#ext-save-btn").onclick = async () => {
      await invoke("set_external_endpoint", {
        baseUrl: impPanel.querySelector("#ext-url").value,
        modelName: impPanel.querySelector("#ext-model").value || null,
      });
      await refreshBootstrap();
      renderHeader();
      closeModal();
    };
  });
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
        <pre class="diff-pre">${escapeHtml(edit.diff)}</pre>
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

function openMcpModal() {
  const b = appState.bootstrap;
  openModal("Connections (Stdio MCP)", (body) => {
    const form = document.createElement("div");
    form.className = "card";
    form.innerHTML = `
      <strong>Add Stdio MCP Server</strong>
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
    `;
    form.querySelector("#mcp-add-btn").onclick = async () => {
      const args = form.querySelector("#mcp-args").value.trim().split(/\s+/).filter(Boolean);
      await invoke("save_mcp_server", {
        input: {
          name: form.querySelector("#mcp-name").value,
          command: form.querySelector("#mcp-cmd").value,
          args,
          scope: form.querySelector("#mcp-scope").value,
          workspace_path: appState.workspacePath,
        },
      });
      await refreshBootstrap();
      renderSidebar();
      openMcpModal();
    };
    body.appendChild(form);

    for (const srv of b.mcp_servers) {
      const card = document.createElement("div");
      card.className = "card";
      card.innerHTML = `
        <div class="row">
          <strong class="grow">${escapeHtml(srv.name)}</strong>
          <span class="mono" style="font-size:12px;color:var(--muted)">${escapeHtml(srv.command)} ${escapeHtml(srv.args.join(" "))}</span>
          <button class="btn btn-ghost btn-sm" data-rm="${escapeHtml(srv.name)}">Remove</button>
        </div>
      `;
      card.querySelector("[data-rm]").onclick = async () => {
        await invoke("remove_mcp_server", {
          name: srv.name,
          workspacePath: appState.workspacePath,
        });
        await refreshBootstrap();
        renderSidebar();
        openMcpModal();
      };
      body.appendChild(card);
    }
  });
}

function openCustomizeModal() {
  const b = appState.bootstrap;
  openModal("Customize", (body) => {
    const ap = appState.appearance;
    const appearanceCard = document.createElement("div");
    appearanceCard.className = "card";
    const palettesHtml = THEME_PALETTES.map(
      (p) => `
      <button type="button" class="palette-chip ${ap.palette === p.id ? "active" : ""}" data-palette-pick="${p.id}">
        <span class="palette-swatch" style="background:linear-gradient(135deg, ${p.ink} 50%, ${p.accent} 50%)"></span>
        <span>${escapeHtml(p.label)}</span>
      </button>
    `,
    ).join("");
    const spacingHtml = SPACING_MODES.map(
      (sp) =>
        `<button type="button" class="seg-btn ${ap.spacing === sp ? "active" : ""}" data-spacing-pick="${sp}">${sp[0].toUpperCase() + sp.slice(1)}</button>`,
    ).join("");
    const textSizeHtml = TEXT_SIZES.map(
      (sz) =>
        `<button type="button" class="seg-btn ${ap.textSize === sz ? "active" : ""}" data-size-pick="${sz}">${sz[0].toUpperCase() + sz.slice(1)}</button>`,
    ).join("");

    appearanceCard.innerHTML = `
      <strong>Palette</strong>
      <div class="palette-grid">${palettesHtml}</div>
      <strong>Message spacing</strong>
      <div class="seg-group">${spacingHtml}</div>
      <strong>Text size</strong>
      <div class="seg-group">${textSizeHtml}</div>
    `;
    appearanceCard.querySelectorAll("[data-palette-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, palette: btn.getAttribute("data-palette-pick") });
        openCustomizeModal();
      };
    });
    appearanceCard.querySelectorAll("[data-spacing-pick]").forEach((btn) => {
      btn.onclick = () => {
        applyAppearance({ ...appState.appearance, spacing: btn.getAttribute("data-spacing-pick") });
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
          `<option value="${escapeHtml(h.id)}" ${h.id === b.config.harness_mode ? "selected" : ""}>${escapeHtml(h.name)}${h.available ? "" : " (CLI not detected)"}</option>`,
      )
      .join("");
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <label><strong>Agent Harness</strong></label>
      <select id="cust-harness" class="select-field">${harnessOptions}</select>
      <label><strong>Custom Instructions</strong></label>
      <textarea id="cust-inst" class="textarea-field" rows="3">${escapeHtml(b.config.custom_instructions)}</textarea>
      <label class="row"><input id="cust-auto" type="checkbox" ${b.config.auto_apply_edits ? "checked" : ""} /> Auto-apply workspace file edits without review cards</label>
      <label><strong>llama-server binary path (optional override)</strong></label>
      <input id="cust-llama" class="input mono" value="${escapeHtml(b.config.llama_server_path || "")}" placeholder="/path/to/llama-server" />
      <div class="row">
        <button id="cust-reset-welcome" type="button" class="btn btn-ghost btn-sm">Reset Welcome</button>
        <div class="spacer"></div>
        <button id="cust-save" class="btn btn-primary btn-sm">Save settings</button>
      </div>
    `;
    card.querySelector("#cust-reset-welcome").onclick = () => {
      try {
        localStorage.removeItem(WELCOME_STORAGE_KEY);
      } catch {
        // Ignore storage failures.
      }
      closeModal();
      syncWelcomeOverlay(true);
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

function autoSizePromptInput() {
  els.promptInput.style.height = "auto";
  els.promptInput.style.height = `${Math.min(els.promptInput.scrollHeight, 140)}px`;
}

function wireUi() {
  els.toggleSidebarBtn.onclick = () => els.sidebar.classList.toggle("collapsed");
  els.newChatBtn.onclick = () => createNewSession(appState.session?.app_preset || "general");
  els.newAgentBtn.onclick = () => createNewSession("general");
  document.getElementById("open-models-btn").onclick = openModelsModal;
  els.modelPillBtn.onclick = openModelsModal;
  document.getElementById("open-mcp-btn").onclick = openMcpModal;
  document.getElementById("open-customize-btn").onclick = openCustomizeModal;
  els.harnessPillBtn.onclick = openCustomizeModal;
  els.diffsPillBtn.onclick = openDiffsModal;
  document.getElementById("modal-close-btn").onclick = closeModal;

  els.welcomeNextBtn.onclick = () => {
    els.welcomeStep1.classList.add("hidden");
    els.welcomeStep2.classList.remove("hidden");
  };
  els.welcomeSkipBtn.onclick = dismissWelcome;
  els.welcomeDoneBtn.onclick = dismissWelcome;

  els.chatScroll.addEventListener("click", async (e) => {
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

  els.workspacePillBtn.onclick = async () => {
    const picked = await invoke("pick_workspace_folder");
    if (picked) {
      appState.workspacePath = await invoke("set_workspace_folder", { path: picked });
      await refreshBootstrap();
      renderAll();
    } else {
      openModal("Select Workspace Folder", (body) => {
        const div = document.createElement("div");
        div.className = "card";
        div.innerHTML = `
          <label>Workspace Folder Path</label>
          <div class="row">
            <input id="ws-path-in" class="input grow mono" value="${escapeHtml(appState.workspacePath || "")}" placeholder="/path/to/project" />
            <button id="ws-set-btn" class="btn btn-primary btn-sm">Open folder</button>
          </div>
        `;
        div.querySelector("#ws-set-btn").onclick = async () => {
          const path = div.querySelector("#ws-path-in").value.trim();
          if (!path) return;
          appState.workspacePath = await invoke("set_workspace_folder", { path });
          await refreshBootstrap();
          renderAll();
          closeModal();
        };
        body.appendChild(div);
      });
    }
  };

  els.toggleViewerBtn.onclick = () => {
    appState.viewerOpen = !appState.viewerOpen;
    renderViewerPane();
  };
  els.closeViewerBtn.onclick = () => {
    appState.viewerOpen = false;
    renderViewerPane();
  };
  els.refreshViewerBtn.onclick = () => refreshWorkspaceFiles().then(loadViewerFile);
  els.viewerFileSelect.onchange = () => {
    appState.viewerFile = els.viewerFileSelect.value;
    loadViewerFile();
  };

  els.sendBtn.onclick = sendCurrentPrompt;
  els.stopBtn.onclick = () => invoke("stop_generation");
  els.promptInput.addEventListener("input", autoSizePromptInput);
  els.promptInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendCurrentPrompt();
    }
  });
}

init().catch((err) => {
  els.chatScroll.innerHTML = `<div class="empty-state"><h1>Keel</h1><p>${escapeHtml(String(err))}</p></div>`;
});
