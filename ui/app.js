import { escapeHtml, renderMarkdown } from "./markdown.js";
import { countStagedEdits, detectViewerForPreset, formatBytes, shortPath } from "./state.js";
import { renderViewer } from "./viewers.js";

const tauri = window.__TAURI__?.core;
const events = window.__TAURI__?.event;

const appState = {
  bootstrap: null,
  session: null,
  workspacePath: null,
  workspaceFiles: [],
  viewerOpen: false,
  viewerFile: null,
  running: false,
  streamingText: "",
  streamingThinking: "",
  streamingTools: [],
  pendingApproval: null,
  downloadProgress: {},
};

const els = {
  sidebar: document.getElementById("sidebar"),
  appsList: document.getElementById("apps-list"),
  sessionsList: document.getElementById("sessions-list"),
  modelPillBtn: document.getElementById("model-pill-btn"),
  modelPillLabel: document.getElementById("model-pill-label"),
  runtimeDot: document.getElementById("runtime-dot"),
  workspacePillBtn: document.getElementById("workspace-pill-btn"),
  workspacePillLabel: document.getElementById("workspace-pill-label"),
  harnessPillBtn: document.getElementById("harness-pill-btn"),
  harnessPillLabel: document.getElementById("harness-pill-label"),
  diffsPillBtn: document.getElementById("diffs-pill-btn"),
  diffsPillLabel: document.getElementById("diffs-pill-label"),
  chatScroll: document.getElementById("chat-scroll"),
  approvalSlot: document.getElementById("approval-slot"),
  promptInput: document.getElementById("prompt-input"),
  thinkingToggle: document.getElementById("thinking-toggle"),
  sendBtn: document.getElementById("send-btn"),
  stopBtn: document.getElementById("stop-btn"),
  viewerPane: document.getElementById("viewer-pane"),
  viewerSelect: document.getElementById("viewer-file-select"),
  viewerContent: document.getElementById("viewer-content"),
  telemetryLeft: document.getElementById("telemetry-left"),
  telemetryRight: document.getElementById("telemetry-right"),
  modalBackdrop: document.getElementById("modal-backdrop"),
  modalTitle: document.getElementById("modal-title"),
  modalBody: document.getElementById("modal-body"),
};

async function invoke(cmd, args = {}) {
  if (!tauri) {
    throw new Error("Tauri runtime API is unavailable outside the desktop window.");
  }
  return tauri.invoke(cmd, args);
}

async function init() {
  if (events) {
    await events.listen("keel://agent-event", (ev) => handleAgentEvent(ev.payload));
    await events.listen("keel://download-progress", (ev) => {
      const p = ev.payload;
      appState.downloadProgress[p.model_id] = p;
      const bar = document.getElementById(`dl-prog-${p.model_id}`);
      if (bar) {
        const pct = p.total_bytes ? Math.round((p.downloaded_bytes / p.total_bytes) * 100) : 0;
        bar.textContent = p.done
          ? "Verified & ready"
          : `${formatBytes(p.downloaded_bytes)} (${pct}%)`;
      }
    });
  }

  wireEvents();
  await refreshBootstrap();
  if (appState.bootstrap.sessions.length > 0) {
    await openSession(appState.bootstrap.sessions[0].id);
  } else {
    await startNewChat("general");
  }
}

async function refreshBootstrap() {
  appState.bootstrap = await invoke("get_bootstrap_state", {
    workspacePath: appState.workspacePath,
  });
  if (!appState.workspacePath && appState.bootstrap.config.recent_workspaces.length > 0) {
    appState.workspacePath = appState.bootstrap.config.recent_workspaces[0];
  }
  renderSidebar();
  renderHeader();
  if (appState.workspacePath) {
    await refreshWorkspaceFiles();
  }
}

async function startNewChat(preset = "general") {
  appState.session = await invoke("create_session", {
    preset,
    workspacePath: appState.workspacePath,
  });
  await refreshBootstrap();
  renderChat();
}

