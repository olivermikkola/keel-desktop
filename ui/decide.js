import { escapeHtml } from "./markdown.js";

export const CHECK_STARTER_PACKS = [
  {
    id: "bills",
    label: "Bills & receipts",
    questions: [
      { id: "q-pay", kind: "yes_no", prompt: "Do I have to pay money because of this document?", options: [] },
      { id: "q-kind", kind: "choice", prompt: "What kind of document is this?", options: ["Bill", "Receipt", "Contract", "Letter", "Other"] },
      { id: "q-when", kind: "scale", prompt: "How soon must I act on it?", options: ["No action needed", "Some time later", "Within a few weeks", "Now — urgent or overdue"] },
    ],
  },
  {
    id: "contracts",
    label: "Contracts & leases",
    questions: [
      { id: "q-renew", kind: "yes_no", prompt: "Does this renew automatically if I do nothing?", options: [] },
      { id: "q-fee", kind: "yes_no", prompt: "Are there early cancellation fees or penalties?", options: [] },
      { id: "q-risk", kind: "scale", prompt: "How strict or risky are the terms?", options: ["Low", "Medium", "High"] },
    ],
  },
  {
    id: "applications",
    label: "Job applications",
    questions: [
      { id: "q-fit", kind: "yes_no", prompt: "Does the candidate meet the main requirements?", options: [] },
      { id: "q-level", kind: "choice", prompt: "What experience level fits best?", options: ["Entry", "Mid-level", "Senior"] },
      { id: "q-next", kind: "scale", prompt: "Recommend for an interview?", options: ["No", "Maybe", "Yes"] },
    ],
  },
];

export function formatConfidencePercent(confidence) {
  const pct = Math.round(Math.max(0, Math.min(1, Number(confidence || 0))) * 100);
  return `${pct}%`;
}

export function sanitizeCsvCell(text = "") {
  let clean = String(text || "").replace(/\r\n/g, " ").replace(/[\r\n]/g, " ");
  if (/^[=+\-@]/.test(clean)) clean = `'${clean}`;
  if (clean.includes(",") || clean.includes('"')) {
    return `"${clean.replace(/"/g, '""')}"`;
  }
  return clean;
}

export function resultsToCsv(questions = [], results = []) {
  const hasPartialImport = results.some((doc) => doc.truncated);
  const header = ["Document", ...questions.map((q) => q.prompt)];
  if (hasPartialImport) header.push("Source coverage");
  const rows = results.map((doc) => {
    const cells = questions.map((q) => {
      const ans = (doc.answers || []).find((a) => a.question_id === q.id);
      if (ans?.evidence) {
        const part = ans.total_parts > 1 && ans.part ? ` (part ${ans.part}/${ans.total_parts})` : "";
        const review = ans.unsure ? " [Review]" : "";
        return `${ans.answer} (${formatConfidencePercent(ans.confidence)})${review}${part} - ${ans.evidence}`;
      }
      if (ans) return `Unsure - ${ans.reason}`;
      if (doc.error) return `Error: ${doc.error}`;
      return "";
    });
    if (hasPartialImport) cells.push(doc.truncated ? "Partial import" : "");
    return [doc.document_name, ...cells].map(sanitizeCsvCell).join(",");
  });
  return [header.map(sanitizeCsvCell).join(","), ...rows].join("\n");
}

