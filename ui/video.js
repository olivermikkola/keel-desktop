import { escapeHtml } from "./markdown.js";

export const VIDEO_ASPECT_RATIOS = [
  { id: "9:16", label: "9:16 Vertical (Reels / TikTok / Shorts)" },
  { id: "1:1", label: "1:1 Square" },
  { id: "16:9", label: "16:9 Landscape (YouTube)" },
];

export const DEFAULT_STORYBOARD_EXAMPLE = `image: assets/scene1.png
say: Private AI that runs right on your own computer.
overlay: PRIVATE LOCAL AI
motion: slow_zoom

image: assets/scene2.png
say: Fast, offline, and ready for real work.
overlay: ZERO CLOUD FEES
motion: pan_right`;

/** Index of the scene active at `time` seconds; last scene after the end. */
export function clipIndexAt(clips, time) {
  if (!Array.isArray(clips) || clips.length === 0) return 0;
  let found = 0;
  for (let i = 0; i < clips.length; i += 1) {
    if (Number(clips[i].start || 0) <= time + 1e-6) found = i;
  }
  return found;
}

/** Previous-scene jump target (restarts current scene after `restartWindow` seconds). */
export function previousSceneStart(clips, time, restartWindow = 1.0) {
  if (!Array.isArray(clips) || clips.length === 0) return 0;
  const idx = clipIndexAt(clips, time);
  const start = Number(clips[idx].start || 0);
  if (time - start > restartWindow || idx === 0) return start;
  return Number(clips[idx - 1].start || 0);
}

/** Next-scene start time, or null on the last scene. */
export function nextSceneStart(clips, time) {
  if (!Array.isArray(clips) || clips.length === 0) return null;
  const idx = clipIndexAt(clips, time);
  return idx + 1 < clips.length ? Number(clips[idx + 1].start || 0) : null;
}

/** Proportional timeline share (0..1) for each scene in a clip strip. */
export function clipFractions(clips) {
  if (!Array.isArray(clips) || clips.length === 0) return [];
  const total = clips.reduce((sum, clip) => sum + Math.max(0, Number(clip.duration || 0)), 0);
  if (total <= 0) return clips.map(() => 0);
  return clips.map((clip) => Math.max(0, Number(clip.duration || 0)) / total);
}

/** Serializes a finished job's plan clips back into editable storyboard text. */
export function planToStoryboardText(plan) {
  const clips = plan?.clips || [];
  return clips
    .map((clip) => {
      const lines = [`image: ${clip.image_path}`, `say: ${clip.narration}`];
      if (clip.overlay) lines.push(`overlay: ${clip.overlay}`);
      if (clip.motion && clip.motion !== "none") lines.push(`motion: ${clip.motion}`);
      return lines.join("\n");
    })
    .join("\n\n");
}

