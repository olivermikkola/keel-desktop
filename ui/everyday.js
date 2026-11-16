import { escapeHtml, renderMarkdown } from "./markdown.js";
import { formatBytes } from "./state.js";

export function createEveryday(api) {
  let setupBusy = false;
  let library = [];
  let searchVersion = 0;
  const { invoke, state, refresh, render, modal, closeModal, toast } = api;
  const safe = escapeHtml;

  async function setup() {
    const plan = await invoke("get_setup_plan");
    modal("Make yourself at home", (body) => {
      body.innerHTML = `<div class="setup-intro"><div class="eyebrow">YOUR AI, ON YOUR COMPUTER</div><h2>Start a conversation.</h2><p>Keel can set up a local assistant for you. Your conversations stay on your computer unless you choose to connect an external service.</p></div>
      <div class="setup-recommendation"><strong>${safe(plan.recommended_model.name)}</strong><span class="capability-badge">Recommended for ${formatBytes(plan.memory_bytes)} RAM</span><p>${safe(plan.recommended_model.detail)}</p><p class="muted">Model: ${formatBytes(plan.recommended_model.bytes)}${plan.runtime_path ? " · Model runner already available" : ` · Runner: ${formatBytes(plan.runtime_asset?.bytes || 0)}`}. Download happens only when you choose setup.</p><button id="setup-start" class="btn btn-primary" ${setupBusy || (!plan.runtime_path && !plan.runtime_asset) ? "disabled" : ""}>${setupBusy ? "Setting up…" : "Download & set up local AI"}</button><p id="setup-progress" class="muted" role="status"></p><progress id="setup-download" class="setup-progress hidden"></progress></div>
      <details class="existing-server"><summary>I already use Ollama, LM Studio, or another server</summary><p class="muted">Use an existing model without downloading another copy. Your selected server handles inference.</p><label for="setup-url">Server address</label><input id="setup-url" class="input mono" value="http://127.0.0.1:11434/v1"/><label for="setup-model">Model name (optional)</label><input id="setup-model" class="input" placeholder="Use the server's first available model"/><button id="setup-connect" class="btn">Connect existing server</button></details>`;
      body.querySelector("#setup-start").onclick = async (event) => {
        if (setupBusy) return;
        setupBusy = true; event.target.disabled = true;
        body.querySelector("#setup-progress").textContent = "Getting your local assistant ready…";
        try { await invoke("setup_local_ai"); await refresh(); render(); closeModal(); toast("Your local assistant is ready. Ask your first question."); }
        catch (error) { body.querySelector("#setup-progress").textContent = String(error); }
        finally { setupBusy = false; event.target.disabled = false; }
      };
      body.querySelector("#setup-connect").onclick = async (event) => {
        event.target.disabled = true;
        try { await invoke("set_external_endpoint", { baseUrl: body.querySelector("#setup-url").value, modelName: body.querySelector("#setup-model").value || null }); await refresh(); render(); closeModal(); toast("Model server connected."); }
        catch (error) { body.querySelector("#setup-progress").textContent = String(error); }
        finally { event.target.disabled = false; }
      };
    });
  }

  function setupProgress(text) {
    const label = document.getElementById("setup-progress");
    if (label) label.textContent = text;
  }
  function downloadProgress(progress) {
    if (!setupBusy) return;
    const label = document.getElementById("setup-progress");
    const bar = document.getElementById("setup-download");
    if (label) label.textContent = `${progress.model_id === "runtime" ? "Model runner" : "Model"}: ${formatBytes(progress.downloaded_bytes)}${progress.total_bytes ? ` of ${formatBytes(progress.total_bytes)}` : ""}${progress.done ? " · Verified" : ""}`;
    if (bar) { bar.classList.remove("hidden"); if (progress.total_bytes) { bar.max = progress.total_bytes; bar.value = progress.downloaded_bytes; } else bar.removeAttribute("value"); }
  }

  async function editProject(id = null) {
    const project = id ? await invoke("get_project", { id }) : { id: crypto.randomUUID(), name: "", instructions: "", documents: [], workspace_path: null };
    modal(id ? "Project settings" : "A space for something you’re working on", (body) => {
      const draw = () => {
        body.innerHTML = `<label for="project-name">Project name</label><input id="project-name" class="input" maxlength="80" placeholder="A book, a business, a course…" value="${safe(project.name)}"/><label for="project-instructions">What should Keel know about this project?</label><textarea id="project-instructions" class="input" rows="5" maxlength="6000" placeholder="Goals, audience, writing style, and anything to keep in mind.">${safe(project.instructions)}</textarea><label>Reference documents</label><p class="muted">Available to every conversation in this project. Long documents use selected excerpts.</p><div class="source-list">${project.documents.map((doc) => `<div class="source-row"><span>${safe(doc.name)}</span><button class="text-action" data-remove-source="${safe(doc.id)}" aria-label="Remove ${safe(doc.name)}">Remove</button></div>`).join("")}</div><button id="project-add-docs" class="btn btn-sm">Add documents</button><label>Working folder (optional)</label><div class="row"><span class="muted mono grow">${safe(project.workspace_path || "Choose a folder when you want Keel to work on files")}</span><button id="project-folder" class="btn btn-sm">Choose folder</button></div><div class="row"><button id="project-save" class="btn btn-primary">Save project</button>${id ? '<button id="project-delete" class="btn btn-ghost">Remove project</button>' : ""}</div><p id="project-error" class="muted" role="status"></p>`;
        const read = () => { project.name = body.querySelector("#project-name").value; project.instructions = body.querySelector("#project-instructions").value; };
        body.querySelector("#project-add-docs").onclick = async () => { read(); try { project.documents.push(...await invoke("pick_documents")); draw(); } catch (error) { toast(String(error)); } };
        body.querySelectorAll("[data-remove-source]").forEach((button) => { button.onclick = () => { read(); project.documents = project.documents.filter((d) => d.id !== button.dataset.removeSource); draw(); }; });
        body.querySelector("#project-folder").onclick = async () => { read(); const folder = await invoke("pick_workspace_folder"); if (folder) project.workspace_path = folder; draw(); };
        body.querySelector("#project-save").onclick = async () => {
          read(); try { await invoke("save_project", { project }); await refresh(); render(); closeModal(); if (!id) await api.selectProject(project.id); }
          catch (error) { body.querySelector("#project-error").textContent = String(error); }
        };
        body.querySelector("#project-delete")?.addEventListener("click", () => {
          read(); body.querySelector("#project-error").innerHTML = 'Remove this project? Its conversations and files will be kept. <button id="confirm-project-delete" class="text-action">Remove project</button>';
          body.querySelector("#confirm-project-delete").onclick = async () => { await invoke("delete_project", { id }); if (state().activeProjectId === id) state().activeProjectId = null; await refresh(); render(); closeModal(); };
        });
      };
      draw();
    });
  }

  function preferences(initial = null) {
    modal("What should Keel remember?", (body) => {
      body.innerHTML = `<p>Save preferences or facts you want available across conversations. You control this text. Keel saves a preference only when you edit it here or explicitly ask “Remember that …”.</p><label for="personal-context">Saved preferences</label><textarea id="personal-context" class="input" rows="9" maxlength="4000" placeholder="I prefer concise answers. I’m learning Finnish. My meals should be vegetarian…">${safe(initial ?? state().bootstrap.config.personal_context ?? "")}</textarea><p class="muted">Saved on this computer. Included in future messages to the model server you select. Clear the text and save to forget it.</p><button id="memory-save" class="btn btn-primary">Save preferences</button>`;
      body.querySelector("#memory-save").onclick = async () => { await invoke("save_personal_context", { text: body.querySelector("#personal-context").value }); await refresh(); closeModal(); toast("Saved for future conversations."); };
    });
  }

  function threadSettings() {
    const thread = state().session; if (!thread || state().streaming) return;
    modal("Conversation", (body) => {
      body.innerHTML = `<label for="thread-title">Name</label><input id="thread-title" class="input" maxlength="120" value="${safe(thread.title)}"/><label for="thread-project">Project</label><select id="thread-project" class="input"><option value="">No project</option>${(state().bootstrap.projects || []).map((p) => `<option value="${safe(p.id)}" ${p.id === thread.project_id ? "selected" : ""}>${safe(p.name)}</option>`).join("")}</select><label class="row"><input id="thread-pinned" type="checkbox" ${thread.pinned ? "checked" : ""}/> Pin in recent conversations</label><div class="row"><button id="thread-save" class="btn btn-primary">Save changes</button><button id="thread-export" class="btn">Export Markdown</button></div>`;
      body.querySelector("#thread-save").onclick = async () => {
        state().session = await invoke("update_thread", { id: thread.id, title: body.querySelector("#thread-title").value, pinned: body.querySelector("#thread-pinned").checked, projectId: body.querySelector("#thread-project").value || null });
        state().activeProjectId = state().session.project_id; await refresh(); render(); closeModal();
      };
      body.querySelector("#thread-export").onclick = async () => { const path = await invoke("export_thread", { id: thread.id }); if (path) toast(`Saved to ${path}`); };
    });
  }

  async function search(query) {
    const version = ++searchVersion;
    const results = await invoke("search_threads", { query, projectId: state().activeProjectId });
    if (version !== searchVersion) return;
    state().searchResults = results; api.renderSidebar();
  }

  function previewSource(doc) {
    modal(doc.name, (body) => { body.innerHTML = `<p class="muted">${doc.text.length.toLocaleString()} characters extracted${doc.truncated ? " · Import limit reached; this is a partial document" : ""}. The assistant selects excerpts when the document is larger than its context window.</p><pre class="document-text">${safe(doc.text)}</pre>`; });
  }

  async function renderLibrary(container) {
    library = await invoke("list_library");
    const draw = (query = "") => {
      const filtered = library.filter((item) => `${item.name} ${item.kind}`.toLowerCase().includes(query.toLowerCase()) && (!state().activeProjectId || item.project_id === state().activeProjectId));
      container.innerHTML = `<div class="hub-heading"><span class="eyebrow">MADE WITH KEEL</span><h1>Your work, in one place.</h1><p>Approved documents, websites, code, and generated images. Your files stay in their original folders.</p></div><input id="library-search" class="input" placeholder="Find a document, image, or file…" aria-label="Search library" value="${safe(query)}"/><div class="library-grid">${filtered.map((item, index) => `<button class="library-card" data-library-index="${index}"><span class="capability-badge">${safe(item.kind.toUpperCase() || "FILE")}</span><strong>${safe(item.name)}</strong><span class="muted mono">${safe(item.workspace_path)}</span><span class="library-open">Open in Work ↗</span></button>`).join("") || '<div class="image-empty"><p>Your finished work will appear here.</p><p class="muted">Create a document in Work and approve its changes, or generate an image in Tools.</p></div>'}</div>`;
      container.querySelector("#library-search").oninput = (event) => { const position = event.target.selectionStart; draw(event.target.value); const input = container.querySelector("#library-search"); input.focus(); input.setSelectionRange(position, position); };
      container.querySelectorAll("[data-library-index]").forEach((button) => { button.onclick = () => api.openArtifact(filtered[Number(button.dataset.libraryIndex)]); });
    };
    draw();
  }

  return { setup, setupProgress, downloadProgress, editProject, preferences, threadSettings, search, previewSource, renderLibrary };
}
