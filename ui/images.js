import { capabilityBadgeLabel } from "./audio.js";
import { createPhotoSlidesStudio } from "./carousels.js";
import { createDocumentCheckStudio } from "./decide.js";
import { escapeHtml } from "./markdown.js";
import { createVideoStudio } from "./video.js";

// No media weights or runtimes are installed until a user chooses to configure them.
// Audio is "available" for Settings packs + Try STT/TTS only — not a chat-mode audio product.
export const CAPABILITIES = [
  {
    id: "images",
    title: "Make Pictures",
    openLabel: "Open Pictures",
    detail: "Describe a picture in plain words and save it straight into your folder. Connects to an optional local picture engine on your computer.",
    state: "available",
  },
  {
    id: "video",
    title: "Short Videos",
    openLabel: "Open Short Videos",
    detail: "Turn your photos or clips into a short narrated video with gentle movement and on-screen captions.",
    state: "available",
  },
  {
    id: "carousels",
    title: "Photo Slides",
    openLabel: "Open Photo Slides",
    detail: "Turn your photos and one short sentence into swipeable photo slides (like multi-photo social posts) with headlines and captions.",
    state: "available",
  },
  {
    id: "decide",
    title: "Check Documents",
    openLabel: "Open Check Documents",
    detail: "Drop in bills, contracts, letters, or PDFs and ask the same questions across all of them — get a clear Yes/No, category, or rating table privately.",
    state: "available",
  },
  {
    id: "audio",
    title: "Voice & Speech",
    detail: "Turn voice recordings into text or turn written lines into spoken audio in Settings packs (Try STT / Try TTS). Not a chat-mode audio product yet.",
    state: "available",
  },
  {
    id: "text",
    title: "AI Assistant",
    detail: "Pick which private AI runs Chat, Work, and Code on this computer. Search the web only when you approve.",
    state: "available",
  },
];

export const EVERYDAY_APPS = [
  {
    id: "websites",
    title: "Web Pages",
    summary: "Build a simple web page and preview it right here — no coding needed.",
    example: "e.g. a café menu, event page, or personal site",
  },
  {
    id: "sheets",
    title: "Spreadsheets",
    summary: "Ask questions about a spreadsheet or build a new budget, tracker, or table.",
    example: "e.g. monthly budget or expense totals",
  },
  {
    id: "docs",
    title: "Documents",
    summary: "Write, organize, and revise letters, reports, or structured notes.",
    example: "e.g. a cover letter, brief, or weekly plan",
  },
  {
    id: "study",
    title: "Flashcards & Quizzes",
    summary: "Turn your notes or files into interactive flashcards and practice quizzes.",
    example: "e.g. study cards from class notes or phrases",
  },
  {
    id: "slides",
    title: "Presentations",
    summary: "Create a slide deck you can click through and save as a PDF.",
    example: "e.g. a 5-slide pitch or team update",
  },
];

export const IMAGE_STYLES = [
  { id: "", label: "Default" },
  { id: "photo", label: "Photo" },
  { id: "illustration", label: "Illustration" },
  { id: "animation", label: "Animation" },
  { id: "sketch", label: "Sketch" },
];

