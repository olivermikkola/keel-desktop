import { escapeHtml, parseCsv, parseStudyDeck, renderMarkdown } from "./markdown.js";

export function renderViewer(container, fileName, content) {
  if (!fileName) {
    container.innerHTML = `<div class="viewer-empty">No previewable file found in this workspace yet. Ask Keel to create one, or pick a file from the dropdown above.</div>`;
    return;
  }

  const lower = fileName.toLowerCase();
  if (lower.endsWith(".html") || lower.endsWith(".htm")) {
    container.innerHTML = "";
    const bar = document.createElement("div");
    bar.className = "viewer-toolbar";
    bar.innerHTML = `<span class="mono">${escapeHtml(fileName)}</span>`;
    const printBtn = document.createElement("button");
    printBtn.className = "btn btn-ghost btn-sm";
    printBtn.textContent = "Print / PDF";
    const frame = document.createElement("iframe");
    frame.className = "viewer-iframe";
    frame.setAttribute("sandbox", "allow-scripts allow-modals");
    frame.srcdoc = content;
    printBtn.addEventListener("click", () => {
      try {
        frame.contentWindow?.print();
      } catch {
        // Ignore sandbox print denial on unsupported platforms.
      }
    });
    bar.appendChild(printBtn);
    container.appendChild(bar);
    container.appendChild(frame);
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
