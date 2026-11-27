import { capabilityBadgeLabel } from "./audio.js";
import { escapeHtml } from "./markdown.js";
import { createVideoStudio } from "./video.js";

// No media weights or runtimes are installed until a user chooses to configure them.
// Audio is "available" for Settings packs + Try STT/TTS only — not a chat-mode audio product.
export const CAPABILITIES = [
  { id: "text", title: "Assistant", detail: "One text model for Chat, Work, and Code. Search the web with your approval and send image pixels to vision-capable models.", state: "available" },
  { id: "images", title: "Images", detail: "Generate images with an optional local AUTOMATIC1111 or Forge server.", state: "available" },
  { id: "audio", title: "Audio", detail: "Settings packs: download checksum-pinned Whisper tiny q5_1 and Piper en_US kusal q8, register local / Whisper.cpp / Piper runners, Try STT (local file dialog) / Try TTS (short sample), and list artifacts. Not a chat-mode audio product yet.", state: "available" },
  { id: "video", title: "Short Video", detail: "Render 9:16, 1:1, or 16:9 short-form videos from workspace photos or clips with Ken Burns motion, safe-zone overlays, and local neural TTS via FFmpeg.", state: "available" },
];

export const IMAGE_STYLES = [
  { id: "", label: "Default" },
  { id: "photo", label: "Photo" },
  { id: "illustration", label: "Illustration" },
  { id: "animation", label: "Animation" },
  { id: "sketch", label: "Sketch" },
];

