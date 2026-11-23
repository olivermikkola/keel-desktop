export function escapeHtml(str = "") {
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function unescapeBasicEntities(str) {
  return str
    .replace(/&amp;/g, "&")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'");
}

function formatInline(text) {
  const tokens = [];
  const protect = (html) => { tokens.push(html); return `\u0000${tokens.length - 1}\u0000`; };
  let out = escapeHtml(text).replace(/`([^`]+)`/g, (_, code) => protect(`<code>${code}</code>`));
  out = out.replace(/\[([^\]]+)\]\(((?:[^\s()]|\([^\s()]*\))+)\)/g, (_, label, escapedUrl) => {
    const url = unescapeBasicEntities(escapedUrl);
    if (!/^https?:\/\//i.test(url)) return label;
    return protect(`<a href="${escapeHtml(url)}" data-external-link="${escapeHtml(url)}" rel="noopener noreferrer">${label}</a>`);
  });
  out = out.replace(/\*\*\*([^*]+)\*\*\*/g, "<strong><em>$1</em></strong>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>").replace(/__([^_]+)__/g, "<strong>$1</strong>");
  out = out.replace(/\*([^*]+)\*/g, "<em>$1</em>").replace(/~~([^~]+)~~/g, "<del>$1</del>");
  return out.replace(/\u0000(\d+)\u0000/g, (_, index) => tokens[Number(index)]);
}

export function renderMarkdown(raw = "") {
  const lines = String(raw).replace(/\r\n/g, "\n").split("\n");
  const out = [];
  let paragraph = [];
  let quote = [];
  let list = null;
  const flush = () => { if (paragraph.length) { out.push(`<p>${formatInline(paragraph.join("\n")).replace(/\n/g, "<br>")}</p>`); paragraph = []; } };
  const flushQuote = () => { if (quote.length) { out.push(`<blockquote>${quote.map((q) => formatInline(q)).join("<br>")}</blockquote>`); quote = []; } };
  const closeList = () => { if (list) { out.push(`</${list}>`); list = null; } };
  const cells = (line) => line.trim().replace(/^\||\|$/g, "").split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, "|"));
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i], trimmed = line.trim();
    const fence = trimmed.match(/^(`{3,}|~{3,})([\w+-]*)/);
    if (fence) {
      flush(); flushQuote(); closeList();
      const code = [];
      while (++i < lines.length && !lines[i].trim().startsWith(fence[1])) code.push(lines[i]);
      out.push(`<div class="code-block"><div class="code-block-head"><span>${escapeHtml(fence[2] || "Code")}</span><button class="text-action" data-copy-code>Copy code</button></div><pre><code>${escapeHtml(code.join("\n"))}</code></pre></div>`);
    } else if (trimmed.includes("|") && i + 1 < lines.length && /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(lines[i + 1])) {
      flush(); flushQuote(); closeList();
      const headers = cells(line); i += 1;
      const rows = [];
      while (i + 1 < lines.length && lines[i + 1].includes("|") && lines[i + 1].trim()) rows.push(cells(lines[++i]));
      out.push(`<div class="markdown-table-wrap"><table><thead><tr>${headers.map((c) => `<th>${formatInline(c)}</th>`).join("")}</tr></thead><tbody>${rows.map((row) => `<tr>${headers.map((_, j) => `<td>${formatInline(row[j] || "")}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`);
    } else if (!trimmed) { flush(); flushQuote(); closeList(); }
    else if (/^#{1,6}\s/.test(trimmed)) {
      flush(); flushQuote(); closeList(); const level = trimmed.indexOf(" "); out.push(`<h${level}>${formatInline(trimmed.slice(level + 1))}</h${level}>`);
    } else if (/^([-*_])(?:\s*\1){2,}\s*$/.test(trimmed)) { flush(); flushQuote(); closeList(); out.push("<hr>"); }
    else if (/^>\s?/.test(trimmed)) { flush(); closeList(); quote.push(trimmed.replace(/^>\s?/, "")); }
    else if (/^(?:[-*+]\s+|\d+[.)]\s+)/.test(trimmed)) {
      flush(); flushQuote(); const kind = /^\d/.test(trimmed) ? "ol" : "ul";
      if (list !== kind) { closeList(); list = kind; out.push(`<${kind}>`); }
      const itemBody = trimmed.replace(/^(?:[-*+]\s+|\d+[.)]\s+)/, "");
      const task = kind === "ul" ? itemBody.match(/^\[([ xX])\]\s+(.*)$/) : null;
      if (task) {
        const checked = task[1].toLowerCase() === "x";
        out.push(`<li class="md-task-item"><input type="checkbox" disabled ${checked ? "checked" : ""} /> ${formatInline(task[2])}</li>`);
      } else {
        out.push(`<li>${formatInline(itemBody)}</li>`);
      }
    } else { flushQuote(); closeList(); paragraph.push(line); }
  }
  flush(); flushQuote(); closeList(); return out.join("\n");
}

export function parseCsv(raw = "") {
  const text = String(raw).replace(/^\uFEFF/, "").replace(/\r\n/g, "\n");
  const rows = []; let row = [], field = "", quoted = false;
  const pushRow = () => { row.push(field); if (row.some((value) => value.trim())) rows.push(row); row = []; field = ""; };
  for (let i = 0; i < text.length; i += 1) {
    const character = text[i];
    if (character === '"') { if (quoted && text[i+1] === '"') { field += '"'; i += 1; } else quoted = !quoted; }
    else if (character === "," && !quoted) { row.push(field); field = ""; }
    else if (character === "\n" && !quoted) pushRow();
    else field += character;
  }
  if (field || row.length) pushRow();
  return rows;
}

export function parseStudyDeck(raw = "") {
  const lines = String(raw).replace(/\r\n/g, "\n").split("\n");
  let title = "Study Deck";
  const cards = [];
  let cur = null;

  const flush = () => {
    if (cur && cur.question) {
      cards.push(cur);
    }
    cur = null;
  };

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) {
      flush();
      continue;
    }
    if (line.startsWith("# ") && cards.length === 0 && !cur) {
      title = line.slice(2).trim();
      continue;
    }
    if (line.startsWith("Q:")) {
      flush();
      cur = { question: line.slice(2).trim(), choices: [], answer: "" };
    } else if (cur && line.startsWith("- ")) {
      cur.choices.push(line.slice(2).trim());
    } else if (cur && line.startsWith("A:")) {
      cur.answer = line.slice(2).trim();
    }
  }
  flush();
  return { title, cards };
}
