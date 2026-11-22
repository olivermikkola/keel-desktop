// First-run welcome and guided setup. One screen, one button: it turns into a live
// progress view while the assistant downloads, and never blocks the rest of the app.
import { escapeHtml } from "./markdown.js";
import { formatBytes } from "./state.js";
import { createProgressTracker, formatEta, formatSpeed, friendlyRuntimeError, friendlySetupError, serverDataNotice, stepStates } from "./onboarding.js";

export function createWelcome(api) {
  const { invoke, root, toast } = api;
  const body = root.querySelector("#welcome-body");
  let view = "intro";
  let plan = null;
  let choice = "recommended";
  try { if (localStorage.getItem("keel_setup_choice") === "fast") choice = "fast"; } catch { /* Storage is optional. */ }
  let stage = "runner";
  let busy = false;
  let error = null;
  let tracker = createProgressTracker();
  let lastChange = 0;

  const chosenModel = () => (choice === "fast" && plan?.fast_model ? plan.fast_model : plan?.recommended_model);
  const runnerBytes = () => (plan?.runtime_path ? 0 : plan?.runtime_asset?.bytes || 0);
  const modelInstalled = () => (api.bootstrap()?.models || []).some((m) => m.id === chosenModel()?.id && m.installed);
  const totalBytes = () => runnerBytes() + (modelInstalled() ? 0 : chosenModel()?.bytes || 0);
  const canAutoSetup = () => Boolean(plan && (plan.runtime_path || plan.runtime_asset));
  const hasPartial = () => plan?.partial_runner || plan?.partial_model_ids?.includes(chosenModel()?.id);
  const snapshot = () => tracker.snapshot();

  function meterText() {
    const s = snapshot();
    if (stage === "start") return "Waking it up. The first start takes a few seconds.";
    if (!s.total) return "Starting the download…";
    const parts = [`${formatBytes(s.downloaded)} of ${formatBytes(s.total)}`];
    const speed = formatSpeed(s.bytesPerSecond);
    if (speed) parts.push(speed);
    const eta = s.etaSeconds === null ? "" : formatEta(s.etaSeconds);
    if (eta) parts.push(`${eta} left`);
    return parts.join(" · ");
  }

  function paint() {
    if (view !== "working") return;
    const s = snapshot();
    const fraction = stage === "start" ? 1 : s.fraction;
    const bar = body.querySelector("#welcome-bar");
    if (bar) { if (stage === "start") bar.removeAttribute("value"); else bar.value = fraction; }
    const pct = body.querySelector("#welcome-pct");
    if (pct) pct.textContent = stage === "start" ? "Almost there" : `${Math.round(fraction * 100)}%`;
    const meter = body.querySelector("#welcome-meter");
    if (meter) meter.textContent = meterText();
  }

  function render() {
    if (view === "working") renderWorking();
    else if (view === "error") renderError();
    else if (view === "paused") renderPaused();
    else if (view === "existing") renderExisting();
    else renderIntro();
    const primary = body.querySelector("[data-primary]");
    if (primary && !root.classList.contains("hidden")) primary.focus({ preventScroll: true });
  }

  function renderIntro() {
    const model = chosenModel();
    const size = model ? formatBytes(runnerBytes() + (modelInstalled() ? 0 : model.bytes)) : "";
    const smaller = plan?.fast_model;
    const sizeLine = size ? `One download of about ${size}, then it works without internet.` : "One download, then it works without internet.";
    const sizeToggle = smaller
      ? `<button id="welcome-size" class="text-action" type="button">${choice === "fast" ? `Use the smarter one instead (${formatBytes(plan.recommended_model.bytes)})` : `Want it faster? Get the lighter one (${formatBytes(smaller.bytes)})`}</button>`
      : "";
    body.innerHTML = `
      <div class="welcome-eyebrow">KEEL</div>
      <h1 class="welcome-headline">Your own private AI.</h1>
      <p class="welcome-sub">Ask it anything, write with it, build things. It runs on this computer, so there is no account, no subscription, and nothing you type leaves your machine.</p>
      <div class="welcome-desktop-actions">
        <button id="welcome-start" data-primary class="welcome-btn-white" ${canAutoSetup() ? "" : "disabled"}>${hasPartial() ? "Resume setup" : "Set up my AI"}</button>
        <button id="welcome-skip" class="btn btn-ghost btn-sm" type="button">Not now</button>
      </div>
      <div class="welcome-desktop-foot">${canAutoSetup() ? escapeHtml(sizeLine) : "Automatic setup isn't available on this computer yet. You can still connect an AI you already run."}</div>
      <div class="welcome-links">${sizeToggle}<button id="welcome-existing" class="text-action" type="button">I already use Ollama or LM Studio</button></div>`;
    body.querySelector("#welcome-start").onclick = start;
    body.querySelector("#welcome-skip").onclick = skip;
    body.querySelector("#welcome-existing").onclick = () => { view = "existing"; render(); };
    const toggle = body.querySelector("#welcome-size");
    if (toggle) toggle.onclick = () => { choice = choice === "fast" ? "recommended" : "fast"; render(); };
  }

  function renderWorking() {
    const steps = stepStates(stage).map((s) => `<li class="welcome-step ${s.state}"><span class="welcome-step-dot" aria-hidden="true"></span>${escapeHtml(s.label)}</li>`).join("");
    const chips = (api.starters() || []).slice(0, 3).map((text, index) => `<button class="welcome-chip" type="button" data-chip="${index}">${escapeHtml(text)}</button>`).join("");
    body.innerHTML = `
      <div class="welcome-eyebrow">SETTING UP · YOU CAN LEAVE THIS OPEN</div>
      <h1 class="welcome-headline" id="welcome-pct" aria-live="polite">0%</h1>
      <progress id="welcome-bar" class="welcome-progress" max="1" value="0" aria-label="Setup progress"></progress>
      <div id="welcome-meter" class="welcome-meter" role="status"></div>
      <ol class="welcome-steps">${steps}</ol>
      ${chips ? `<div class="welcome-wait"><p>Too slow? Pick a question now. Keel answers the second it is ready.</p><div class="welcome-chips">${chips}</div></div>` : ""}
      <div class="welcome-desktop-actions"><button id="welcome-background" data-primary class="btn" type="button">Keep exploring while it downloads</button>${stage === "start" ? "" : ' <button id="welcome-pause" class="btn btn-ghost" type="button">Pause download</button>'}</div>`;
    body.querySelector("#welcome-background").onclick = hide;
    const pause = body.querySelector("#welcome-pause");
    if (pause) pause.onclick = async () => {
      pause.disabled = true;
      try {
        if (!await invoke("pause_local_setup")) pause.disabled = false;
      } catch {
        pause.disabled = false;
        toast("Couldn’t pause the download. Try again.");
      }
    };
    const starters = api.starters() || [];
    body.querySelectorAll("[data-chip]").forEach((button) => { button.onclick = () => { hide(); api.queuePrompt(starters[Number(button.dataset.chip)]); }; });
    paint();
  }

  function renderPaused() {
    body.innerHTML = `
      <div class="welcome-eyebrow">SETUP PAUSED</div>
      <h1 class="welcome-headline">Continue when you’re ready.</h1>
      <p class="welcome-sub">Keel keeps the downloaded files on this computer.</p>
      <div class="welcome-desktop-actions">
        <button id="welcome-resume" data-primary class="welcome-btn-white" type="button">Resume download</button>
        <button id="welcome-skip" class="btn btn-ghost btn-sm" type="button">Keep exploring</button>
      </div>`;
    body.querySelector("#welcome-resume").onclick = start;
    body.querySelector("#welcome-skip").onclick = hide;
  }

  function renderError() {
    const e = error || friendlySetupError("");
    body.innerHTML = `
      <div class="welcome-eyebrow welcome-eyebrow-warn">THAT DIDN'T WORK</div>
      <h1 class="welcome-headline">Let's try that again.</h1>
      <p class="welcome-sub" role="alert">${escapeHtml(e.message)}</p>
      <div class="welcome-desktop-actions">
        ${e.retry ? '<button id="welcome-retry" data-primary class="welcome-btn-white" type="button">Try again</button>' : ""}
        ${e.existing ? '<button id="welcome-existing" data-primary class="welcome-btn-white" type="button">Connect an AI I already run</button>' : ""}
        <button id="welcome-skip" class="btn btn-ghost btn-sm" type="button">Not now</button>
      </div>
      ${e.detail ? `<details class="welcome-details"><summary>Technical details</summary><pre class="mono">${escapeHtml(e.detail)}</pre></details>` : ""}`;
    const retry = body.querySelector("#welcome-retry");
    if (retry) retry.onclick = start;
    const existing = body.querySelector("#welcome-existing");
    if (existing) existing.onclick = () => { view = "existing"; render(); };
    body.querySelector("#welcome-skip").onclick = skip;
  }

  function renderExisting() {
    body.innerHTML = `
      <div class="welcome-eyebrow">USE WHAT YOU HAVE</div>
      <h1 class="welcome-headline">Connect Ollama or LM Studio.</h1>
      <p class="welcome-sub">Already running Ollama or LM Studio on this computer? Pick it below — nothing extra is downloaded.</p>
      <div class="row" style="margin-top:4px">
        <button type="button" class="btn btn-sm" data-preset-url="http://127.0.0.1:11434/v1">Ollama</button>
        <button type="button" class="btn btn-sm" data-preset-url="http://127.0.0.1:1234/v1">LM Studio</button>
      </div>
      <label class="welcome-label" for="setup-url">App address</label>
      <input id="setup-url" class="input mono" value="http://127.0.0.1:11434/v1" />
      <label class="welcome-label" for="setup-model">Model name (optional)</label>
      <input id="setup-model" class="input" placeholder="Use the first available model" />
      <p class="welcome-sub" id="welcome-server-notice">${serverDataNotice("http://127.0.0.1:11434/v1")}</p>
      <p id="welcome-existing-error" class="welcome-error" role="alert"></p>
      <div class="welcome-desktop-actions">
        <button id="setup-connect" data-primary class="welcome-btn-white" type="button">Connect</button>
        <button id="welcome-back" class="btn btn-ghost btn-sm" type="button">Back</button>
      </div>`;
    body.querySelector("#welcome-back").onclick = () => { view = "intro"; render(); };
    body.querySelectorAll("[data-preset-url]").forEach((btn) => {
      btn.onclick = () => {
        const input = body.querySelector("#setup-url");
        input.value = btn.dataset.presetUrl;
        body.querySelector("#welcome-server-notice").textContent = serverDataNotice(input.value);
      };
    });
    body.querySelector("#setup-url").oninput = (event) => {
      body.querySelector("#welcome-server-notice").textContent = serverDataNotice(event.target.value);
    };
    body.querySelector("#setup-connect").onclick = async (event) => {
      event.target.disabled = true;
      try {
        await invoke("set_external_endpoint", { baseUrl: body.querySelector("#setup-url").value, modelName: body.querySelector("#setup-model").value || null });
        api.markDone();
        hide();
        await api.refresh();
        toast("Connected. Ask your first question.");
        api.ready();
      } catch (failure) {
        body.querySelector("#welcome-existing-error").textContent = friendlyRuntimeError(failure);
        event.target.disabled = false;
      }
    };
  }

  async function start() {
    if (busy || !canAutoSetup()) return;
    const model = chosenModel();
    busy = true;
    try { localStorage.setItem("keel_setup_choice", choice); } catch { /* Storage is optional. */ }
    error = null;
    stage = plan.runtime_path ? (modelInstalled() ? "start" : "model") : "runner";
    tracker = createProgressTracker({ runtime: runnerBytes(), [model.id]: modelInstalled() ? 0 : model.bytes });
    view = "working";
    render();
    api.changed();
    try {
      await invoke("setup_local_ai", { modelId: model.id === plan.recommended_model.id ? null : model.id });
      busy = false;
      api.markDone();
      hide();
      await api.refresh();
      toast("Your AI is ready. Ask it anything.");
      api.ready();
    } catch (failure) {
      busy = false;
      if (String(failure).replace(/^Error:\s*/i, "") === "Setup download paused") {
        view = "paused";
        try { plan = await invoke("get_setup_plan"); } catch { /* The last plan still allows retrying. */ }
      } else {
        error = friendlySetupError(failure);
        view = "error";
      }
      api.changed();
      open();
    }
  }

  function hide() { root.classList.add("hidden"); api.changed(); }
  function skip() { api.markDone(); hide(); }

  function open() {
    root.classList.remove("hidden");
    render();
  }

  async function show() {
    if (!busy && view !== "paused") {
      view = "intro";
      try { plan = await invoke("get_setup_plan"); } catch { plan = null; }
    }
    open();
  }

  return {
    show,
    hide,
    isOpen: () => !root.classList.contains("hidden"),
    status: () => ({ busy, paused: view === "paused", fraction: stage === "start" ? 1 : snapshot().fraction, stage, expectedBytes: totalBytes() }),
    stage(next) { if (!busy) return; stage = next; if (view === "working") render(); api.changed(); },
    download(progress) {
      if (!busy) return;
      tracker.update(progress);
      paint();
      const now = Date.now();
      if (now - lastChange > 250) { lastChange = now; api.changed(); }
    },
    handleEscape() { if (busy) hide(); else if (view === "intro" || view === "error") skip(); },
  };
}
