import { escapeHtml, parseCsv, parseStudyDeck, renderMarkdown } from "./markdown.js";
import { parseDiffLines } from "./state.js";

export function renderDiffHtml(diffText = "") {
  const lines = parseDiffLines(diffText);
  if (lines.length === 0) {
    return `<div class="viewer-empty">No diff hunks available.</div>`;
  }
  return `<div class="diff-lines">${lines
    .map((line) => `<div class="diff-line diff-${line.kind}">${escapeHtml(line.text || " ")}</div>`)
    .join("")}</div>`;
}

export function renderCodeView(container, fileName, content) {
  if (!fileName) {
    container.innerHTML = `<div class="viewer-empty">Select a workspace file above to inspect its source.</div>`;
    return;
  }
  const rows = (content || "").split("\n");
  const linesHtml = rows
    .map(
      (line, idx) =>
        `<div class="code-line"><span class="code-ln">${idx + 1}</span><span class="code-src">${escapeHtml(line || " ")}</span></div>`,
    )
    .join("");
  container.innerHTML = `<div class="viewer-scroll code-viewer">${linesHtml}</div>`;
}

export function renderDiffView(container, edit) {
  if (!edit) {
    container.innerHTML = `<div class="viewer-empty">No staged edits in this session yet. Ask Keel to modify a file to review diffs here.</div>`;
    return;
  }
  container.innerHTML = `
    <div class="viewer-scroll diff-pane-wrap">
      <div class="diff-pane-head">
        <span class="mono">${escapeHtml(edit.summary)}</span>
        <span class="inline-edit-counts">
          ${edit.added_lines ? `<span class="diff-plus">+${edit.added_lines}</span>` : ""}
          ${edit.removed_lines ? `<span class="diff-minus">-${edit.removed_lines}</span>` : ""}
        </span>
      </div>
      ${renderDiffHtml(edit.diff)}
    </div>
  `;
}

const CODE_EXTENSIONS = new Set([
  "rs", "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "json", "toml",
  "yaml", "yml", "css", "scss", "sh", "bash", "zsh", "sql", "xml",
  "c", "cc", "cpp", "h", "hpp", "go", "java", "kt", "swift", "rb", "php",
]);

function injectPrintBridge(html) {
  const bridge =
    `<script>window.addEventListener("message",function(e){if(e&&e.data&&e.data.keelPrint){window.print();}});</script>`;
  if (/<\/body>/i.test(html)) {
    return html.replace(/<\/body>/i, `${bridge}</body>`);
  }
  return `${html}\n${bridge}`;
}