export function createDocumentCheckStudio({ invoke, getWorkspace, notice }) {
  const state = {
    documents: [], // { id, name, text, truncated? }
    questions: CHECK_STARTER_PACKS[0].questions.map((q) => ({ ...q, options: [...q.options] })),
    results: [],
    pasting: false,
    pasteText: "",
    newQPrompt: "",
    newQKind: "yes_no",
    newQOptions: "",
    runId: null,
    get busy() { return this.runId !== null; },
    stopping: false,
    importing: false,
  };

  let mountedHost;

  async function addDocuments(loadDocuments) {
    if (state.busy || state.importing) {
      notice("Wait for the current check or import before adding files.");
      return;
    }
    if (state.documents.length >= 50) {
      notice("Remove a document before adding more files.");
      return;
    }
    state.importing = true;
    if (mountedHost?.isConnected) renderInto(mountedHost);
    try {
      const documents = await loadDocuments();
      const added = (documents || []).slice(0, 50 - state.documents.length);
      if (added.length) {
        state.documents.push(...added.map((doc) => ({
          id: doc.id || crypto.randomUUID(), name: doc.name, text: doc.text, truncated: doc.truncated,
        })));
        state.results = [];
      }
    } catch (error) {
      notice(String(error));
    } finally {
      state.importing = false;
      if (mountedHost?.isConnected) renderInto(mountedHost);
    }
  }

  function renderInto(host) {
    if (!host) return;
    mountedHost = host;
    host.innerHTML = `
      <div class="image-workspace">
        <section class="image-controls card">
          <div class="row"><h2>Check Documents</h2><span class="capability-badge">Compare files</span></div>
          <p class="muted">Drop in bills, receipts, contracts, PDFs, or photos of letters. Ask the same questions about every file and review the answers alongside quotes from the files.</p>

          <label style="font-weight:600;color:var(--text)">1. Add files to check (${state.documents.length})</label>
          <div class="row" style="gap:8px;flex-wrap:wrap">
            <button type="button" id="check-pick-files" class="btn btn-sm" ${state.busy || state.importing || state.documents.length >= 50 ? "disabled" : ""}>Add PDFs, Word docs, or photos</button>
            <button type="button" id="check-toggle-paste" class="btn btn-sm btn-ghost" ${state.busy ? "disabled" : ""}>${state.pasting ? "Hide paste box" : "Paste text"}</button>
            ${state.documents.length ? `<button type="button" id="check-clear-docs" class="btn btn-sm btn-ghost" ${state.busy ? "disabled" : ""}>Clear all</button>` : ""}
          </div>
          ${state.pasting ? `
            <div class="video-scene-card">
              <textarea id="check-paste-text" class="input" rows="4" placeholder="Paste the text of a bill, email, or contract here…">${escapeHtml(state.pasteText)}</textarea>
              <button type="button" id="check-add-paste" class="btn btn-sm" ${state.busy || state.documents.length >= 50 ? "disabled" : ""}>Add pasted text</button>
            </div>
          ` : ""}
          ${state.documents.length ? `<div class="video-scene-list">${state.documents.map((doc, idx) => `
            <div class="row" style="justify-content:space-between;align-items:center;padding:6px 10px;border:1px solid var(--border);border-radius:8px">
              <span style="font-size:13px">${escapeHtml(doc.name)} <span class="muted">(${doc.text.trim().split(/\s+/).filter(Boolean).length} words)${doc.truncated ? " · Partial import" : ""}</span></span>
              <button type="button" class="btn btn-sm btn-ghost" data-remove-doc="${idx}" ${state.busy ? "disabled" : ""}>Remove</button>
            </div>`).join("")}</div>` : '<p class="muted">No files added yet.</p>'}

          <label style="font-weight:600;color:var(--text);margin-top:4px">2. What do you want to check? (${state.questions.length})</label>
          <div class="row" style="gap:6px;flex-wrap:wrap">
            <span class="muted" style="font-size:12px">Starter question packs:</span>
            ${CHECK_STARTER_PACKS.map((pack) => `<button type="button" class="btn btn-sm btn-ghost" data-pack="${pack.id}" ${state.busy ? "disabled" : ""}>${escapeHtml(pack.label)}</button>`).join("")}
          </div>
          <div class="video-scene-list">
            ${state.questions.map((q, idx) => `
              <div class="video-scene-card">
                <div class="row" style="justify-content:space-between;align-items:center">
                  <strong style="font-size:13px">${idx + 1}. ${escapeHtml(q.prompt)}</strong>
                  <button type="button" class="btn btn-sm btn-ghost" data-remove-q="${idx}" ${state.busy ? "disabled" : ""}>×</button>
                </div>
                <span class="muted" style="font-size:12px">${q.kind === "yes_no" ? "Answer: Yes or No" : q.kind === "choice" ? `Pick one: ${escapeHtml(q.options.join(", "))}` : `Rating: ${escapeHtml(q.options.join(" → "))}`}</span>
              </div>`).join("")}
          </div>

          <div class="video-scene-card">
            <strong style="font-size:12px">Add your own question</strong>
            <input id="check-new-prompt" class="input" value="${escapeHtml(state.newQPrompt)}" placeholder="e.g. Does this mention a late fee?" />
            <div class="image-options">
              <label>Answer type
                <select id="check-new-kind" class="input">
                  <option value="yes_no" ${state.newQKind === "yes_no" ? "selected" : ""}>Yes or No</option>
                  <option value="choice" ${state.newQKind === "choice" ? "selected" : ""}>Pick one category</option>
                  <option value="scale" ${state.newQKind === "scale" ? "selected" : ""}>Rating scale</option>
                </select>
              </label>
              ${state.newQKind !== "yes_no" ? `<label>Options (comma-separated)<input id="check-new-options" class="input" value="${escapeHtml(state.newQOptions)}" placeholder="Low, Medium, High" /></label>` : "<div></div>"}
            </div>
            <button type="button" id="check-add-q" class="btn btn-sm" ${state.busy || state.questions.length >= 10 || !state.newQPrompt.trim() ? "disabled" : ""}>+ Add question</button>
          </div>

          <button type="button" id="check-run" class="btn btn-primary" ${state.busy || state.importing || !state.documents.length || !state.questions.length ? "disabled" : ""}>${state.busy ? (state.stopping ? "Stopping…" : "Checking documents…") : `Check ${state.documents.length || ""} document${state.documents.length === 1 ? "" : "s"}`}</button>
          ${state.busy ? `<button type="button" id="check-stop" class="btn" ${state.stopping ? "disabled" : ""}>Stop check</button>` : ""}
        </section>

        <section class="image-results" aria-live="polite">
          <h2>Results</h2>
          ${state.results.length ? `
            <div class="row" style="gap:8px;margin-bottom:12px;flex-wrap:wrap">
              <button type="button" id="check-copy-csv" class="btn btn-sm">Copy table for spreadsheets</button>
              ${getWorkspace() ? '<button type="button" id="check-save-csv" class="btn btn-sm btn-ghost">Save spreadsheet to folder</button>' : ""}
            </div>
            <div class="video-scene-list">
              ${state.results.map((doc) => `
                <article class="card" style="display:flex;flex-direction:column;gap:8px">
                  <strong>${escapeHtml(doc.document_name)}</strong>
                  ${doc.truncated ? '<p class="muted">Only part of this document was imported. Answers use the extracted text.</p>' : ""}
                  ${doc.error ? `<p class="muted">${escapeHtml(doc.error)}</p>` : ""}
                  ${(doc.answers || []).map((ans) => {
                    const q = state.questions.find((item) => item.id === ans.question_id);
                    return `
                      <div style="border-top:1px solid var(--border);padding-top:8px">
                        <div class="row" style="justify-content:space-between;align-items:baseline;gap:8px">
                          <span class="muted" style="font-size:12px">${escapeHtml(q?.prompt || ans.question_id)}</span>
                          <span><strong>${escapeHtml(ans.answer)}</strong> ${ans.evidence ? `<span class="capability-badge" title="Model-reported confidence">${formatConfidencePercent(ans.confidence)}${ans.unsure ? " · Review" : ""}</span>` : ""}</span>
                        </div>
                        ${ans.evidence ? `
                          <p class="muted" style="margin:4px 0 0;font-size:12px">
                            Source${ans.part ? ` · Part ${ans.part} of ${ans.total_parts}` : ""} · “${escapeHtml(ans.evidence)}”
                          </p>` : ""}
                        ${ans.reason ? `<p class="muted" style="margin:4px 0 0;font-size:12px">${escapeHtml(ans.reason)}</p>` : ""}
                      </div>`;
                  }).join("")}
                </article>`).join("")}
            </div>
          ` : '<div class="image-empty"><p>Your answers table will appear here.</p><p class="muted">Add one or more files on the left, pick a starter question pack, and click Check documents. Review the source quotes to verify the answers.</p></div>'}
        </section>
      </div>`;

    const inputsChanged = () => {
      state.results = [];
      renderInto(host);
    };

    host.querySelector("#check-pick-files").onclick = () =>
      addDocuments(() => invoke("pick_documents"));

    host.querySelector("#check-toggle-paste").onclick = () => {
      state.pasting = !state.pasting;
      renderInto(host);
    };

    const clearDocs = host.querySelector("#check-clear-docs");
    if (clearDocs) {
      clearDocs.onclick = () => {
        state.documents = [];
        inputsChanged();
      };
    }

    const pasteBox = host.querySelector("#check-paste-text");
    if (pasteBox) pasteBox.oninput = (ev) => { state.pasteText = ev.target.value; };
    const addPaste = host.querySelector("#check-add-paste");
    if (addPaste) {
      addPaste.onclick = () => {
        const txt = state.pasteText.trim();
        if (!txt) return;
        const firstLine = txt.split("\n")[0].trim().slice(0, 40) || "Pasted note";
        state.documents.push({ id: crypto.randomUUID(), name: firstLine, text: txt });
        state.pasteText = "";
        state.pasting = false;
        inputsChanged();
      };
    }

    host.querySelectorAll("[data-remove-doc]").forEach((btn) => {
      btn.onclick = () => {
        state.documents.splice(Number(btn.dataset.removeDoc), 1);
        inputsChanged();
      };
    });

    host.querySelectorAll("[data-pack]").forEach((btn) => {
      btn.onclick = () => {
        const pack = CHECK_STARTER_PACKS.find((p) => p.id === btn.dataset.pack);
        if (!pack) return;
        state.questions = pack.questions.map((q) => ({ ...q, options: [...q.options] }));
        inputsChanged();
      };
    });

    host.querySelectorAll("[data-remove-q]").forEach((btn) => {
      btn.onclick = () => {
        state.questions.splice(Number(btn.dataset.removeQ), 1);
        inputsChanged();
      };
    });

    const newPrompt = host.querySelector("#check-new-prompt");
    if (newPrompt) {
      newPrompt.oninput = (ev) => {
        state.newQPrompt = ev.target.value;
        const btn = host.querySelector("#check-add-q");
        if (btn) btn.disabled = state.busy || state.questions.length >= 10 || !state.newQPrompt.trim();
      };
    }
    const newKind = host.querySelector("#check-new-kind");
    if (newKind) {
      newKind.onchange = (ev) => {
        state.newQKind = ev.target.value;
        renderInto(host);
      };
    }
    const newOpts = host.querySelector("#check-new-options");
    if (newOpts) newOpts.oninput = (ev) => { state.newQOptions = ev.target.value; };

    host.querySelector("#check-add-q").onclick = () => {
      const prompt = state.newQPrompt.trim();
      if (!prompt) return;
      const options = state.newQKind === "yes_no"
        ? []
        : state.newQOptions.split(",").map((s) => s.trim()).filter(Boolean);
      if (state.newQKind !== "yes_no" && options.length < 2) {
        notice("Enter at least two comma-separated options.");
        return;
      }
      state.questions.push({ id: `q-${Date.now()}`, kind: state.newQKind, prompt, options });
      state.newQPrompt = "";
      state.newQOptions = "";
      inputsChanged();
    };

    host.querySelector("#check-run").onclick = async () => {
      const runId = crypto.randomUUID();
      state.runId = runId;
      renderInto(host);
      try {
        const results = await invoke("run_document_check", {
          runId,
          documents: state.documents,
          questions: state.questions,
        });
        if (results === null) {
          notice("Document check stopped.");
          return;
        }
        state.results = results;
        notice(`Checked ${results.length} document${results.length === 1 ? "" : "s"}.`);
      } catch (err) {
        notice(String(err));
      } finally {
        state.runId = null;
        state.stopping = false;
        renderInto(mountedHost);
      }
    };

    const stopBtn = host.querySelector("#check-stop");
    if (stopBtn) {
      stopBtn.onclick = async () => {
        const runId = state.runId;
        state.stopping = true;
        renderInto(host);
        try {
          if (await invoke("stop_document_check", { runId })) return;
        } catch (err) {
          notice(String(err));
        }
        if (state.runId === runId) {
          state.stopping = false;
          renderInto(mountedHost);
        }
      };
    }

    const copyBtn = host.querySelector("#check-copy-csv");
    if (copyBtn) {
      copyBtn.onclick = async () => {
        try {
          await navigator.clipboard.writeText(resultsToCsv(state.questions, state.results));
          notice("Copied results as CSV.");
        } catch {
          notice("Could not access clipboard.");
        }
      };
    }

    const saveBtn = host.querySelector("#check-save-csv");
    if (saveBtn) {
      saveBtn.onclick = async () => {
        try {
          const savedPath = await invoke("export_document_check_csv", {
            workspacePath: getWorkspace(),
            questions: state.questions,
            results: state.results,
          });
          notice(`Saved spreadsheet to ${savedPath}.`);
        } catch (err) {
          notice(String(err));
        }
      };
    }
  }

  return {
    renderInto,
    addDroppedDocuments: (paths) => addDocuments(() => invoke("import_documents_from_paths", { paths })),
  };
}