async function openSession(id) {
  appState.session = await invoke("load_session_cmd", { id });
  if (appState.session.workspace_path) {
    appState.workspacePath = appState.session.workspace_path;
    await refreshWorkspaceFiles();
  }
  renderHeader();
  renderSidebar();
  renderChat();
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

function renderSidebar() {
  const b = appState.bootstrap;
  if (!b) return;
  const activePreset = appState.session?.app_preset ?? "general";

  els.appsList.innerHTML = "";
  for (const preset of b.presets) {
    const btn = document.createElement("button");
    btn.className = `nav-item ${activePreset === preset.id ? "active" : ""}`;
    btn.innerHTML = `<span>${escapeHtml(preset.title)}</span><span class="sub">${preset.id === "general" ? "Code" : "App"}</span>`;
    btn.addEventListener("click", () => startNewChat(preset.id));
    els.appsList.appendChild(btn);
  }

  els.sessionsList.innerHTML = "";
  for (const s of b.sessions) {
    const btn = document.createElement("button");
    btn.className = `nav-item ${appState.session?.id === s.id ? "active" : ""}`;
    btn.innerHTML = `<span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${escapeHtml(s.title)}</span><span class="sub">${s.message_count}</span>`;
    btn.addEventListener("click", () => openSession(s.id));
    els.sessionsList.appendChild(btn);
  }
}

function renderHeader() {
  const b = appState.bootstrap;
  if (!b) return;
  const rt = b.runtime;
  els.runtimeDot.className = `status-dot ${rt.state}`;
  const selected = b.models.find((m) => m.id === (rt.active_model_id || b.config.selected_model_id));
  els.modelPillLabel.textContent =
    rt.state === "external"
      ? b.config.external_model_name || "External endpoint"
      : selected
        ? `${selected.name} (${rt.state})`
        : b.config.selected_model_id;

  els.workspacePillLabel.textContent = shortPath(appState.workspacePath);
  const harness = b.harnesses.find((h) => h.id === b.config.harness_mode);
  els.harnessPillLabel.textContent = harness ? harness.name : "Keel Harness";

  const counts = countStagedEdits(appState.session?.staged_edits ?? []);
  els.diffsPillLabel.textContent = `${counts.applied} applied · ${counts.pending} awaiting review`;
  els.telemetryRight.textContent = `Keel v${b.version}`;
}

function renderChat() {
  const s = appState.session;
  const b = appState.bootstrap;
  if (!s || !b) return;

  if (s.messages.length === 0 && !appState.running) {
    const preset = b.presets.find((p) => p.id === s.app_preset) || b.presets[0];
    const starters = preset.starters
      .map(
        (st) =>
          `<button class="starter-card" data-starter="${escapeHtml(st)}">${escapeHtml(st)}</button>`,
      )
      .join("");
    els.chatScroll.innerHTML = `
      <div class="empty-state">
        <img src="assets/mark.svg" alt="Keel" />
        <h1>${escapeHtml(preset.title)}</h1>
        <p>${escapeHtml(preset.summary)}</p>
        <div class="starters-grid">${starters}</div>
      </div>
    `;
    els.chatScroll.querySelectorAll("[data-starter]").forEach((btn) => {
      btn.addEventListener("click", () => {
        els.promptInput.value = btn.getAttribute("data-starter") || "";
        sendCurrentPrompt();
      });
    });
    return;
  }

  const fragments = [];
  for (const msg of s.messages) {
    fragments.push(renderMessageHtml(msg));
  }

  if (appState.running) {
    const liveTools = appState.streamingTools
      .map(
        (t) =>
          `<div class="tool-card ${t.is_error ? "error" : ""}"><strong>${escapeHtml(t.name)}</strong>${
            t.result ? `<pre class="mono">${escapeHtml(t.result)}</pre>` : " <span>running...</span>"
          }</div>`,
      )
      .join("");
    const liveThink = appState.streamingThinking
      ? `<details class="thinking-box"><summary>Thinking</summary><pre class="mono">${escapeHtml(appState.streamingThinking)}</pre></details>`
      : "";
    fragments.push(`
      <div class="msg assistant">
        ${liveThink}
        ${liveTools}
        <div class="bubble markdown-body">${renderMarkdown(appState.streamingText || "Thinking...")}</div>
      </div>
    `);
  }

  els.chatScroll.innerHTML = fragments.join("\n");
  els.chatScroll.scrollTop = els.chatScroll.scrollHeight;
}

function renderMessageHtml(msg) {
  if (msg.role === "user") {
    return `<div class="msg user"><div class="bubble">${escapeHtml(msg.content)}</div></div>`;
  }
  const think = msg.thinking
    ? `<details class="thinking-box"><summary>Thinking</summary><pre class="mono">${escapeHtml(msg.thinking)}</pre></details>`
    : "";
  const tools = (msg.tool_calls || [])
    .map(
      (t) =>
        `<details class="tool-card ${t.is_error ? "error" : ""}"><summary><strong>${escapeHtml(t.name)}</strong></summary><pre class="mono">${escapeHtml(t.result || "")}</pre></details>`,
    )
    .join("");
  return `
    <div class="msg assistant">
      ${think}
      ${tools}
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
    appState.streamingTools.push({ name: ev.name, result: null, is_error: false });
  } else if (ev.type === "tool_call_result") {
    const last = [...appState.streamingTools].reverse().find((t) => t.name === ev.name && !t.result);
    if (last) {
      last.result = ev.result;
      last.is_error = ev.is_error;
    }
  } else if (ev.type === "approval_required") {
    renderApprovalCard(ev);
  } else if (ev.type === "staged_edit_added") {
    if (appState.session) {
      appState.session.staged_edits.push(ev.edit);
      renderHeader();
    }
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
  const prompt = els.promptInput.value.trim();
  if (!prompt || appState.running || !appState.session) return;

  els.promptInput.value = "";
  appState.running = true;
  appState.streamingText = "";
  appState.streamingThinking = "";
  appState.streamingTools = [];
  els.sendBtn.classList.add("hidden");
  els.stopBtn.classList.remove("hidden");

  appState.session.messages.push({
    id: `pending-${Date.now()}`,
    role: "user",
    content: prompt,
    thinking: null,
    tool_calls: [],
  });
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
    await refreshWorkspaceFiles();
    await refreshBootstrap();
    if (appState.viewerOpen) {
      await loadSelectedViewerFile();
    }
  } catch (err) {
    appState.session.messages.push({
      id: `err-${Date.now()}`,
      role: "assistant",
      content: `**Error:** ${String(err)}`,
      thinking: null,
      tool_calls: [],
    });
  } finally {
    appState.running = false;
    els.approvalSlot.innerHTML = "";
    els.stopBtn.classList.add("hidden");
    els.sendBtn.classList.remove("hidden");
    renderHeader();
    renderChat();
  }
}

function renderViewerOptions() {
  const files = appState.workspaceFiles.filter((f) => !f.endsWith("/"));
  els.viewerSelect.innerHTML = files
    .map((f) => `<option value="${escapeHtml(f)}">${escapeHtml(f)}</option>`)
    .join("");
  if (!appState.viewerFile || !files.includes(appState.viewerFile)) {
    appState.viewerFile = detectViewerForPreset(appState.session?.app_preset, files);
  }
  if (appState.viewerFile) {
    els.viewerSelect.value = appState.viewerFile;
  }
}

async function loadSelectedViewerFile() {
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

function openModal(title, renderFn) {
  els.modalTitle.textContent = title;
  els.modalBody.innerHTML = "";
  renderFn(els.modalBody);
  els.modalBackdrop.classList.remove("hidden");
}

function closeModal() {
  els.modalBackdrop.classList.add("hidden");
}

function openModelsModal() {
  const b = appState.bootstrap;
  openModal("Models & Local Runtime", (body) => {
    const ext = document.createElement("div");
    ext.className = "card";
    ext.innerHTML = `
      <strong>Connect to an existing server (llama-server / LM Studio / Ollama)</strong>
      <div class="row">
        <input id="ext-url" class="input grow mono" placeholder="http://127.0.0.1:8080/v1" value="${escapeHtml(b.config.external_base_url || "http://127.0.0.1:8080/v1")}" />
        <input id="ext-model" class="input mono" placeholder="model id (optional)" value="${escapeHtml(b.config.external_model_name || "")}" />
        <button id="ext-connect-btn" class="btn btn-primary btn-sm">Connect</button>
      </div>
    `;
    body.appendChild(ext);
    ext.querySelector("#ext-connect-btn").onclick = async () => {
      await invoke("set_external_endpoint", {
        baseUrl: ext.querySelector("#ext-url").value,
        modelName: ext.querySelector("#ext-model").value || null,
      });
      await refreshBootstrap();
      closeModal();
    };

    const imp = document.createElement("div");
    imp.className = "card";
    imp.innerHTML = `
      <strong>Import GGUF from disk or Hugging Face URL</strong>
      <div class="row">
        <input id="hf-url" class="input grow mono" placeholder="https://huggingface.co/.../resolve/main/model.gguf" />
        <button id="hf-dl-btn" class="btn btn-sm">Download URL</button>
      </div>
      <div class="row">
        <input id="local-gguf" class="input grow mono" placeholder="/path/to/local/model.gguf" />
        <button id="local-imp-btn" class="btn btn-sm">Link file</button>
      </div>
    `;
    body.appendChild(imp);
    imp.querySelector("#hf-dl-btn").onclick = async () => {
      await invoke("download_huggingface_gguf", { url: imp.querySelector("#hf-url").value });
      await refreshBootstrap();
      openModelsModal();
    };
    imp.querySelector("#local-imp-btn").onclick = async () => {
      await invoke("import_local_gguf", { path: imp.querySelector("#local-gguf").value });
      await refreshBootstrap();
      openModelsModal();
    };

    for (const m of b.models) {
      const card = document.createElement("div");
      card.className = "card";
      card.innerHTML = `
        <div class="row">
          <strong class="grow">${escapeHtml(m.name)}</strong>
          <span class="mono" style="font-size:12px;color:var(--muted)">${escapeHtml(m.quantization)} · ${formatBytes(m.bytes)}</span>
        </div>
        <div style="font-size:12.5px;color:var(--muted)">${escapeHtml(m.detail)}</div>
        <div class="row">
          <span id="dl-prog-${escapeHtml(m.id)}" class="grow mono" style="font-size:12px;color:var(--accent)">${m.installed ? "Installed" : "Not downloaded"}</span>
          ${
            m.installed
              ? `<button class="btn btn-primary btn-sm" data-start="${escapeHtml(m.id)}">Start runtime</button>`
              : `<button class="btn btn-sm" data-dl="${escapeHtml(m.id)}">Download</button>`
          }
        </div>
      `;
      card.querySelector("[data-start]")?.addEventListener("click", async () => {
        await invoke("start_model", { modelId: m.id });
        await refreshBootstrap();
        closeModal();
      });
      card.querySelector("[data-dl]")?.addEventListener("click", async () => {
        await invoke("download_catalog_model", { modelId: m.id });
        await refreshBootstrap();
        openModelsModal();
      });
      body.appendChild(card);
    }
  });
}

function openDiffsModal() {
  const edits = appState.session?.staged_edits ?? [];
  openModal("Workspace File Changes", (body) => {
    const top = document.createElement("div");
    top.className = "row";
    top.innerHTML = `
      <span class="grow" style="color:var(--muted)">Review proposed edits before writing them to disk.</span>
      <button id="apply-all-btn" class="btn btn-primary btn-sm">Apply all pending</button>
    `;
    body.appendChild(top);
    top.querySelector("#apply-all-btn").onclick = async () => {
      appState.session = await invoke("apply_all_staged_edits", { sessionId: appState.session.id });
      await refreshWorkspaceFiles();
      renderHeader();
      openDiffsModal();
    };

    if (edits.length === 0) {
      const empty = document.createElement("div");
      empty.textContent = "No staged edits in this chat yet.";
      empty.style.color = "var(--muted)";
      body.appendChild(empty);
      return;
    }

    for (const edit of edits) {
      const card = document.createElement("div");
      card.className = "card";
      card.innerHTML = `
        <div class="row">
          <strong class="grow mono">${escapeHtml(edit.summary)}</strong>
          <span class="mono" style="font-size:12px;color:var(--accent)">${escapeHtml(edit.status)}</span>
        </div>
        <pre class="diff-pre mono">${escapeHtml(edit.diff)}</pre>
      `;
      if (edit.status === "pending") {
        const actions = document.createElement("div");
        actions.className = "row";
        const applyBtn = document.createElement("button");
        applyBtn.className = "btn btn-primary btn-sm";
        applyBtn.textContent = "Apply";
        applyBtn.onclick = async () => {
          appState.session = await invoke("apply_staged_edit_cmd", {
            sessionId: appState.session.id,
            editId: edit.id,
          });
          await refreshWorkspaceFiles();
          renderHeader();
          openDiffsModal();
        };
        const discBtn = document.createElement("button");
        discBtn.className = "btn btn-ghost btn-sm";
        discBtn.textContent = "Discard";
        discBtn.onclick = async () => {
          appState.session = await invoke("discard_staged_edit_cmd", {
            sessionId: appState.session.id,
            editId: edit.id,
          });
          renderHeader();
          openDiffsModal();
        };
        actions.appendChild(applyBtn);
        actions.appendChild(discBtn);
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
      <strong>Add local MCP server</strong>
      <div class="row">
        <input id="mcp-name" class="input" placeholder="name (e.g. filesystem)" />
        <input id="mcp-cmd" class="input grow mono" placeholder="command (e.g. npx)" />
        <input id="mcp-args" class="input grow mono" placeholder="args (space-separated)" />
        <button id="mcp-save" class="btn btn-primary btn-sm">Add</button>
      </div>
    `;
    body.appendChild(form);
    form.querySelector("#mcp-save").onclick = async () => {
      const args = form
        .querySelector("#mcp-args")
        .value.split(/\s+/)
        .map((s) => s.trim())
        .filter(Boolean);
      await invoke("save_mcp_server", {
        input: {
          name: form.querySelector("#mcp-name").value,
          command: form.querySelector("#mcp-cmd").value,
          args,
        },
        workspacePath: appState.workspacePath,
      });
      await refreshBootstrap();
      openMcpModal();
    };

    for (const srv of b.mcp_servers) {
      const card = document.createElement("div");
      card.className = "card";
      card.innerHTML = `
        <div class="row">
          <strong class="grow">${escapeHtml(srv.name)}</strong>
          <span class="mono" style="font-size:12px;color:var(--muted)">${escapeHtml(srv.command)} ${escapeHtml(srv.args.join(" "))}</span>
          <button class="btn btn-ghost btn-sm" data-rm="${escapeHtml(srv.name)}">Remove</button>
        </div>
        <div style="font-size:12px;color:${srv.error ? "var(--danger)" : "var(--accent)"}">
          ${srv.error ? escapeHtml(srv.error) : `${srv.tools.length} tool(s) loaded`}
        </div>
      `;
      card.querySelector("[data-rm]").onclick = async () => {
        await invoke("remove_mcp_server", {
          name: srv.name,
          workspacePath: appState.workspacePath,
        });
        await refreshBootstrap();
        openMcpModal();
      };
      body.appendChild(card);
    }
  });
}