export function createVideoStudio({ invoke, getWorkspace, getWorkspaceFiles, selectFolder, openSettings, refresh, notice }) {
  const state = {
    jobs: [],
    ffmpeg: null,
    outputPath: "videos/keel-short.mp4",
    title: "",
    aspectRatio: "9:16",
    useTts: true,
    musicPath: "",
    scenes: DEFAULT_STORYBOARD_EXAMPLE,
    busy: false,
  };
  let requestVersion = 0;

  function renderInto(host) {
    if (!host) return;
    const workspace = getWorkspace();
    const files = (getWorkspaceFiles ? getWorkspaceFiles() : []).filter((f) =>
      /\.(png|jpe?g|webp|heic|mp4|mov|webm|mkv)$/i.test(f),
    );
    const running = state.jobs.find((job) => job.status === "running");

    host.innerHTML = `
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Short Video Studio</h2><span class="capability-badge">${state.ffmpeg ? "FFmpeg ready" : "Local FFmpeg"}</span></div>
          <p class="muted">Turn workspace images or screen clips into 9:16, 1:1, or 16:9 short videos with Ken Burns motion, safe-zone overlays, and local neural TTS (audio.cpp / Piper). Saves an editable <code>.storyboard.md</code> beside every <code>.mp4</code>.</p>
          <label for="video-path">Output file (.mp4 inside folder)</label>
          <input id="video-path" class="input mono" value="${escapeHtml(state.outputPath)}" placeholder="videos/keel-short.mp4" />
          <div class="image-options">
            <label>Aspect ratio<select id="video-aspect" class="input">${VIDEO_ASPECT_RATIOS.map((ar) => `<option value="${ar.id}" ${ar.id === state.aspectRatio ? "selected" : ""}>${escapeHtml(ar.label)}</option>`).join("")}</select></label>
            <label>Title (optional)<input id="video-title" class="input" value="${escapeHtml(state.title)}" placeholder="Product launch" /></label>
          </div>
          <label for="video-scenes">Storyboard scenes</label>
          <textarea id="video-scenes" class="input mono" rows="9" placeholder="image: assets/shot1.png&#10;say: Spoken narration line.&#10;overlay: ON-SCREEN HOOK&#10;motion: slow_zoom">${escapeHtml(state.scenes)}</textarea>
          ${files.length ? `<div class="row" style="gap:6px;flex-wrap:wrap"><span class="muted" style="font-size:12px">Insert workspace media:</span>${files.slice(0, 8).map((f) => `<button type="button" class="btn btn-sm btn-ghost" data-insert-media="${escapeHtml(f)}">+ ${escapeHtml(f)}</button>`).join("")}</div>` : ""}
          <label for="video-music">Background music (optional workspace .wav/.mp3)</label>
          <input id="video-music" class="input mono" value="${escapeHtml(state.musicPath)}" placeholder="audio/bed.wav" />
          <label class="row" style="gap:8px;align-items:center">
            <input type="checkbox" id="video-use-tts" ${state.useTts ? "checked" : ""} />
            <span>Narrate with local neural TTS (falls back to timed preview if no TTS runner is set)</span>
          </label>
          <div class="row" style="gap:8px">
            <button id="video-folder" class="btn btn-sm">${workspace ? "Change output folder" : "Choose output folder"}</button>
            ${openSettings ? '<button id="video-audio-settings" class="btn btn-sm btn-ghost">Configure TTS voice…</button>' : ""}
          </div>
          <p class="muted mono">${escapeHtml(workspace || "No folder selected")}</p>
          <button id="video-render" class="btn btn-primary" ${!workspace || running || state.busy || !state.scenes.trim() ? "disabled" : ""}>${running ? "Rendering…" : "Render short video"}</button>
        </section>
        <section class="image-results" aria-live="polite">
          <h2>Your short videos</h2>
          ${state.jobs.length ? state.jobs.map((job) => {
            const clips = job.plan?.clips || [];
            const fracs = clipFractions(clips);
            return `
            <article class="image-job card" data-video-job-card="${escapeHtml(job.id)}">
              <div class="row">
                <span class="capability-badge">${escapeHtml(job.status)}</span>
                <span class="muted">${job.plan ? `${job.plan.width} × ${job.plan.height} · ${clips.length} scenes · ${Number(job.plan.total_seconds || 0).toFixed(1)}s` : escapeHtml(job.request.path)}</span>
              </div>
              <p class="mono">${escapeHtml(job.artifact_path || job.request.path)}</p>
              ${job.status === "running" ? `<p class="muted">Rendering locally with FFmpeg. You can keep using Chat, Work, or Code.</p><button class="btn btn-sm" data-cancel-video="${escapeHtml(job.id)}">Cancel render</button>` : ""}
              ${job.error ? `<p class="muted">${escapeHtml(job.error)}</p>` : ""}
              ${job.artifact_path ? `
                <video class="generated-image" controls playsinline data-video-player="${escapeHtml(job.id)}" style="max-height:420px;background:#000"></video>
                ${clips.length ? `
                  <div class="row" style="gap:4px;margin-top:6px" data-scene-strip="${escapeHtml(job.id)}">
                    ${clips.map((c, idx) => `<button type="button" class="btn btn-sm btn-ghost" style="flex:${Math.max(0.15, fracs[idx] || 0.25)}" data-seek-video="${escapeHtml(job.id)}" data-seek-time="${Number(c.start || 0).toFixed(3)}" title="${escapeHtml(c.narration)}">S${idx + 1}</button>`).join("")}
                  </div>
                ` : ""}
                <div class="row" style="gap:8px;margin-top:8px">
                  <button class="btn btn-sm" data-load-storyboard="${escapeHtml(job.id)}">Edit storyboard</button>
                  <button class="btn btn-sm btn-ghost" data-copy-video-path="${escapeHtml(job.artifact_path)}">Copy video path</button>
                </div>
              ` : ""}
            </article>`;
          }).join("") : '<div class="image-empty"><p>Your short videos will appear here.</p><p class="muted">Pick a folder with images or clips, write a quick storyboard, and render a local MP4.</p></div>'}
        </section>
      </div>`;

    for (const [id, key] of [
      ["video-path", "outputPath"],
      ["video-aspect", "aspectRatio"],
      ["video-title", "title"],
      ["video-scenes", "scenes"],
      ["video-music", "musicPath"],
    ]) {
      const el = host.querySelector(`#${id}`);
      if (!el) continue;
      el.oninput = (event) => {
        state[key] = event.target.value;
        if (key === "scenes") {
          const btn = host.querySelector("#video-render");
          if (btn) btn.disabled = !workspace || running || state.busy || !state.scenes.trim();
        }
      };
    }
    const ttsBox = host.querySelector("#video-use-tts");
    if (ttsBox) {
      ttsBox.onchange = (event) => { state.useTts = Boolean(event.target.checked); };
    }
    host.querySelector("#video-folder").onclick = selectFolder;
    const audioBtn = host.querySelector("#video-audio-settings");
    if (audioBtn && openSettings) audioBtn.onclick = openSettings;

    host.querySelectorAll("[data-insert-media]").forEach((btn) => {
      btn.onclick = () => {
        const rel = btn.dataset.insertMedia;
        const snippet = `image: ${rel}\nsay: Describe what happens here.\nmotion: slow_zoom`;
        state.scenes = state.scenes.trim() ? `${state.scenes.trim()}\n\n${snippet}` : snippet;
        renderInto(host);
      };
    });

    host.querySelector("#video-render").onclick = async () => {
      state.busy = true;
      renderInto(host);
      try {
        const job = await invoke("start_video_job", {
          workspacePath: getWorkspace(),
          request: {
            path: state.outputPath.trim() || "videos/keel-short.mp4",
            scenes: state.scenes,
            aspect_ratio: state.aspectRatio,
            title: state.title.trim() || null,
            voice: null,
            use_tts: state.useTts,
            allow_silent_fallback: true,
            music_path: state.musicPath.trim() || null,
          },
        });
        await reload(host);
        notice(`Short video render started (${job.request.path}).`);
      } catch (error) {
        notice(String(error));
      } finally {
        state.busy = false;
        renderInto(host);
      }
    };

    host.querySelectorAll("[data-cancel-video]").forEach((btn) => {
      btn.onclick = async () => {
        try {
          await invoke("cancel_video_job", { id: btn.dataset.cancelVideo });
          await reload(host);
        } catch (error) {
          notice(String(error));
        }
      };
    });

    host.querySelectorAll("[data-load-storyboard]").forEach((btn) => {
      btn.onclick = () => {
        const job = state.jobs.find((j) => j.id === btn.dataset.loadStoryboard);
        if (!job) return;
        if (job.plan?.clips?.length) {
          state.scenes = planToStoryboardText(job.plan);
        } else if (job.request?.scenes) {
          state.scenes = job.request.scenes;
        }
        state.outputPath = job.request?.path || state.outputPath;
        state.aspectRatio = job.plan?.aspect_ratio || state.aspectRatio;
        state.title = job.plan?.title || "";
        renderInto(host);
        notice("Loaded storyboard into editor.");
      };
    });

    host.querySelectorAll("[data-copy-video-path]").forEach((btn) => {
      btn.onclick = async () => {
        try {
          await navigator.clipboard.writeText(btn.dataset.copyVideoPath);
          btn.textContent = "Copied";
          setTimeout(() => { if (btn.isConnected) btn.textContent = "Copy video path"; }, 1800);
        } catch {
          notice(`Video path: ${btn.dataset.copyVideoPath}`);
        }
      };
    });

    const escAttr = (val) => (typeof CSS !== "undefined" && CSS.escape ? CSS.escape(val) : String(val).replace(/["\\]/g, "\\$&"));
    host.querySelectorAll("[data-seek-video]").forEach((btn) => {
      btn.onclick = () => {
        const video = host.querySelector(`[data-video-player="${escAttr(btn.dataset.seekVideo)}"]`);
        if (video) {
          video.currentTime = Number(btn.dataset.seekTime || 0);
          video.play().catch(() => {});
        }
      };
    });

    const version = ++requestVersion;
    for (const job of state.jobs.filter((j) => j.artifact_path)) {
      invoke("read_workspace_video", { workspacePath: job.workspace_path, relPath: job.artifact_path })
        .then((src) => {
          if (version !== requestVersion) return;
          const video = host.querySelector(`[data-video-player="${escAttr(job.id)}"]`);
          if (video) video.src = src;
        })
        .catch(() => {});
    }
  }

  async function reload(host) {
    try {
      state.jobs = await invoke("list_video_jobs");
    } catch {
      state.jobs = [];
    }
    try {
      const detected = await invoke("detect_video_tools");
      state.ffmpeg = detected?.ffmpeg || null;
    } catch {
      state.ffmpeg = null;
    }
    if (host) renderInto(host);
  }

  return {
    renderInto,
    reload,
    async onJob(host) {
      await reload(host);
      if (refresh) await refresh();
    },
  };
}