export function createImageWorkspace({ invoke, getWorkspace, getWorkspaceFiles, getEndpoint, getSdConfig, refresh, selectFolder, openModels, openSettings, notice }) {
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
    container.innerHTML = `
      <div class="hub-heading"><span class="eyebrow">YOUR AI TOOLBOX</span><h1>Use what you need.</h1>
      <p>Keep Keel light. Your assistant is ready for a text model; media tools are optional.</p></div>
      <div class="capability-grid">${CAPABILITIES.map((cap) => `
        <article class="capability-card ${cap.id === state.activeTool ? "selected" : ""}">
          <div class="row"><h2>${cap.title}</h2><span class="capability-badge">${capabilityBadgeLabel(cap)}</span></div>
          <p>${cap.detail}</p>${cap.id === "text" ? '<button class="btn btn-sm" id="tool-models">Choose text model</button>' : cap.id === "audio" && openSettings ? '<button class="btn btn-sm" id="tool-audio-settings">Configure audio in Settings</button>' : cap.id === "images" ? '<button class="btn btn-sm" id="tool-images-open">Open Images</button>' : cap.id === "video" ? '<button class="btn btn-sm" id="tool-video-open">Open Video Studio</button>' : ""}
        </article>`).join("")}</div>
      ${state.activeTool === "video" ? '<div id="video-studio-host"></div>' : `
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Images</h2><span class="capability-badge">${state.backend === "stable_diffusion_cpp" ? "Direct sd-cli" : "Local server"}</span></div>
          <label for="image-backend">Engine</label>
          <select class="input" id="image-backend">
            <option value="server" ${state.backend === "server" ? "selected" : ""}>AUTOMATIC1111 / Forge Server (loopback)</option>
            <option value="stable_diffusion_cpp" ${state.backend === "stable_diffusion_cpp" ? "selected" : ""}>stable-diffusion.cpp (direct local GGUF / Safetensors)</option>
          </select>
          ${state.backend === "server" ? `
            <p class="muted">Start AUTOMATIC1111 or Forge with <code>--api</code>, then connect here. Install image models in that server only if you want this tool. Keel does not download them automatically.</p>
            <label for="image-endpoint">Image server</label>
            <div class="row"><input id="image-endpoint" class="input grow mono" value="${escapeHtml(endpoint)}" />
            <button id="image-connect" class="btn btn-sm" ${state.busy ? "disabled" : ""}>${state.busy ? "Connecting…" : "Connect"}</button>${Boolean(getEndpoint()) ? `<button id="image-disconnect" class="btn btn-ghost btn-sm" ${state.busy ? "disabled" : ""}>Disconnect</button>` : ""}</div>
            <label for="image-checkpoint">Checkpoint</label>
            <select class="input" id="image-checkpoint"><option value="">Server's current model</option>${state.checkpoints.map((name) => `<option ${name === state.checkpoint ? "selected" : ""} value="${escapeHtml(name)}">${escapeHtml(name)}</option>`).join("")}</select>
          ` : `
            <p class="muted">Run <code>sd-cli</code> directly on Vulkan, Metal, CUDA, or CPU — no Python or web server required.</p>
            <label for="sd-cli-path">sd-cli binary</label>
            <input id="sd-cli-path" class="input mono" value="${escapeHtml(sdCliPath)}" placeholder="sd-cli" />
            <label for="sd-model-path">Model file (.gguf or .safetensors)</label>
            <div class="row"><input id="sd-model-path" class="input grow mono" value="${escapeHtml(sdModelPath)}" placeholder="/path/to/model.gguf" />
            <button id="sd-save" class="btn btn-sm" ${state.busy ? "disabled" : ""}>Save</button></div>
          `}
          <label for="image-prompt">Describe your image</label>
          <textarea id="image-prompt" class="input" rows="4" placeholder="A quiet lakeside cabin in the evening, warm light, watercolor…">${escapeHtml(state.prompt)}</textarea>
          <label for="image-negative">Avoid (optional)</label><input id="image-negative" class="input" value="${escapeHtml(state.negative)}" />
          <div class="image-options">
            <label>Style<select id="image-style" class="input">${IMAGE_STYLES.map((st) => `<option ${st.id === state.style ? "selected" : ""} value="${st.id}">${st.label}</option>`).join("")}</select></label>
            <label>Size<select id="image-size" class="input">${["512", "768", "1024"].map((size) => `<option ${size === state.size ? "selected" : ""} value="${size}">${size} × ${size}</option>`).join("")}</select></label>
            <label>Steps<input id="image-steps" class="input" type="number" min="1" max="50" value="${escapeHtml(state.steps)}" /></label>
          </div>
          <button id="image-folder" class="btn btn-sm">${workspace ? "Change output folder" : "Choose output folder"}</button>
          <p class="muted mono">${escapeHtml(workspace || "No folder selected")}</p>
          <button id="image-generate" class="btn btn-primary" ${!connected || !workspace || running || state.busy || !state.prompt.trim() ? "disabled" : ""}>${running ? "Generating…" : "Generate image"}</button>
          <p class="muted">${!connected ? "Connect a local engine to get started. " : ""}One image at a time. PNGs save to assets/ in your folder. Larger sizes need more memory; choose a size supported by your checkpoint.</p>
        </section>
        <section class="image-results" aria-live="polite">
          <h2>Your generations</h2>
          ${state.jobs.length ? state.jobs.map((job) => `
            <article class="image-job card" data-job="${escapeHtml(job.id)}">
              <div class="row"><span class="capability-badge">${escapeHtml(job.status)}</span><span class="muted">${job.request.width} × ${job.request.height}${job.request.style ? ` · ${escapeHtml(job.request.style)}` : ""}</span></div>
              <p>${escapeHtml(job.request.prompt)}</p>
              ${job.status === "running" ? `<p class="muted">Generating locally. You can keep using Chat, Work, or Code.</p><button class="btn btn-sm" data-cancel-image="${escapeHtml(job.id)}">Stop waiting</button>` : ""}
              ${job.error ? `<p class="muted">${escapeHtml(job.error)}</p>` : ""}
              ${job.artifact_path ? `<img class="generated-image" alt="${escapeHtml(job.request.prompt)}" data-image-job="${escapeHtml(job.id)}" /><p class="muted mono">${escapeHtml(job.workspace_path)}/${escapeHtml(job.artifact_path)}</p><button class="btn btn-sm" data-copy-path="${escapeHtml(job.artifact_path)}">Copy asset path</button>` : ""}
            </article>`).join("") : '<div class="image-empty"><p>Your images will appear here.</p><p class="muted">Generate a visual, then use its asset path in a website or slide deck in Work.</p></div>'}
        </section>
      </div>`}`;

    container.querySelector("#tool-models").onclick = openModels;
    const audioSettingsBtn = container.querySelector("#tool-audio-settings");
    if (audioSettingsBtn && openSettings) audioSettingsBtn.onclick = openSettings;
    const imagesOpenBtn = container.querySelector("#tool-images-open");
    if (imagesOpenBtn) {
      imagesOpenBtn.onclick = () => {
        if (state.activeTool !== "images") {
          state.activeTool = "images";
          render();
        }
      };
    }
    const videoOpenBtn = container.querySelector("#tool-video-open");
    if (videoOpenBtn) {
      videoOpenBtn.onclick = async () => {
        state.activeTool = "video";
        render();
        const host = container.querySelector("#video-studio-host");
        await videoStudio.reload(host);
      };
    }
    if (state.activeTool === "video") {
      const host = container.querySelector("#video-studio-host");
      videoStudio.renderInto(host);
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
    reload,
    async onJob() { await reload(); await refresh(); },
    async onVideoJob() {
      const host = container?.querySelector("#video-studio-host");
      await videoStudio.onJob(host);
    },
  };
}