function openCustomizeModal() {
  const b = appState.bootstrap;
  openModal("Customize & Harness", (body) => {
    const harnessOptions = b.harnesses
      .map(
        (h) =>
          `<option value="${escapeHtml(h.id)}" ${b.config.harness_mode === h.id ? "selected" : ""} ${!h.available ? "disabled" : ""}>${escapeHtml(h.name)}${h.available ? "" : " (not on PATH)"}</option>`,
      )
      .join("");
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <label>Standing instructions (added to every chat)</label>
      <textarea id="cust-inst" class="textarea-field" rows="4">${escapeHtml(b.config.custom_instructions)}</textarea>
      <label class="row"><input id="cust-auto" type="checkbox" ${b.config.auto_apply_edits ? "checked" : ""} /> Auto-apply workspace file edits without staging diffs</label>
      <label>Agent harness</label>
      <select id="cust-harness">${harnessOptions}</select>
      <label>llama-server binary path (optional override)</label>
      <input id="cust-llama" class="input mono" value="${escapeHtml(b.config.llama_server_path || "")}" placeholder="/path/to/llama-server" />
      <div class="row"><button id="cust-save" class="btn btn-primary btn-sm">Save settings</button></div>
    `;
    body.appendChild(card);
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
      closeModal();
    };
  });
}

function wireEvents() {
  document.getElementById("toggle-sidebar-btn").onclick = () =>
    els.sidebar.classList.toggle("collapsed");
  document.getElementById("new-chat-btn").onclick = () => startNewChat("general");
  document.getElementById("open-models-btn").onclick = openModelsModal;
  els.modelPillBtn.onclick = openModelsModal;
  document.getElementById("open-mcp-btn").onclick = openMcpModal;
  document.getElementById("open-customize-btn").onclick = openCustomizeModal;
  els.harnessPillBtn.onclick = openCustomizeModal;
  els.diffsPillBtn.onclick = openDiffsModal;
  document.getElementById("modal-close-btn").onclick = closeModal;

  els.workspacePillBtn.onclick = async () => {
    const picked = await invoke("pick_workspace_folder");
    if (picked) {
      appState.workspacePath = await invoke("set_workspace_folder", { path: picked });
      await refreshBootstrap();
      renderHeader();
    } else {
      openModal("Select Workspace Folder", (body) => {
        const div = document.createElement("div");
        div.className = "card";
        div.innerHTML = `
          <label>Enter folder path</label>
          <div class="row">
            <input id="ws-path-in" class="input grow mono" value="${escapeHtml(appState.workspacePath || "")}" placeholder="/path/to/project" />
            <button id="ws-set-btn" class="btn btn-primary btn-sm">Open folder</button>
          </div>
        `;
        body.appendChild(div);
        div.querySelector("#ws-set-btn").onclick = async () => {
          const path = div.querySelector("#ws-path-in").value;
          appState.workspacePath = await invoke("set_workspace_folder", { path });
          await refreshBootstrap();
          closeModal();
        };
      });
    }
  };

  document.getElementById("toggle-viewer-btn").onclick = async () => {
    appState.viewerOpen = !appState.viewerOpen;
    els.viewerPane.classList.toggle("hidden", !appState.viewerOpen);
    if (appState.viewerOpen) {
      renderViewerOptions();
      await loadSelectedViewerFile();
    }
  };
  document.getElementById("close-viewer-btn").onclick = () => {
    appState.viewerOpen = false;
    els.viewerPane.classList.add("hidden");
  };
  document.getElementById("refresh-viewer-btn").onclick = async () => {
    await refreshWorkspaceFiles();
    await loadSelectedViewerFile();
  };
  els.viewerSelect.onchange = async () => {
    appState.viewerFile = els.viewerSelect.value;
    await loadSelectedViewerFile();
  };

  els.sendBtn.onclick = sendCurrentPrompt;
  els.stopBtn.onclick = () => invoke("stop_generation");
  els.promptInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendCurrentPrompt();
    }
  });
}

init().catch((err) => {
  els.chatScroll.innerHTML = `<div class="empty-state"><h1>Keel Desktop</h1><p>${escapeHtml(String(err))}</p></div>`;
});
