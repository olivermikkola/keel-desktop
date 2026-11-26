import { escapeHtml } from "./markdown.js";

/** Offline Arena compare panel — host chrome only; HTML comes from keel_arena view. */
export function createArenaCompare({ invoke, notice }) {
  const state = {
    busy: false,
    error: null,
    path: null,
    view: null,
    tab: "compare", // compare | dashboard | overview
  };
  let container;

  function contestantRows(contestants) {
    const entries = Object.entries(contestants || {});
    if (!entries.length) return '<p class="muted">No contestant rows in this manifest.</p>';
    return `<table class="arena-table"><thead><tr>
      <th>Contestant</th><th>Grade</th><th>Matched</th><th>Task-mean</th><th>Outcomes</th>
    </tr></thead><tbody>${entries
      .map(([name, row]) => {
        const outcomes = row?.outcomes || {};
        const matched = `${row?.matched_solved ?? "—"}/${row?.matched_attempts ?? "—"}`;
        const mean = row?.matched_task_mean_solve_rate;
        const meanLabel = typeof mean === "number" ? `${(mean * 100).toFixed(0)}%` : "—";
        const outcomeLabel = ["solved", "unsolved", "timeout", "infrastructure_error"]
          .map((key) => `${key[0]}:${outcomes[key] ?? 0}`)
          .join(" ");
        return `<tr>
          <td class="mono">${escapeHtml(name)}</td>
          <td>${escapeHtml(String(row?.grade ?? "—"))}</td>
          <td class="mono">${escapeHtml(matched)}</td>
          <td class="mono">${escapeHtml(meanLabel)}</td>
          <td class="mono muted">${escapeHtml(outcomeLabel)}</td>
        </tr>`;
      })
      .join("")}</tbody></table>`;
  }

  function frameHtml() {
    if (!state.view) return "";
    if (state.tab === "dashboard") return state.view.dashboardHtml || "";
    if (state.tab === "overview") {
      const svg = state.view.overviewSvg;
      if (!svg) return "<p class=\"muted\">No overview.svg in this Arena bundle.</p>";
      return `<!doctype html><html><head><meta charset="utf-8"><style>body{margin:0;padding:24px;background:#0a1112;display:flex;justify-content:center}svg,img{max-width:100%;height:auto}</style></head><body>${svg}</body></html>`;
    }
    return state.view.indexHtml || "";
  }

  function render() {
    if (!container) return;
    const view = state.view;
    const purpose = view?.purpose || "";
    const purposeClass = purpose === "benchmark" ? "arena-badge-bench" : "arena-badge-demo";
    container.innerHTML = `
      <div class="hub-heading">
        <span class="eyebrow">KEEL ARENA</span>
        <h1>Compare saved runs.</h1>
        <p>Pick a local Arena results JSON. Desktop asks Arena to validate, summarize, and write cards — then shows the offline compare HTML here. No model calls.</p>
      </div>
      <div class="arena-workspace">
        <section class="arena-controls card">
          <div class="row"><h2>Results file</h2><span class="capability-badge">Offline</span></div>
          <p class="muted">Use a public fixture (for example Arena’s <code>examples/synthetic-gauntlet-results.json</code>) or a private results file outside Git. Requires <code>python3 -m keel_arena</code> on PATH.</p>
          <p class="mono arena-path">${escapeHtml(state.path || "No file selected")}</p>
          <div class="row arena-actions">
            <button id="arena-pick" class="btn btn-sm" ${state.busy ? "disabled" : ""}>Choose JSON…</button>
            <button id="arena-open" class="btn btn-primary btn-sm" ${state.busy || !state.path ? "disabled" : ""}>${state.busy ? "Opening…" : "Open compare"}</button>
            ${view ? `<button id="arena-clear" class="btn btn-sm" ${state.busy ? "disabled" : ""}>Clear</button>` : ""}
          </div>
          ${state.error ? `<p class="arena-error">${escapeHtml(state.error)}</p>` : ""}
          ${view ? `
            <div class="arena-summary">
              <div class="row"><span class="arena-badge ${purposeClass}">${escapeHtml(purpose || "unknown")}</span>
              <span class="muted mono">${escapeHtml(view.sourceLabel || "")}</span></div>
              <p class="muted">Matched ${escapeHtml(String(view.matchedBlocks))} · Planned ${escapeHtml(String(view.plannedBlocks))} · Excluded ${escapeHtml(String(view.excludedBlocks))} · Tasks ${escapeHtml(String(view.independentTasks))}</p>
              <p class="muted mono" title="${escapeHtml(view.resultSha256 || "")}">result ${escapeHtml(digestPreview(view.resultSha256))}</p>
              <p class="muted mono" title="${escapeHtml(view.protocol?.harness_revision || "")}">harness ${escapeHtml(digestPreview(view.protocol?.harness_revision))}</p>
              <p class="muted mono" title="${escapeHtml(view.protocol?.environment_sha256 || "")}">env ${escapeHtml(digestPreview(view.protocol?.environment_sha256))}</p>
              <p class="muted mono" title="${escapeHtml(view.protocol?.sampling_sha256 || "")}">sampling ${escapeHtml(digestPreview(view.protocol?.sampling_sha256))}</p>
              ${contestantRows(view.contestants)}
            </div>` : ""}
        </section>
        <section class="arena-viewer card" aria-live="polite">
          ${view ? `
            <div class="arena-tabs" role="tablist">
              <button type="button" class="arena-tab ${state.tab === "compare" ? "active" : ""}" data-arena-tab="compare">${arenaTabLabel("compare")}</button>
              <button type="button" class="arena-tab ${state.tab === "dashboard" ? "active" : ""}" data-arena-tab="dashboard">${arenaTabLabel("dashboard")}</button>
              <button type="button" class="arena-tab ${state.tab === "overview" ? "active" : ""}" data-arena-tab="overview">${arenaTabLabel("overview")}</button>
            </div>
            <iframe class="arena-iframe" title="Arena compare viewer" sandbox="allow-scripts"></iframe>
          ` : `<div class="arena-empty"><p>Your compare view will appear here.</p><p class="muted">Host owns the file picker and tabs. Arena owns validation, metrics, and the HTML cards.</p></div>`}
        </section>
      </div>`;

    container.querySelector("#arena-pick").onclick = async () => {
      try {
        const picked = await invoke("pick_arena_results_file");
        if (!picked) return;
        state.path = picked;
        state.error = null;
        render();
      } catch (error) {
        state.error = String(error);
        notice(String(error));
        render();
      }
    };

    const openBtn = container.querySelector("#arena-open");
    if (openBtn) {
      openBtn.onclick = async () => {
        if (!state.path) return;
        state.busy = true;
        state.error = null;
        render();
        try {
          state.view = await invoke("open_arena_compare", { resultsPath: state.path });
          state.tab = "compare";
          notice(`Opened Arena compare (${state.view.purpose}).`);
        } catch (error) {
          state.view = null;
          state.error = String(error);
          notice(String(error));
        } finally {
          state.busy = false;
          render();
        }
      };
    }

    const clearBtn = container.querySelector("#arena-clear");
    if (clearBtn) {
      clearBtn.onclick = () => {
        state.view = null;
        state.error = null;
        state.tab = "compare";
        render();
      };
    }

    container.querySelectorAll("[data-arena-tab]").forEach((btn) => {
      btn.onclick = () => {
        state.tab = btn.getAttribute("data-arena-tab") || "compare";
        render();
      };
    });

    const frame = container.querySelector(".arena-iframe");
    if (frame) frame.srcdoc = frameHtml();
  }

  return {
    render(target) {
      container = target;
      render();
    },
  };
}

/** Pure helpers exported for unit tests. */
export function arenaTabLabel(tab) {
  if (tab === "dashboard") return "Dashboard";
  if (tab === "overview") return "Overview";
  return "Compare";
}

export function digestPreview(value, keep = 10) {
  const text = String(value || "");
  if (text.length <= keep + 7) return text || "—";
  return `${text.slice(0, keep)}…${text.slice(-6)}`;
}