export function createImageWorkspace({
  invoke,
  getWorkspace,
  getWorkspaceFiles,
  getPresets,
  getEndpoint,
  getSdConfig,
  refresh,
  selectFolder,
  openModels,
  openPreset,
  openSettings,
  notice,
}) {
  const state = {
    activeTool: "images",
    backend: "server",
    endpoint: null,
    sdCliPath: "",
    sdModelPath: "",
    detected: null,
    checkpoints: [],
    jobs: [],
    busy: false,
    prompt: "",
    negative: "",
    checkpoint: "",
    style: "",
    size: "512",
    steps: "20",
  };
  const videoStudio = createVideoStudio({
    invoke,
    getWorkspace,
    getWorkspaceFiles,
    selectFolder,
    openSettings,
    refresh,
    notice,
  });
  const carouselsStudio = createPhotoSlidesStudio({
    invoke,
    getWorkspace,
    getWorkspaceFiles,
    selectFolder,
    refresh,
    notice,
  });
  const documentCheckStudio = createDocumentCheckStudio({
    invoke,
    getWorkspace,
    notice,
  });
  let container;
  let requestVersion = 0;

  function render() {
    if (!container || container.classList.contains("hidden")) return;
    const endpoint = state.endpoint || getEndpoint() || "http://127.0.0.1:7860";
    const sdCfg = getSdConfig ? getSdConfig() : null;
    const sdCliPath = state.sdCliPath || sdCfg?.sd_cli_path || state.detected?.sd_cli || "sd-cli";
    const sdModelPath = state.sdModelPath || sdCfg?.sd_model_path || "";
    const sdReady = Boolean(sdCfg?.sd_cli_path && sdCfg?.sd_model_path);
    const connected = state.backend === "stable_diffusion_cpp" ? sdReady : Boolean(getEndpoint());
    const workspace = getWorkspace();
    const running = state.jobs.find((job) => job.status === "running");
    const presetsById = new Map((getPresets ? getPresets() : []).map((p) => [p.id, p]));
    const everydayCards = EVERYDAY_APPS.map((item) => {
      const live = presetsById.get(item.id);
      return {
        id: item.id,
        title: item.title,
        summary: live?.summary && !/\b(SQL|CSV|Markdown|HTML)\b/i.test(live.summary) ? live.summary : item.summary,
        example: item.example,
      };
    });
    const workspaceTitle = CAPABILITIES.find((capability) => capability.id === state.activeTool).title;
    container.innerHTML = `
      <div class="hub-heading"><span class="eyebrow">APPS</span><h1>What do you want to make?</h1>
      <p>Pick an app below. Everything runs privately on your computer — no cloud account required.</p></div>
      <div class="apps-section-head"><h2>Create pictures, videos &amp; audio</h2><p class="muted">Optional creative tools that save straight to your folder.</p></div>
      <div class="capability-grid">${CAPABILITIES.map((cap) => `
        <article class="capability-card ${cap.id === state.activeTool ? "selected" : ""}">
          <div class="row"><h3>${escapeHtml(cap.title)}</h3><span class="capability-badge">${capabilityBadgeLabel(cap)}</span></div>
          <p>${escapeHtml(cap.detail)}</p>
          ${cap.openLabel ? `<button class="btn btn-sm" id="tool-${cap.id}-open" data-open-studio="${cap.id}">${escapeHtml(cap.openLabel)}</button>`
            : cap.id === "text" ? '<button class="btn btn-sm" id="tool-models">Choose AI model</button>'
            : cap.id === "audio" && openSettings ? '<button class="btn btn-sm" id="tool-audio-settings">Open audio settings</button>'
            : ""}
        </article>`).join("")}</div>
      <div class="apps-section-head"><h2>Write, plan &amp; study</h2><p class="muted">Each app opens a focused workspace ready for that job.</p></div>
      <div class="apps-preset-grid">${everydayCards.map((app) => `
        <article class="capability-card preset-app-card">
          <div class="row"><h3>${escapeHtml(app.title)}</h3><span class="capability-badge">Ready</span></div>
          <p>${escapeHtml(app.summary)}</p>
          <p class="muted app-example">${escapeHtml(app.example)}</p>
          <button class="btn btn-sm" data-open-preset="${escapeHtml(app.id)}">Start</button>
        </article>`).join("")}</div>
      <section id="app-workspace" tabindex="-1" aria-label="${escapeHtml(workspaceTitle)}">
      ${state.activeTool === "video" ? '<div id="video-studio-host"></div>' : state.activeTool === "carousels" ? '<div id="photo-slides-host"></div>' : state.activeTool === "decide" ? '<div id="document-check-host"></div>' : `
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Make Pictures</h2><span class="capability-badge">${state.backend === "stable_diffusion_cpp" ? "Direct file engine" : "Local picture app"}</span></div>
          <p class="muted">Type what you want to see and pick a style. Pictures are saved in your chosen folder.</p>
          <details class="advanced-disclosure" ${!connected ? "open" : ""}>
            <summary>${connected ? "Picture engine connected (settings)" : "Connect a local picture engine first"}</summary>
            <div class="advanced-disclosure-body">
              <label for="image-backend">How to run pictures on this computer</label>
              <select class="input" id="image-backend">
                <option value="server" ${state.backend === "server" ? "selected" : ""}>Local picture server (port 7860)</option>
                <option value="stable_diffusion_cpp" ${state.backend === "stable_diffusion_cpp" ? "selected" : ""}>Direct local engine (no background server)</option>
              </select>
              ${state.backend === "server" ? `
                <p class="muted">If you already run a local picture app on this computer with its API turned on, connect it below. Keel never downloads large picture models automatically.</p>
                <label for="image-endpoint">Local address</label>
                <div class="row"><input id="image-endpoint" class="input grow mono" value="${escapeHtml(endpoint)}" />
                <button id="image-connect" class="btn btn-sm" ${state.busy ? "disabled" : ""}>${state.busy ? "Connecting…" : "Connect"}</button>${Boolean(getEndpoint()) ? `<button id="image-disconnect" class="btn btn-ghost btn-sm" ${state.busy ? "disabled" : ""}>Disconnect</button>` : ""}</div>
                <label for="image-checkpoint">Picture model</label>
                <select class="input" id="image-checkpoint"><option value="">Use server’s current model</option>${state.checkpoints.map((name) => `<option ${name === state.checkpoint ? "selected" : ""} value="${escapeHtml(name)}">${escapeHtml(name)}</option>`).join("")}</select>
              ` : `
                <p class="muted">Point directly to a local image program and model file on this computer — no background server required.</p>
                <label for="sd-cli-path">Local image program path</label>
                <input id="sd-cli-path" class="input mono" value="${escapeHtml(sdCliPath)}" placeholder="sd-cli" />
                <label for="sd-model-path">Picture model file</label>
                <div class="row"><input id="sd-model-path" class="input grow mono" value="${escapeHtml(sdModelPath)}" placeholder="/path/to/picture-model" />
                <button id="sd-save" class="btn btn-sm" ${state.busy ? "disabled" : ""}>Save</button></div>
              `}
            </div>
          </details>
          <label for="image-prompt">Describe the picture you want</label>
          <textarea id="image-prompt" class="input" rows="4" placeholder="A quiet lakeside cabin in the evening, warm light, watercolor…">${escapeHtml(state.prompt)}</textarea>
          <label for="image-negative">Things to leave out (optional)</label><input id="image-negative" class="input" placeholder="e.g. blurry, text, extra fingers" value="${escapeHtml(state.negative)}" />
          <div class="image-options">
            <label>Style<select id="image-style" class="input">${IMAGE_STYLES.map((st) => `<option ${st.id === state.style ? "selected" : ""} value="${st.id}">${st.label}</option>`).join("")}</select></label>
            <label>Size<select id="image-size" class="input">${["512", "768", "1024"].map((size) => `<option ${size === state.size ? "selected" : ""} value="${size}">${size} × ${size}</option>`).join("")}</select></label>
            <label>Detail steps<input id="image-steps" class="input" type="number" min="1" max="50" value="${escapeHtml(state.steps)}" /></label>
          </div>
          <button id="image-folder" class="btn btn-sm">${workspace ? "Change save folder" : "Choose save folder"}</button>
          <p class="muted mono">${escapeHtml(workspace || "No folder selected yet")}</p>
          <button id="image-generate" class="btn btn-primary" ${!connected || !workspace || running || state.busy || !state.prompt.trim() ? "disabled" : ""}>${running ? "Making picture…" : "Make picture"}</button>
          <p class="muted">${!connected ? "Connect a local picture engine above to get started. " : ""}Saves one picture at a time to the <code>assets/</code> folder inside your workspace.</p>
        </section>
        <section class="image-results" aria-live="polite">
          <h2>Pictures you made</h2>
          ${state.jobs.length ? state.jobs.map((job) => `
            <article class="image-job card" data-job="${escapeHtml(job.id)}">
              <div class="row"><span class="capability-badge">${escapeHtml(job.status)}</span><span class="muted">${job.request.width} × ${job.request.height}${job.request.style ? ` · ${escapeHtml(job.request.style)}` : ""}</span></div>
              <p>${escapeHtml(job.request.prompt)}</p>
              ${job.status === "running" ? `<p class="muted">Drawing on your computer. You can keep using Chat, Work, or Code while it finishes.</p><button class="btn btn-sm" data-cancel-image="${escapeHtml(job.id)}">Stop waiting</button>` : ""}
              ${job.error ? `<p class="muted">${escapeHtml(job.error)}</p>` : ""}
              ${job.artifact_path ? `<img class="generated-image" alt="${escapeHtml(job.request.prompt)}" data-image-job="${escapeHtml(job.id)}" /><p class="muted mono">${escapeHtml(job.workspace_path)}/${escapeHtml(job.artifact_path)}</p><button class="btn btn-sm" data-copy-path="${escapeHtml(job.artifact_path)}">Copy asset path</button>` : ""}
            </article>`).join("") : '<div class="image-empty"><p>Your pictures will appear here.</p><p class="muted">Make a picture, then use it in a web page, presentation, or short video.</p></div>'}
        </section>
      </div>`}
      </section>`;

    container.querySelector("#tool-models").onclick = openModels;
    const audioSettingsBtn = container.querySelector("#tool-audio-settings");
    if (audioSettingsBtn && openSettings) audioSettingsBtn.onclick = openSettings;
    container.querySelectorAll("[data-open-preset]").forEach((btn) => {
      btn.onclick = () => {
        if (openPreset) openPreset(btn.dataset.openPreset);
      };
    });
    container.querySelectorAll("[data-open-studio]").forEach((button) => {
      button.onclick = () => openStudio(button.dataset.openStudio);
    });
    if (state.activeTool === "video") {
      const host = container.querySelector("#video-studio-host");
      videoStudio.renderInto(host);
      return;
    }
    if (state.activeTool === "carousels") {
      const host = container.querySelector("#photo-slides-host");
      carouselsStudio.renderInto(host);
      return;
    }
    if (state.activeTool === "decide") {
      const host = container.querySelector("#document-check-host");
      documentCheckStudio.renderInto(host);
      return;
    }
    container.querySelector("#image-backend").onchange = (event) => {
      state.backend = event.target.value;
      render();
    };
    const disconnectBtn = container.querySelector("#image-disconnect");
    if (disconnectBtn) {
      disconnectBtn.onclick = async () => {
        state.busy = true;
        render();
        try {
          await invoke("disconnect_image_server");
          state.checkpoints = [];
          state.checkpoint = "";
          await refresh();
          notice("Image server disconnected.");
        } catch (error) { notice(String(error)); }
        finally { state.busy = false; render(); }
      };
    }
    for (const [id, key] of [
      ["image-endpoint", "endpoint"],
      ["sd-cli-path", "sdCliPath"],
      ["sd-model-path", "sdModelPath"],
      ["image-prompt", "prompt"],
      ["image-negative", "negative"],
      ["image-style", "style"],
      ["image-size", "size"],
      ["image-steps", "steps"],
      ["image-checkpoint", "checkpoint"],
    ]) {
      const el = container.querySelector(`#${id}`);
      if (!el) continue;
      el.oninput = (event) => {
        state[key] = event.target.value;
        if (key === "prompt") {
          const genBtn = container.querySelector("#image-generate");
          if (genBtn) genBtn.disabled = !connected || !workspace || running || state.busy || !state.prompt.trim();
        }
      };
    }
    const connectBtn = container.querySelector("#image-connect");
    if (connectBtn) {
      connectBtn.onclick = async () => {
        state.busy = true;
        render();
        try {
          state.checkpoints = await invoke("connect_image_server", { endpoint: state.endpoint || endpoint });
          state.checkpoint = "";
          await refresh();
          notice("Image server connected. Choose a checkpoint and output folder.");
        } catch (error) { notice(String(error)); }
        finally { state.busy = false; render(); }
      };
    }
    const sdSaveBtn = container.querySelector("#sd-save");
    if (sdSaveBtn) {
      sdSaveBtn.onclick = async () => {
        state.busy = true;
        render();
        try {
          await invoke("configure_sd_cli_images", {
            sdCliPath: state.sdCliPath || sdCliPath,
            modelPath: state.sdModelPath || sdModelPath,
          });
          await refresh();
          notice("Local sd-cli image engine saved.");
        } catch (error) { notice(String(error)); }
        finally { state.busy = false; render(); }
      };
    }
    container.querySelector("#image-folder").onclick = selectFolder;
    container.querySelector("#image-generate").onclick = async () => {
      try {
        const request = {
          backend: state.backend,
          endpoint: state.backend === "server" ? getEndpoint() : "",
          prompt: state.prompt,
          negative_prompt: state.negative,
          checkpoint: state.checkpoint,
          style: state.style || null,
          sd_cli_path: state.backend === "stable_diffusion_cpp" ? (state.sdCliPath || sdCliPath) : null,
          model_path: state.backend === "stable_diffusion_cpp" ? (state.sdModelPath || sdModelPath) : null,
          width: Number(state.size),
          height: Number(state.size),
          steps: Number(state.steps),
        };
        const job = await invoke("start_image_job", { workspacePath: getWorkspace(), request });
        await reload();
        notice(`Image generation started (${job.request.width} × ${job.request.height}).`);
      } catch (error) { notice(String(error)); }
    };
    container.querySelectorAll("[data-cancel-image]").forEach((button) => {
      button.onclick = async () => {
        try { await invoke("cancel_image_job", { id: button.dataset.cancelImage }); await reload(); }
        catch (error) { notice(String(error)); }
      };
    });
    container.querySelectorAll("[data-copy-path]").forEach((button) => {
      button.onclick = async () => {
        try {
          await navigator.clipboard.writeText(button.dataset.copyPath);
          button.textContent = "Copied";
          setTimeout(() => { if (button.isConnected) button.textContent = "Copy asset path"; }, 1800);
        }
        catch { notice(`Asset path: ${button.dataset.copyPath}`); }
      };
    });
    const version = ++requestVersion;
    const escAttr = (val) => (typeof CSS !== "undefined" && CSS.escape ? CSS.escape(val) : String(val).replace(/["\\]/g, "\\$&"));
    for (const job of state.jobs.filter((j) => j.artifact_path)) {
      invoke("read_workspace_png", { workspacePath: job.workspace_path, relPath: job.artifact_path }).then((src) => {
        if (version !== requestVersion) return;
        const image = container.querySelector(`[data-image-job="${escAttr(job.id)}"]`);
        if (image) image.src = src;
      }).catch(() => {
        if (version !== requestVersion) return;
        const image = container.querySelector(`[data-image-job="${escAttr(job.id)}"]`);
        if (image) image.alt = "Image unavailable. Its workspace may have moved.";
      });
    }
  }

  async function openStudio(tool) {
    if (state.activeTool !== tool) {
      state.activeTool = tool;
      render();
    }
    const workspace = container.querySelector("#app-workspace");
    workspace.scrollIntoView({ block: "start" });
    workspace.focus({ preventScroll: true });

    if (tool === "video") {
      await videoStudio.reload(workspace.querySelector("#video-studio-host"));
    } else if (tool === "carousels") {
      await carouselsStudio.reload(workspace.querySelector("#photo-slides-host"));
    }
  }

  async function reload() {
    state.jobs = await invoke("list_image_jobs");
    try {
      state.detected = await invoke("detect_local_media_tools");
    } catch {
      // Optional in smoke fixtures.
    }
    render();
  }
  return {
    render(target) { container = target; render(); },
    isDocumentCheckOpen() { return state.activeTool === "decide"; },
    addDroppedDocuments: documentCheckStudio.addDroppedDocuments,
    reload,
    async onJob() { await reload(); await refresh(); },
    async onVideoJob() {
      const host = container?.querySelector("#video-studio-host");
      await videoStudio.onJob(host);
    },
  };
}
