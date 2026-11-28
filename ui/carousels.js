import { escapeHtml } from "./markdown.js";

/** Moves an item in an array earlier (-1) or later (+1) without mutating input. */
export function moveSlidePhoto(list, index, offset) {
  const target = index + offset;
  if (!Array.isArray(list) || index < 0 || index >= list.length || target < 0 || target >= list.length || index === target) {
    return Array.isArray(list) ? [...list] : [];
  }
  const copy = [...list];
  const [item] = copy.splice(index, 1);
  copy.splice(target, 0, item);
  return copy;
}

/** Suggests a clean filename inside `photo-slides/` from a short description. */
export function slugifySlideName(description = "") {
  const slug = String(description || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
  return `photo-slides/${slug || "my-photo-slides"}.html`;
}

/** Detects whether a filename looks like a phone/computer screenshot. */
export function isScreenshotPath(path = "") {
  return /screen[\s_-]?shot/i.test(String(path || ""));
}

/** Creates initial Cover + Photo + Closing slides for a list of photo entries. */
export function defaultCarouselSlides(photos = []) {
  const first = photos[0] || null;
  const last = photos[photos.length - 1] || null;
  const slides = [
    {
      kind: "cover",
      headline: "Your big headline",
      body: "One short sentence that makes people want to swipe.",
      photo_path: first?.path || null,
      style: first?.screenshot ? "screenshot" : "photo",
    },
  ];
  photos.forEach((p, idx) => {
    slides.push({
      kind: "photo",
      headline: `Point ${idx + 1}`,
      body: "One clear detail about this photo.",
      photo_path: p.path,
      style: p.screenshot ? "screenshot" : "photo",
    });
  });
  slides.push({
    kind: "closing",
    headline: "Wrap it up",
    body: "Give readers one takeaway or simple next step.",
    photo_path: last?.path || null,
    style: last?.screenshot ? "screenshot" : "photo",
  });
  return slides;
}

export function createPhotoSlidesStudio({ invoke, getWorkspace, getWorkspaceFiles, selectFolder, refresh, notice }) {
  const state = {
    decks: [],
    photos: [], // { path, screenshot }
    description: "",
    slides: defaultCarouselSlides([]),
    outputPath: "photo-slides/my-photo-slides.html",
    activePreviewHtml: "",
    activePreviewTitle: "",
    activePreviewPath: "",
    changeDraft: "",
    busy: false,
    writing: false,
  };

  function syncSlidesToPhotos(previousPhotos = state.photos) {
    const captions = state.slides.filter((slide) => slide.kind === "photo" && slide.photo_path);
    const captionsByPhoto = new Map(previousPhotos.map((photo, index) => [photo, captions[index]]));
    const cover = state.slides.find((slide) => slide.kind === "cover");
    const closing = state.slides.find((slide) => slide.kind === "closing");
    state.slides = defaultCarouselSlides(state.photos).map((slide, index) => {
      const previous = slide.kind === "cover" ? cover
        : slide.kind === "closing" ? closing
        : captionsByPhoto.get(state.photos[index - 1]);
      if (!previous) return slide;
      return { ...slide, headline: previous.headline, body: previous.body };
    });
  }

  function renderInto(host) {
    if (!host) return;
    const workspace = getWorkspace();
    const editingLocked = state.writing || state.busy;
    const folderPhotos = (getWorkspaceFiles ? getWorkspaceFiles() : []).filter((f) =>
      /\.(png|jpe?g|webp|gif)$/i.test(f),
    );
    const slideLabel = (s, idx) =>
      s.kind === "cover"
        ? "Cover slide (first card)"
        : s.kind === "closing"
          ? "Closing slide (last card)"
          : `Photo ${idx} slide`;

    host.innerHTML = `
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Photo Slides</h2><span class="capability-badge">Swipeable post</span></div>
          <p class="muted">Turn photos and a short sentence into ready-to-share swipeable photo slides (like the multi-photo posts you swipe through on social media). Saves an offline file you can swipe through or save as PDF.</p>

          <label style="font-weight:600;color:var(--text)">1. Pick your photos (${state.photos.length}/8)</label>
          ${folderPhotos.length ? `<div class="row" style="gap:6px;flex-wrap:wrap"><span class="muted" style="font-size:12px">Add from folder:</span>${folderPhotos.slice(0, 10).map((f) => `<button type="button" class="btn btn-sm btn-ghost" data-add-slide-photo="${escapeHtml(f)}" ${editingLocked || state.photos.length >= 8 ? "disabled" : ""}>+ ${escapeHtml(f)}</button>`).join("")}</div>` : '<p class="muted">Choose a folder that has PNG or JPG photos, or make pictures in Apps first.</p>'}
          ${state.photos.length ? `<div class="video-scene-list">${state.photos.map((p, idx) => `
            <div class="video-scene-card">
              <div class="row" style="justify-content:space-between;align-items:center">
                <span class="mono" style="font-size:12px">${idx + 1}. ${escapeHtml(p.path)}</span>
                <div class="row" style="gap:4px">
                  <button type="button" class="btn btn-sm btn-ghost" data-move-photo="${idx}" data-dir="-1" ${editingLocked || idx === 0 ? "disabled" : ""}>↑</button>
                  <button type="button" class="btn btn-sm btn-ghost" data-move-photo="${idx}" data-dir="1" ${editingLocked || idx === state.photos.length - 1 ? "disabled" : ""}>↓</button>
                  <button type="button" class="btn btn-sm btn-ghost" data-remove-photo="${idx}" ${editingLocked ? "disabled" : ""}>×</button>
                </div>
              </div>
              <label class="row" style="gap:6px;align-items:center;font-size:12px">
                <input type="checkbox" data-photo-screenshot="${idx}" ${editingLocked ? "disabled" : ""} ${p.screenshot ? "checked" : ""} />
                <span>Show inside a phone frame (keeps screenshot text readable)</span>
              </label>
            </div>`).join("")}</div>` : ""}

          <label for="slides-about">2. What are these slides about?</label>
          <textarea id="slides-about" class="input" rows="2" placeholder="e.g. 4 things we loved about our weekend trip to Porvoo">${escapeHtml(state.description)}</textarea>
          <div class="row" style="gap:8px;flex-wrap:wrap">
            <button type="button" id="slides-write-ai" class="btn btn-sm" ${state.writing || state.busy || !state.photos.length || !state.description.trim() ? "disabled" : ""}>${state.writing ? "Writing slides…" : "Write slide text with AI"}</button>
            <span class="muted" style="font-size:12px">Or edit the headlines and lines yourself below.</span>
          </div>

          <label style="font-weight:600;color:var(--text)">3. Slide headlines &amp; lines (${state.slides.length} slides)</label>
          <div class="video-scene-list">
            ${state.slides.map((slide, idx) => `
              <div class="video-scene-card">
                <strong style="font-size:12px">${slideLabel(slide, idx)}</strong>
                <input class="input" data-slide-head="${idx}" ${editingLocked ? "disabled" : ""} value="${escapeHtml(slide.headline)}" placeholder="Short bold headline" />
                <input class="input" data-slide-body="${idx}" ${editingLocked ? "disabled" : ""} value="${escapeHtml(slide.body)}" placeholder="One short sentence (optional)" />
              </div>`).join("")}
          </div>

          <div class="row" style="gap:8px">
            <button type="button" id="slides-folder" class="btn btn-sm">${workspace ? "Change folder" : "Choose folder"}</button>
          </div>
          <p class="muted mono">${escapeHtml(workspace || "No folder selected yet")}</p>
          <button type="button" id="slides-build" class="btn btn-primary" ${!workspace || state.busy || state.writing || !state.slides.length ? "disabled" : ""}>${state.busy ? "Building slides…" : "Build Photo Slides"}</button>
        </section>

        <section class="image-results" aria-live="polite">
          <h2>Preview &amp; saved decks</h2>
          ${state.activePreviewHtml ? `
            <article class="card" style="display:flex;flex-direction:column;gap:10px">
              <div class="row" style="justify-content:space-between;align-items:center">
                <strong>${escapeHtml(state.activePreviewTitle || "Photo Slides")}</strong>
                <span class="muted mono" style="font-size:12px">${escapeHtml(state.activePreviewPath)}</span>
              </div>
              <iframe id="slides-preview-frame" sandbox="allow-scripts allow-modals" style="width:100%;height:560px;border:1px solid var(--border);border-radius:12px;background:#091110"></iframe>
              <div class="row" style="gap:6px">
                <input id="slides-change-input" class="input grow" value="${escapeHtml(state.changeDraft)}" placeholder="Describe a change… e.g. Make headlines punchier or funnier" />
                <button type="button" id="slides-apply-change" class="btn btn-sm" ${state.writing || state.busy || !state.photos.length ? "disabled" : ""}>Rewrite</button>
              </div>
              <div class="row" style="gap:8px">
                <button type="button" id="slides-copy-path" class="btn btn-sm btn-ghost">Copy file path</button>
              </div>
            </article>
          ` : '<div class="image-empty"><p>Your swipeable photo slides will appear here.</p><p class="muted">Pick a few photos on the left, say what they are about, and click Build Photo Slides.</p></div>'}

          ${state.decks.length ? `
            <h3 style="margin:16px 0 8px;font-size:14px">Saved in this folder</h3>
            <div class="video-scene-list">
              ${state.decks.map((deck) => `
                <article class="video-scene-card">
                  <div class="row" style="justify-content:space-between;align-items:center">
                    <strong>${escapeHtml(deck.title)}</strong>
                    <span class="capability-badge">${deck.slide_count} slides</span>
                  </div>
                  <span class="muted mono" style="font-size:12px">${escapeHtml(deck.html_path)}</span>
                  <div class="row" style="gap:8px">
                    <button type="button" class="btn btn-sm" data-open-deck="${escapeHtml(deck.html_path)}">Preview</button>
                    <button type="button" class="btn btn-sm btn-ghost" data-edit-deck="${escapeHtml(deck.id)}" ${editingLocked ? "disabled" : ""}>Load into editor</button>
                  </div>
                </article>`).join("")}
            </div>
          ` : ""}
        </section>
      </div>`;

    const frame = host.querySelector("#slides-preview-frame");
    if (frame && state.activePreviewHtml) {
      frame.srcdoc = state.activePreviewHtml;
    }

    host.querySelector("#slides-folder").onclick = selectFolder;

    host.querySelectorAll("[data-add-slide-photo]").forEach((btn) => {
      btn.onclick = () => {
        if (state.photos.length >= 8) return;
        const path = btn.dataset.addSlidePhoto;
        const previousPhotos = state.photos;
        state.photos = [...previousPhotos, { path, screenshot: isScreenshotPath(path) }];
        syncSlidesToPhotos(previousPhotos);
        renderInto(host);
      };
    });

    host.querySelectorAll("[data-move-photo]").forEach((btn) => {
      btn.onclick = () => {
        const previousPhotos = state.photos;
        state.photos = moveSlidePhoto(previousPhotos, Number(btn.dataset.movePhoto), Number(btn.dataset.dir));
        syncSlidesToPhotos(previousPhotos);
        renderInto(host);
      };
    });

    host.querySelectorAll("[data-remove-photo]").forEach((btn) => {
      btn.onclick = () => {
        const previousPhotos = state.photos;
        state.photos = previousPhotos.filter((_, index) => index !== Number(btn.dataset.removePhoto));
        syncSlidesToPhotos(previousPhotos);
        renderInto(host);
      };
    });

    host.querySelectorAll("[data-photo-screenshot]").forEach((box) => {
      box.onchange = (ev) => {
        const idx = Number(box.dataset.photoScreenshot);
        if (state.photos[idx]) {
          state.photos[idx].screenshot = Boolean(ev.target.checked);
          syncSlidesToPhotos();
        }
      };
    });

    const aboutEl = host.querySelector("#slides-about");
    if (aboutEl) {
      aboutEl.oninput = (ev) => {
        state.description = ev.target.value;
        state.outputPath = slugifySlideName(state.description);
        const aiBtn = host.querySelector("#slides-write-ai");
        if (aiBtn) aiBtn.disabled = state.writing || state.busy || !state.photos.length || !state.description.trim();
      };
    }

    host.querySelectorAll("[data-slide-head]").forEach((el) => {
      el.oninput = (ev) => {
        const idx = Number(el.dataset.slideHead);
        if (state.slides[idx]) state.slides[idx].headline = ev.target.value;
      };
    });

    host.querySelectorAll("[data-slide-body]").forEach((el) => {
      el.oninput = (ev) => {
        const idx = Number(el.dataset.slideBody);
        if (state.slides[idx]) state.slides[idx].body = ev.target.value;
      };
    });

    const runWrite = async (change = null) => {
      if (!state.photos.length) return;
      state.writing = true;
      renderInto(host);
      try {
        const previous = change
          ? state.slides.map((s) => ({ kind: s.kind, headline: s.headline, body: s.body }))
          : [];
        const generated = await invoke("generate_photo_slides_text", {
          description: state.description.trim() || "Photo slides",
          photoPaths: state.photos.map((p) => p.path),
          previousSlides: previous,
          change,
        });
        const first = state.photos[0];
        const last = state.photos[state.photos.length - 1];
        state.slides = generated.map((g, idx) => {
          const p = g.kind === "cover" ? first : g.kind === "closing" ? last : state.photos[idx - 1];
          return {
            kind: g.kind,
            headline: g.headline,
            body: g.body,
            photo_path: p?.path || null,
            style: p?.screenshot ? "screenshot" : "photo",
          };
        });
        state.changeDraft = "";
        notice(change ? "Updated slide text with your change. Click Build Photo Slides to refresh." : "Wrote slide text. Review it or click Build Photo Slides.");
      } catch (err) {
        notice(String(err));
      } finally {
        state.writing = false;
        renderInto(host);
      }
    };

    const writeBtn = host.querySelector("#slides-write-ai");
    if (writeBtn) writeBtn.onclick = () => runWrite(null);

    const changeInput = host.querySelector("#slides-change-input");
    if (changeInput) changeInput.oninput = (ev) => { state.changeDraft = ev.target.value; };
    const applyChangeBtn = host.querySelector("#slides-apply-change");
    if (applyChangeBtn) {
      applyChangeBtn.onclick = () => {
        if (!state.changeDraft.trim()) {
          notice("Type what you want to change first.");
          return;
        }
        runWrite(state.changeDraft.trim());
      };
    }

    const copyBtn = host.querySelector("#slides-copy-path");
    if (copyBtn) {
      copyBtn.onclick = async () => {
        try {
          await navigator.clipboard.writeText(state.activePreviewPath);
          notice("Copied file path.");
        } catch {
          notice(state.activePreviewPath);
        }
      };
    }

    host.querySelector("#slides-build").onclick = async () => {
      state.busy = true;
      renderInto(host);
      try {
        const deck = await invoke("build_photo_slides", {
          workspacePath: getWorkspace(),
          request: {
            path: state.outputPath || slugifySlideName(state.description),
            description: state.description.trim() || "Photo Slides",
            slides: state.slides,
            accent: null,
          },
        });
        const html = await invoke("read_photo_slides_html", {
          workspacePath: getWorkspace(),
          relPath: deck.html_path,
        });
        state.activePreviewHtml = html;
        state.activePreviewTitle = deck.title;
        state.activePreviewPath = deck.html_path;
        await reload(host);
        if (refresh) await refresh();
        notice(`Saved Photo Slides to ${deck.html_path}.`);
      } catch (err) {
        notice(String(err));
      } finally {
        state.busy = false;
        renderInto(host);
      }
    };

    host.querySelectorAll("[data-open-deck]").forEach((btn) => {
      btn.onclick = async () => {
        const relPath = btn.dataset.openDeck;
        try {
          state.activePreviewHtml = await invoke("read_photo_slides_html", {
            workspacePath: getWorkspace(),
            relPath,
          });
          const found = state.decks.find((d) => d.html_path === relPath);
          state.activePreviewTitle = found?.title || relPath;
          state.activePreviewPath = relPath;
          renderInto(host);
        } catch (err) {
          notice(String(err));
        }
      };
    });

    host.querySelectorAll("[data-edit-deck]").forEach((btn) => {
      btn.onclick = () => {
        const deck = state.decks.find((d) => d.id === btn.dataset.editDeck);
        if (!deck) return;
        state.description = deck.description || "";
        state.outputPath = deck.html_path;
        state.slides = deck.slides.map((s) => ({ ...s }));
        state.photos = deck.slides
          .filter((s) => s.kind === "photo" && s.photo_path)
          .map((s) => ({ path: s.photo_path, screenshot: s.style === "screenshot" }));
        renderInto(host);
        notice("Loaded deck into editor.");
      };
    });
  }

  async function reload(host) {
    const ws = getWorkspace();
    if (!ws) {
      state.decks = [];
      if (host) renderInto(host);
      return;
    }
    try {
      state.decks = await invoke("list_photo_slides", { workspacePath: ws });
    } catch {
      state.decks = [];
    }
    if (host) renderInto(host);
  }

  return { renderInto, reload };
}
