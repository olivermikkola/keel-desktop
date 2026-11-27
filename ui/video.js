import { escapeHtml } from "./markdown.js";

export const VIDEO_ASPECT_RATIOS = [
  { id: "9:16", label: "Vertical — Phone / Reels / TikTok (9:16)" },
  { id: "1:1", label: "Square — Social Feed (1:1)" },
  { id: "16:9", label: "Widescreen — YouTube / TV (16:9)" },
];

export const VIDEO_MOTIONS = [
  { id: "slow_zoom", label: "Gentle zoom in" },
  { id: "pan_left", label: "Slide left" },
  { id: "pan_right", label: "Slide right" },
  { id: "none", label: "Stay still" },
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

/** Parses storyboard text blocks into visual scene card objects. */
export function parseStoryboardBlocks(raw = "") {
  const normalized = String(raw || "").replace(/\r\n/g, "\n").trim();
  if (!normalized) return [];
  return normalized
    .split(/\n\s*\n/)
    .map((chunk) => {
      const scene = { image_path: "", narration: "", overlay: "", motion: "slow_zoom" };
      for (const line of chunk.split("\n")) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith("#")) continue;
        const colon = trimmed.indexOf(":");
        if (colon < 0) continue;
        const key = trimmed.slice(0, colon).trim().toLowerCase();
        const val = trimmed.slice(colon + 1).trim();
        if (["image", "photo", "picture", "clip", "media"].includes(key)) scene.image_path = val;
        else if (["say", "narration", "line", "text"].includes(key)) scene.narration = val;
        else if (["overlay", "caption", "title"].includes(key)) scene.overlay = val;
        else if (["motion", "camera", "kenburns"].includes(key)) scene.motion = val || "none";
      }
      return scene;
    })
    .filter((s) => s.image_path || s.narration || s.overlay);
}

/** Converts visual scene card objects back into storyboard block text. */
export function scenesToStoryboardText(cards = []) {
  return (cards || [])
    .filter((c) => (c.image_path || "").trim() || (c.narration || "").trim())
    .map((c) => {
      const lines = [
        `image: ${(c.image_path || "").trim()}`,
        `say: ${(c.narration || "").trim()}`,
      ];
      if ((c.overlay || "").trim()) lines.push(`overlay: ${c.overlay.trim()}`);
      if (c.motion && c.motion !== "none") lines.push(`motion: ${c.motion}`);
      return lines.join("\n");
    })
    .join("\n\n");
}

/** Moves a scene card earlier (-1) or later (+1) without mutating the original array. */
export function moveScene(cards, index, offset) {
  const target = index + offset;
  if (!Array.isArray(cards) || index < 0 || index >= cards.length || target < 0 || target >= cards.length || index === target) {
    return Array.isArray(cards) ? [...cards] : [];
  }
  const copy = [...cards];
  const [item] = copy.splice(index, 1);
  copy.splice(target, 0, item);
  return copy;
}

