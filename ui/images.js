import { escapeHtml } from "./markdown.js";

// No media weights or runtimes are installed until a user chooses to configure them.
export const CAPABILITIES = [
  { id: "text", title: "Assistant", detail: "One text model for Chat, Work, and Code. Chat and Work can search the web with your approval; Chat can send image pixels for vision-capable models.", state: "available" },
  { id: "images", title: "Images", detail: "Generate images with an optional local AUTOMATIC1111 or Forge server.", state: "available" },
  { id: "audio", title: "Audio", detail: "Settings can mark packs installed and register a local STT/TTS runner; real Whisper adapters and downloads are still ahead.", state: "planned" },
  { id: "video", title: "Video", detail: "Video generation backends are planned.", state: "planned" },
];

export function createImageWorkspace({ invoke, getWorkspace, getEndpoint, refresh, selectFolder, openModels, notice }) {
  const state = { endpoint: null, checkpoints: [], jobs: [], busy: false, prompt: "", negative: "", checkpoint: "", size: "512", steps: "20" };
  let container;
  let requestVersion = 0;

  function render() {
    if (!container || container.classList.contains("hidden")) return;
    const endpoint = state.endpoint || getEndpoint() || "http://127.0.0.1:7860";
    const connected = Boolean(getEndpoint());
    const workspace = getWorkspace();
    const running = state.jobs.find((job) => job.status === "running");
    container.innerHTML = `
      <div class="hub-heading"><span class="eyebrow">YOUR AI TOOLBOX</span><h1>Use what you need.</h1>
      <p>Keep Keel light. Your assistant is ready for a text model; media tools are optional.</p></div>
      <div class="capability-grid">${CAPABILITIES.map((cap) => `
        <article class="capability-card ${cap.id === "images" ? "selected" : ""}">
          <div class="row"><h2>${cap.title}</h2><span class="capability-badge">${cap.state === "planned" ? "Planned" : cap.id === "images" ? "Optional" : "Built in"}</span></div>
          <p>${cap.detail}</p>${cap.id === "text" ? '<button class="btn btn-sm" id="tool-models">Choose text model</button>' : ""}
        </article>`).join("")}</div>
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Images</h2><span class="capability-badge">Local server</span></div>
          <p class="muted">Start AUTOMATIC1111 or Forge with <code>--api</code>, then connect here. Install image models in that server only if you want this tool. Keel does not download them automatically.</p>
          <label for="image-endpoint">Image server</label>
          <div class="row"><input id="image-endpoint" class="input grow mono" value="${escapeHtml(endpoint)}" />
          <button id="image-connect" class="btn btn-sm" ${state.busy ? "disabled" : ""}>${state.busy ? "Connecting…" : "Connect"}</button></div>
          <label for="image-checkpoint">Checkpoint</label>
          <select class="input" id="image-checkpoint"><option value="">Server's current model</option>${state.checkpoints.map((name) => `<option ${name === state.checkpoint ? "selected" : ""} value="${escapeHtml(name)}">${escapeHtml(name)}</option>`).join("")}</select>
          <label for="image-prompt">Describe your image</label>
          <textarea id="image-prompt" class="input" rows="4" placeholder="A quiet lakeside cabin in the evening, warm light, watercolor…">${escapeHtml(state.prompt)}</textarea>
          <label for="image-negative">Avoid (optional)</label><input id="image-negative" class="input" value="${escapeHtml(state.negative)}" />
          <div class="image-options"><label>Size<select id="image-size" class="input">${["512", "768", "1024"].map((size) => `<option ${size === state.size ? "selected" : ""} value="${size}">${size} × ${size}</option>`).join("")}</select></label>
          <label>Steps<input id="image-steps" class="input" type="number" min="1" max="50" value="${escapeHtml(state.steps)}" /></label></div>
          <button id="image-folder" class="btn btn-sm">${workspace ? "Change output folder" : "Choose output folder"}</button>
          <p class="muted mono">${escapeHtml(workspace || "No folder selected")}</p>
          <button id="image-generate" class="btn btn-primary" ${!connected || !workspace || running || state.busy || !state.prompt.trim() ? "disabled" : ""}>${running ? "Generating…" : "Generate image"}</button>
          <p class="muted">${!connected ? "Connect a server to get started. " : ""}One image at a time. PNGs save to assets/ in your folder. Larger sizes need more memory; choose a size supported by your checkpoint.</p>
        </section>
        <section class="image-results" aria-live="polite">
          <h2>Your generations</h2>
          ${state.jobs.length ? state.jobs.map((job) => `
            <article class="image-job card" data-job="${escapeHtml(job.id)}">
              <div class="row"><span class="capability-badge">${escapeHtml(job.status)}</span><span class="muted">${job.request.width} × ${job.request.height}</span></div>
              <p>${escapeHtml(job.request.prompt)}</p>
              ${job.status === "running" ? `<p class="muted">Waiting for the local server. You can keep using Chat, Work, or Code.</p><button class="btn btn-sm" data-cancel-image="${escapeHtml(job.id)}">Stop waiting</button>` : ""}
              ${job.error ? `<p class="muted">${escapeHtml(job.error)}</p>` : ""}
              ${job.artifact_path ? `<img class="generated-image" alt="${escapeHtml(job.request.prompt)}" data-image-job="${escapeHtml(job.id)}" /><p class="muted mono">${escapeHtml(job.workspace_path)}/${escapeHtml(job.artifact_path)}</p><button class="btn btn-sm" data-copy-path="${escapeHtml(job.artifact_path)}">Copy asset path</button>` : ""}
            </article>`).join("") : '<div class="image-empty"><p>Your images will appear here.</p><p class="muted">Generate a visual, then use its asset path in a website or slide deck in Work.</p></div>'}
        </section>
      </div>`;

    container.querySelector("#tool-models").onclick = openModels;
    for (const [id, key] of [["image-endpoint", "endpoint"], ["image-prompt", "prompt"], ["image-negative", "negative"], ["image-size", "size"], ["image-steps", "steps"], ["image-checkpoint", "checkpoint"]]) {
      container.querySelector(`#${id}`).oninput = (event) => {
        state[key] = event.target.value;
        if (key === "prompt") {
          const genBtn = container.querySelector("#image-generate");
          if (genBtn) genBtn.disabled = !connected || !workspace || running || state.busy || !state.prompt.trim();
        }
      };
    }
    container.querySelector("#image-connect").onclick = async () => {
      state.busy = true;
      render();
      try {
        state.checkpoints = await invoke("connect_image_server", { endpoint: state.endpoint || endpoint });
        // Reset a checkpoint from any previously connected server.
        state.checkpoint = "";
        await refresh();
        notice("Image server connected. Choose a checkpoint and output folder.");
      } catch (error) { notice(String(error)); }
      finally { state.busy = false; render(); }
    };
    container.querySelector("#image-folder").onclick = selectFolder;
    container.querySelector("#image-generate").onclick = async () => {
      try {
        const job = await invoke("start_image_job", { workspacePath: getWorkspace(), request: {
          endpoint: getEndpoint(), prompt: state.prompt, negative_prompt: state.negative,
          checkpoint: state.checkpoint, width: Number(state.size), height: Number(state.size), steps: Number(state.steps),
        } });
        // Fetch canonical state in case a very fast job already completed.
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
    render();
  }
  return {
    render(target) { container = target; render(); },
    reload,
    async onJob() { await reload(); await refresh(); },
  };
}