export function renderViewer(container, fileName, content) {
  if (!fileName) {
    container.innerHTML = `<div class="viewer-empty">No previewable file found in this workspace yet. Ask Keel to create one, or pick a file from the dropdown above.</div>`;
    return;
  }

  const lower = fileName.toLowerCase();
  const ext = lower.includes(".") ? lower.split(".").pop() : "";
  if (lower.endsWith(".html") || lower.endsWith(".htm")) {
    container.innerHTML = "";
    const bar = document.createElement("div");
    bar.className = "viewer-toolbar";
    bar.innerHTML = `<span class="mono grow">${escapeHtml(fileName)}</span>`;
    const downloadBtn = document.createElement("button");
    downloadBtn.className = "btn btn-ghost btn-sm";
    downloadBtn.type = "button";
    downloadBtn.textContent = "Export HTML";
    downloadBtn.addEventListener("click", () => {
      const blob = new Blob([content], { type: "text/html;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = fileName.split("/").pop() || "preview.html";
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    });
    const printBtn = document.createElement("button");
    printBtn.className = "btn btn-ghost btn-sm";
    printBtn.type = "button";
    printBtn.textContent = "Print / PDF";
    const frame = document.createElement("iframe");
    frame.className = "viewer-iframe";
    frame.setAttribute("sandbox", "allow-scripts allow-modals");
    frame.srcdoc = injectPrintBridge(content);
    printBtn.addEventListener("click", () => {
      try {
        frame.contentWindow?.postMessage({ keelPrint: true }, "*");
      } catch {
        // Fallback if iframe is not ready.
      }
    });
    bar.appendChild(downloadBtn);
    bar.appendChild(printBtn);
    container.appendChild(bar);
    container.appendChild(frame);
    return;
  }

  if (CODE_EXTENSIONS.has(ext)) {
    renderCodeView(container, fileName, content);
    return;
  }

  if (lower.endsWith(".csv")) {
    const rows = parseCsv(content);
    if (rows.length === 0) {
      container.innerHTML = `<div class="viewer-empty">CSV file is empty.</div>`;
      return;
    }
    const headers = rows[0];
    const body = rows.slice(1);
    const ths = headers.map((h) => `<th>${escapeHtml(h)}</th>`).join("");
    const trs = body
      .map(
        (row) =>
          `<tr>${headers.map((_, i) => `<td>${escapeHtml(row[i] ?? "")}</td>`).join("")}</tr>`,
      )
      .join("");
    container.innerHTML = `<div class="viewer-scroll"><table class="csv-table"><thead><tr>${ths}</tr></thead><tbody>${trs}</tbody></table></div>`;
    return;
  }

  const deck = parseStudyDeck(content);
  if (lower.endsWith(".md") && deck.cards.length > 0) {
    container.innerHTML = "";
    const wrap = document.createElement("div");
    wrap.className = "viewer-scroll study-deck";
    const heading = document.createElement("h2");
    heading.textContent = deck.title;
    wrap.appendChild(heading);

    deck.cards.forEach((card, idx) => {
      const el = document.createElement("div");
      el.className = "study-card";
      const q = document.createElement("div");
      q.className = "study-question";
      q.textContent = `${idx + 1}. ${card.question}`;
      el.appendChild(q);

      if (card.choices.length > 0) {
        const list = document.createElement("div");
        list.className = "study-choices";
        const feedback = document.createElement("div");
        feedback.className = "study-feedback";
        card.choices.forEach((choice) => {
          const btn = document.createElement("button");
          btn.className = "btn btn-ghost btn-sm study-choice";
          btn.textContent = choice;
          btn.addEventListener("click", () => {
            const correct = choice.toLowerCase() === card.answer.toLowerCase();
            feedback.textContent = correct
              ? `Correct: ${card.answer}`
              : `Answer: ${card.answer}`;
            feedback.dataset.state = correct ? "ok" : "warn";
          });
          list.appendChild(btn);
        });
        el.appendChild(list);
        el.appendChild(feedback);
      } else {
        const ans = document.createElement("div");
        ans.className = "study-answer hidden";
        ans.textContent = card.answer;
        const toggle = document.createElement("button");
        toggle.className = "btn btn-ghost btn-sm";
        toggle.textContent = "Reveal answer";
        toggle.addEventListener("click", () => {
          ans.classList.toggle("hidden");
          toggle.textContent = ans.classList.contains("hidden")
            ? "Reveal answer"
            : "Hide answer";
        });
        el.appendChild(toggle);
        el.appendChild(ans);
      }
      wrap.appendChild(el);
    });
    container.appendChild(wrap);
    return;
  }

  container.innerHTML = `<div class="viewer-scroll markdown-body">${renderMarkdown(content)}</div>`;
}

// Sandboxed srcdoc has no workspace URL. Embed referenced local PNGs so generated
// assets work in Websites and Slides while preserving the iframe sandbox.
export async function inlineWorkspaceImages(html, fileName, readPng) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const directory = fileName.split("/").slice(0, -1);
  await Promise.all([...doc.querySelectorAll("img[src]")].map(async (image) => {
    const src = image.getAttribute("src");
    if (!src || !/\.png$/i.test(src) || /^[a-z][a-z0-9+.-]*:|^[/\\]/i.test(src)) return;
    const parts = [...directory];
    for (const part of src.split("/")) {
      if (part === "..") { if (!parts.length) return; parts.pop(); }
      else if (part && part !== ".") parts.push(part);
    }
    try { image.setAttribute("src", await readPng(parts.join("/"))); }
    catch { /* Keep the original reference if an asset has moved or is unavailable. */ }
  }));
  return "<!doctype html>\n" + doc.documentElement.outerHTML;
}