export function createVideoStudio({ invoke, getWorkspace, getWorkspaceFiles, selectFolder, openSettings, refresh, notice }) {
  const state = {
    jobs: [],
    ffmpeg: null,
    outputPath: "videos/keel-short.mp4",
    title: "",
    about: "",
    aspectRatio: "9:16",
    useTts: true,
    musicPath: "",
    scenes: DEFAULT_STORYBOARD_EXAMPLE,
    cards: parseStoryboardBlocks(DEFAULT_STORYBOARD_EXAMPLE),
    busy: false,
    writingScript: false,
    changeDrafts: {},
  };
  let requestVersion = 0;

  function syncTextFromCards() {
    state.scenes = scenesToStoryboardText(state.cards);
  }

  function syncCardsFromText() {
    const parsed = parseStoryboardBlocks(state.scenes);
    if (parsed.length) state.cards = parsed;
  }

  function renderInto(host) {
    if (!host) return;
    const workspace = getWorkspace();
    const allFiles = getWorkspaceFiles ? getWorkspaceFiles() : [];
    const mediaFiles = allFiles.filter((f) => /\.(png|jpe?g|webp|heic|mp4|mov|webm|mkv)$/i.test(f));
    const audioFiles = allFiles.filter((f) => /\.(wav|mp3|m4a|aac|flac|ogg)$/i.test(f));
    const running = state.jobs.find((job) => job.status === "running");

    host.innerHTML = `
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Short Videos</h2><span class="capability-badge">${state.ffmpeg ? "Ready" : "Needs video tool"}</span></div>
          <p class="muted">Turn photos or short clips into a narrated video with gentle movement, bold on-screen captions, and optional background music. Everything runs privately on your computer.</p>

          <label for="video-about">1. What is this video about? (optional — for AI line writing)</label>
          <textarea id="video-about" class="input" rows="2" placeholder="e.g. A funny 20-second video about our dog Ruben, or a quick tour of our new cafe">${escapeHtml(state.about)}</textarea>
          <div class="row" style="gap:8px;flex-wrap:wrap">
            <button type="button" id="video-write-ai" class="btn btn-sm" ${state.writingScript || state.busy || !state.about.trim() || state.cards.filter((c) => c.image_path.trim()).length === 0 ? "disabled" : ""}>${state.writingScript ? "Writing lines…" : "Write lines with AI"}</button>
            <span class="muted" style="font-size:12px">Writes one spoken sentence &amp; caption for each scene below.</span>
          </div>

          <div class="row" style="justify-content:space-between;align-items:center;margin-top:4px">
            <label style="margin:0;font-weight:600;color:var(--text)">2. Your scenes (${state.cards.length})</label>
            <button type="button" id="video-add-scene" class="btn btn-sm btn-ghost" ${state.cards.length >= 12 ? "disabled" : ""}>+ Add scene</button>
          </div>
          ${mediaFiles.length ? `<div class="row" style="gap:6px;flex-wrap:wrap"><span class="muted" style="font-size:12px">Quick-add from folder:</span>${mediaFiles.slice(0, 8).map((f) => `<button type="button" class="btn btn-sm btn-ghost" data-insert-media="${escapeHtml(f)}">+ ${escapeHtml(f)}</button>`).join("")}</div>` : ""}

          <div class="video-scene-list">
            ${state.cards.map((card, idx) => `
              <div class="video-scene-card" data-scene-idx="${idx}">
                <div class="row" style="justify-content:space-between;align-items:center">
                  <strong style="font-size:12px">Scene ${idx + 1}</strong>
                  <div class="row" style="gap:4px">
                    <button type="button" class="btn btn-sm btn-ghost" data-move-scene="${idx}" data-move-dir="-1" ${idx === 0 ? "disabled" : ""} title="Move earlier">↑</button>
                    <button type="button" class="btn btn-sm btn-ghost" data-move-scene="${idx}" data-move-dir="1" ${idx === state.cards.length - 1 ? "disabled" : ""} title="Move later">↓</button>
                    <button type="button" class="btn btn-sm btn-ghost" data-remove-scene="${idx}" ${state.cards.length <= 1 ? "disabled" : ""} title="Remove scene">×</button>
                  </div>
                </div>
                <label>Photo or clip in folder
                  <input class="input mono" data-scene-field="image_path" data-scene-idx="${idx}" value="${escapeHtml(card.image_path)}" placeholder="assets/photo1.png" list="video-media-list" />
                </label>
                <label>What the voice says
                  <input class="input" data-scene-field="narration" data-scene-idx="${idx}" value="${escapeHtml(card.narration)}" placeholder="One short, natural spoken sentence" />
                </label>
                <div class="image-options">
                  <label>On-screen text (optional)
                    <input class="input" data-scene-field="overlay" data-scene-idx="${idx}" value="${escapeHtml(card.overlay)}" placeholder="2–4 bold words" />
                  </label>
                  <label>Movement
                    <select class="input" data-scene-field="motion" data-scene-idx="${idx}">
                      ${VIDEO_MOTIONS.map((m) => `<option value="${m.id}" ${m.id === card.motion ? "selected" : ""}>${escapeHtml(m.label)}</option>`).join("")}
                    </select>
                  </label>
                </div>
              </div>
            `).join("")}
          </div>
          <datalist id="video-media-list">${mediaFiles.map((f) => `<option value="${escapeHtml(f)}"></option>`).join("")}</datalist>

          <div class="image-options">
            <label>Video shape<select id="video-aspect" class="input">${VIDEO_ASPECT_RATIOS.map((ar) => `<option value="${ar.id}" ${ar.id === state.aspectRatio ? "selected" : ""}>${escapeHtml(ar.label)}</option>`).join("")}</select></label>
            <label>Title (optional)<input id="video-title" class="input" value="${escapeHtml(state.title)}" placeholder="Weekend trip" /></label>
          </div>
          <label for="video-music">Background music (optional — plays quietly under the voice)</label>
          <input id="video-music" class="input mono" value="${escapeHtml(state.musicPath)}" placeholder="audio/music.mp3" list="video-audio-list" />
          <datalist id="video-audio-list">${audioFiles.map((f) => `<option value="${escapeHtml(f)}"></option>`).join("")}</datalist>
          <label class="row" style="gap:8px;align-items:center">
            <input type="checkbox" id="video-use-tts" ${state.useTts ? "checked" : ""} />
            <span>Read the lines aloud with a voice on this computer (makes a quiet timed video if no voice is set up yet)</span>
          </label>
          <details class="advanced-disclosure">
            <summary>Save file &amp; raw storyboard text (advanced)</summary>
            <div class="advanced-disclosure-body">
              <label for="video-path">Save file name inside folder (.mp4)</label>
              <input id="video-path" class="input mono" value="${escapeHtml(state.outputPath)}" placeholder="videos/keel-short.mp4" />
              <label for="video-scenes">Raw storyboard text</label>
              <textarea id="video-scenes" class="input mono" rows="7" placeholder="image: assets/shot1.png&#10;say: Spoken narration line.&#10;overlay: ON-SCREEN HOOK&#10;motion: slow_zoom">${escapeHtml(state.scenes)}</textarea>
            </div>
          </details>
          <div class="row" style="gap:8px">
            <button id="video-folder" class="btn btn-sm">${workspace ? "Change folder" : "Choose folder"}</button>
            ${openSettings ? '<button id="video-audio-settings" class="btn btn-sm btn-ghost">Voice settings…</button>' : ""}
          </div>
          <p class="muted mono">${escapeHtml(workspace || "No folder selected yet")}</p>
          <button id="video-render" class="btn btn-primary" ${!workspace || running || state.busy || !state.scenes.trim() ? "disabled" : ""}>${running ? "Making video…" : "Make short video"}</button>
        </section>
        <section class="image-results" aria-live="polite">
          <h2>Videos you made</h2>
          ${state.jobs.length ? state.jobs.map((job) => {
            const clips = job.plan?.clips || [];
            const fracs = clipFractions(clips);
            const draft = state.changeDrafts[job.id] || "";
            return `
            <article class="image-job card" data-video-job-card="${escapeHtml(job.id)}">
              <div class="row">
                <span class="capability-badge">${escapeHtml(job.status)}</span>
                <span class="muted">${job.plan ? `${job.plan.width} × ${job.plan.height} · ${clips.length} scenes · ${Number(job.plan.total_seconds || 0).toFixed(1)}s` : escapeHtml(job.request.path)}</span>
              </div>
              <p class="mono">${escapeHtml(job.artifact_path || job.request.path)}</p>
              ${job.status === "running" ? `<p class="muted">Putting your video together on this computer. You can keep using Chat, Work, or Code.</p><button class="btn btn-sm" data-cancel-video="${escapeHtml(job.id)}">Stop</button>` : ""}
              ${job.error ? `<p class="muted">${escapeHtml(job.error)}</p>` : ""}
              ${job.artifact_path ? `
                <video class="generated-image" controls playsinline data-video-player="${escapeHtml(job.id)}" style="max-height:420px;background:#000"></video>
                ${clips.length ? `
                  <div class="row" style="gap:4px;margin-top:6px" data-scene-strip="${escapeHtml(job.id)}">
                    ${clips.map((c, idx) => `<button type="button" class="btn btn-sm btn-ghost" style="flex:${Math.max(0.15, fracs[idx] || 0.25)}" data-seek-video="${escapeHtml(job.id)}" data-seek-time="${Number(c.start || 0).toFixed(3)}" title="${escapeHtml(c.narration)}">Scene ${idx + 1}</button>`).join("")}
                  </div>
                ` : ""}
                <div class="row" style="gap:6px;margin-top:8px">
                  <input class="input grow" data-change-input="${escapeHtml(job.id)}" value="${escapeHtml(draft)}" placeholder="Describe a change… e.g. Make it funnier or shorter" />
                  <button type="button" class="btn btn-sm" data-apply-change="${escapeHtml(job.id)}" ${state.writingScript || state.busy ? "disabled" : ""}>Rewrite</button>
                </div>
                <div class="row" style="gap:8px;margin-top:8px">
                  <button class="btn btn-sm" data-load-storyboard="${escapeHtml(job.id)}">Edit scenes</button>
                  <button class="btn btn-sm btn-ghost" data-copy-video-path="${escapeHtml(job.artifact_path)}">Copy video path</button>
                </div>
              ` : ""}
            </article>`;
          }).join("") : '<div class="image-empty"><p>Your videos will appear here.</p><p class="muted">Pick a folder with photos or clips, add your scenes on the left, and click Make short video.</p></div>'}
        </section>
      </div>`;

    const aboutEl = host.querySelector("#video-about");
    if (aboutEl) {
      aboutEl.oninput = (event) => {
        state.about = event.target.value;
        const btn = host.querySelector("#video-write-ai");
        if (btn) btn.disabled = state.writingScript || state.busy || !state.about.trim() || state.cards.filter((c) => c.image_path.trim()).length === 0;
      };
    }

    const writeBtn = host.querySelector("#video-write-ai");
    if (writeBtn) {
      writeBtn.onclick = async () => {
        const paths = state.cards.map((c) => c.image_path.trim()).filter(Boolean);
        if (!paths.length || !state.about.trim()) return;
        state.writingScript = true;
        renderInto(host);
        try {
          const generated = await invoke("generate_video_script", {
            description: state.about.trim(),
            mediaPaths: paths,
            previousLines: [],
            change: null,
          });
          state.cards = generated.map((s) => ({
            image_path: s.image_path,
            narration: s.narration,
            overlay: s.overlay || "",
            motion: s.motion || "slow_zoom",
          }));
          syncTextFromCards();
          notice("Wrote lines for your scenes. Review them or click Make short video.");
        } catch (error) {
          notice(String(error));
        } finally {
          state.writingScript = false;
          renderInto(host);
        }
      };
    }

    const addBtn = host.querySelector("#video-add-scene");
    if (addBtn) {
      addBtn.onclick = () => {
        if (state.cards.length >= 12) return;
        const unused = mediaFiles.find((f) => !state.cards.some((c) => c.image_path === f)) || mediaFiles[0] || "";
        state.cards.push({ image_path: unused, narration: "", overlay: "", motion: "slow_zoom" });
        syncTextFromCards();
        renderInto(host);
      };
    }

    host.querySelectorAll("[data-scene-field]").forEach((el) => {
      const idx = Number(el.dataset.sceneIdx);
      const field = el.dataset.sceneField;
      el.oninput = (event) => {
        if (!state.cards[idx]) return;
        state.cards[idx][field] = event.target.value;
        syncTextFromCards();
        const rawEl = host.querySelector("#video-scenes");
        if (rawEl) rawEl.value = state.scenes;
        const renderBtn = host.querySelector("#video-render");
        if (renderBtn) renderBtn.disabled = !workspace || running || state.busy || !state.scenes.trim();
        const aiBtn = host.querySelector("#video-write-ai");
        if (aiBtn) aiBtn.disabled = state.writingScript || state.busy || !state.about.trim() || state.cards.filter((c) => c.image_path.trim()).length === 0;
      };
    });

    host.querySelectorAll("[data-move-scene]").forEach((btn) => {
      btn.onclick = () => {
        state.cards = moveScene(state.cards, Number(btn.dataset.moveScene), Number(btn.dataset.moveDir));
        syncTextFromCards();
        renderInto(host);
      };
    });

    host.querySelectorAll("[data-remove-scene]").forEach((btn) => {
      btn.onclick = () => {
        if (state.cards.length <= 1) return;
        state.cards.splice(Number(btn.dataset.removeScene), 1);
        syncTextFromCards();
        renderInto(host);
      };
    });

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
          syncCardsFromText();
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
        if (state.cards.length < 12) {
          state.cards.push({ image_path: rel, narration: "Describe what happens here.", overlay: "", motion: "slow_zoom" });
          syncTextFromCards();
        }
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
        notice(`Started making your short video (${job.request.path}).`);
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

    host.querySelectorAll("[data-change-input]").forEach((input) => {
      input.oninput = (event) => {
        state.changeDrafts[input.dataset.changeInput] = event.target.value;
      };
    });

    host.querySelectorAll("[data-apply-change]").forEach((btn) => {
      btn.onclick = async () => {
        const jobId = btn.dataset.applyChange;
        const job = state.jobs.find((j) => j.id === jobId);
        const change = (state.changeDrafts[jobId] || "").trim();
        if (!job || !change) {
          notice("Type what you want to change first.");
          return;
        }
        const clips = job.plan?.clips || parseStoryboardBlocks(job.request?.scenes || "");
        if (!clips.length) return;
        state.writingScript = true;
        renderInto(host);
        try {
          const generated = await invoke("generate_video_script", {
            description: state.about.trim() || job.plan?.title || "Short video",
            mediaPaths: clips.map((c) => c.image_path),
            previousLines: clips.map((c) => ({ narration: c.narration, caption: c.overlay || null })),
            change,
          });
          state.cards = generated.map((s) => ({
            image_path: s.image_path,
            narration: s.narration,
            overlay: s.overlay || "",
            motion: s.motion || "slow_zoom",
          }));
          syncTextFromCards();
          state.changeDrafts[jobId] = "";
          notice("Updated your scene lines with that change. Click Make short video when ready.");
        } catch (error) {
          notice(String(error));
        } finally {
          state.writingScript = false;
          renderInto(host);
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
        syncCardsFromText();
        state.outputPath = job.request?.path || state.outputPath;
        state.aspectRatio = job.plan?.aspect_ratio || state.aspectRatio;
        state.title = job.plan?.title || "";
        renderInto(host);
        notice("Loaded scenes into the editor.");
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
