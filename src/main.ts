import "katex/dist/katex.min.css";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { renderMarkdown } from "./markdown";
import "./styles.css";

type Provider = "chatgpt" | "compatible";
type Protocol = "responses" | "chat_completions";

type Settings = {
  provider: Provider;
  protocol: Protocol;
  baseUrl: string;
  model: string;
};

type Snapshot = {
  settings: Settings;
  chatgpt: { signedIn: boolean; email: string | null };
  hasApiKey: boolean;
};

type ResponseEvent =
  | { event: "started" }
  | { event: "text_delta"; data: { delta: string } }
  | { event: "source"; data: { title: string; url: string } }
  | { event: "completed" }
  | { event: "cancelled" }
  | { event: "failed"; data: { message: string } };

type LoginEvent = {
  success: boolean;
  email?: string;
  error?: string;
};

const app = document.querySelector<HTMLElement>("#app");

if (!app) {
  throw new Error("Application root was not found");
}

app.innerHTML = `
  <section class="shell">
    <header class="masthead">
      <div>
        <p class="eyebrow">Worked Examples</p>
        <h1>Understand it<br>one step at a time.</h1>
      </div>
      <button class="provider-button" id="open-settings" type="button">
        <span id="provider-dot" class="status-dot"></span>
        <span id="provider-label">Loading...</span>
      </button>
    </header>

    <section class="workspace">
      <form class="composer" id="prompt-form">
        <label for="prompt">What should we work through?</label>
        <textarea id="prompt" rows="4" placeholder="For example: Solve 2x² + 7x - 4 = 0 and explain each step."></textarea>
        <div class="composer-actions">
          <label class="search-option">
            <input id="web-search" type="checkbox">
            <span>Search the web</span>
          </label>
          <div class="action-buttons">
            <button class="secondary hidden" id="stop" type="button">Stop</button>
            <button class="primary" id="generate" type="submit">Work it out</button>
          </div>
        </div>
        <p class="capability-note" id="capability-note"></p>
      </form>

      <article class="result empty" id="result" aria-live="polite">
        <div class="empty-state" id="empty-state">
          <span>01</span>
          <p>Ask for a worked example and the explanation will appear here.</p>
        </div>
        <div class="result-status hidden" id="result-status"></div>
        <div class="markdown hidden" id="markdown"></div>
        <section class="sources hidden" id="sources">
          <h2>Sources</h2>
          <ol id="source-list"></ol>
        </section>
      </article>
    </section>
  </section>

  <dialog id="settings-dialog">
    <form class="settings" id="settings-form" method="dialog">
      <div class="settings-heading">
        <div>
          <p class="eyebrow">Connection</p>
          <h2>Choose how to think.</h2>
        </div>
        <button class="icon-button" id="close-settings" type="button" aria-label="Close settings">&times;</button>
      </div>

      <fieldset class="provider-choice">
        <legend>Provider</legend>
        <label>
          <input type="radio" name="provider" value="chatgpt">
          <span><strong>ChatGPT</strong><small>Use a Plus, Pro, or workspace subscription</small></span>
        </label>
        <label>
          <input type="radio" name="provider" value="compatible">
          <span><strong>Compatible endpoint</strong><small>Connect to an OpenAI-shaped HTTP API</small></span>
        </label>
      </fieldset>

      <section class="provider-settings" id="chatgpt-settings">
        <div class="account-row">
          <div>
            <strong id="account-title">Not signed in</strong>
            <small id="account-detail">Authorization opens in your browser.</small>
          </div>
          <button class="secondary" id="account-action" type="button">Sign in</button>
        </div>
      </section>

      <section class="provider-settings hidden" id="compatible-settings">
        <label>Protocol
          <select id="protocol">
            <option value="responses">Responses</option>
            <option value="chat_completions">Chat Completions</option>
          </select>
        </label>
        <label>Base URL
          <input id="base-url" type="url" placeholder="http://localhost:11434">
        </label>
        <label>API key
          <input id="api-key" type="password" autocomplete="off" placeholder="Leave unchanged">
          <small id="api-key-help">No API key is stored.</small>
        </label>
      </section>

      <label class="model-field">Model
        <input id="model" type="text" required>
      </label>

      <p class="form-error hidden" id="settings-error"></p>
      <div class="settings-actions">
        <button class="secondary" id="cancel-settings" type="button">Cancel</button>
        <button class="primary" type="submit" value="save">Save connection</button>
      </div>
    </form>
  </dialog>
`;

const elements = {
  openSettings: requiredButton("open-settings"),
  providerDot: required("provider-dot"),
  providerLabel: required("provider-label"),
  promptForm: requiredForm("prompt-form"),
  prompt: requiredTextArea("prompt"),
  webSearch: requiredInput("web-search"),
  generate: requiredButton("generate"),
  stop: requiredButton("stop"),
  capabilityNote: required("capability-note"),
  result: required("result"),
  emptyState: required("empty-state"),
  resultStatus: required("result-status"),
  markdown: required("markdown"),
  sources: required("sources"),
  sourceList: required("source-list"),
  dialog: requiredDialog("settings-dialog"),
  settingsForm: requiredForm("settings-form"),
  closeSettings: requiredButton("close-settings"),
  cancelSettings: requiredButton("cancel-settings"),
  chatgptSettings: required("chatgpt-settings"),
  compatibleSettings: required("compatible-settings"),
  accountTitle: required("account-title"),
  accountDetail: required("account-detail"),
  accountAction: requiredButton("account-action"),
  protocol: requiredSelect("protocol"),
  baseUrl: requiredInput("base-url"),
  apiKey: requiredInput("api-key"),
  apiKeyHelp: required("api-key-help"),
  model: requiredInput("model"),
  settingsError: required("settings-error"),
};

let snapshot: Snapshot | null = null;
let output = "";
let renderQueued = false;
let busy = false;
const sourceMap = new Map<string, string>();

void initialize();

async function initialize() {
  try {
    snapshot = await invoke<Snapshot>("get_app_snapshot");
    updateConnectionUI();
  } catch (error) {
    showResultError(messageFrom(error));
  }

  await listen<LoginEvent>("chatgpt-login-completed", ({ payload }) => {
    elements.accountAction.disabled = false;
    if (!payload.success) {
      showSettingsError(payload.error ?? "ChatGPT sign-in failed");
      return;
    }
    void refreshSnapshot();
  });
}

elements.openSettings.addEventListener("click", () => {
  populateSettings();
  elements.dialog.showModal();
});
elements.closeSettings.addEventListener("click", () => elements.dialog.close());
elements.cancelSettings.addEventListener("click", () => elements.dialog.close());
elements.dialog.addEventListener("click", (event) => {
  if (event.target === elements.dialog) elements.dialog.close();
});

for (const input of document.querySelectorAll<HTMLInputElement>('input[name="provider"]')) {
  input.addEventListener("change", updateSettingsSections);
}

elements.settingsForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void saveConnection();
});

elements.accountAction.addEventListener("click", () => void changeChatgptAccount());
elements.promptForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void generate();
});
elements.stop.addEventListener("click", () => void invoke("cancel_generation"));

elements.markdown.addEventListener("click", openExternalLink);
elements.sourceList.addEventListener("click", openExternalLink);

function populateSettings() {
  if (!snapshot) return;
  const settings = snapshot.settings;
  const provider = document.querySelector<HTMLInputElement>(
    `input[name="provider"][value="${settings.provider}"]`,
  );
  if (provider) provider.checked = true;
  elements.protocol.value = settings.protocol;
  elements.baseUrl.value = settings.baseUrl;
  elements.model.value = settings.model;
  elements.apiKey.value = "";
  elements.apiKey.placeholder = snapshot.hasApiKey ? "Leave unchanged" : "Optional";
  elements.apiKeyHelp.textContent = snapshot.hasApiKey
    ? "An API key is stored in Windows Credential Manager. Enter a replacement, or type CLEAR to remove it."
    : "No API key is stored.";
  hideSettingsError();
  updateSettingsSections();
  updateAccountUI();
}

function updateSettingsSections() {
  const provider = selectedProvider();
  elements.chatgptSettings.classList.toggle("hidden", provider !== "chatgpt");
  elements.compatibleSettings.classList.toggle("hidden", provider !== "compatible");
}

async function saveConnection() {
  if (!snapshot) return;
  hideSettingsError();
  const apiKeyInput = elements.apiKey.value;
  const settings: Settings = {
    provider: selectedProvider(),
    protocol: elements.protocol.value as Protocol,
    baseUrl: elements.baseUrl.value.trim(),
    model: elements.model.value.trim(),
  };
  try {
    snapshot = await invoke<Snapshot>("save_settings", {
      request: {
        settings,
        apiKey: apiKeyInput === "CLEAR" ? null : apiKeyInput,
        changeApiKey: apiKeyInput.length > 0,
      },
    });
    elements.dialog.close();
    updateConnectionUI();
  } catch (error) {
    showSettingsError(messageFrom(error));
  }
}

async function changeChatgptAccount() {
  if (!snapshot) return;
  elements.accountAction.disabled = true;
  hideSettingsError();
  try {
    if (snapshot.chatgpt.signedIn) {
      snapshot = await invoke<Snapshot>("logout_chatgpt");
      updateAccountUI();
      updateConnectionUI();
      elements.accountAction.disabled = false;
      return;
    }
    const url = await invoke<string>("start_chatgpt_login");
    await openUrl(url);
    elements.accountTitle.textContent = "Waiting for browser";
    elements.accountDetail.textContent = "Complete authorization in the browser window.";
  } catch (error) {
    elements.accountAction.disabled = false;
    showSettingsError(messageFrom(error));
  }
}

async function refreshSnapshot() {
  try {
    snapshot = await invoke<Snapshot>("get_app_snapshot");
    updateAccountUI();
    updateConnectionUI();
  } catch (error) {
    showSettingsError(messageFrom(error));
  }
}

function updateAccountUI() {
  if (!snapshot) return;
  const { chatgpt } = snapshot;
  elements.accountTitle.textContent = chatgpt.signedIn ? chatgpt.email ?? "ChatGPT connected" : "Not signed in";
  elements.accountDetail.textContent = chatgpt.signedIn
    ? "Tokens are secured by Windows Credential Manager."
    : "Authorization opens in your browser.";
  elements.accountAction.textContent = chatgpt.signedIn ? "Sign out" : "Sign in";
  elements.accountAction.disabled = false;
}

function updateConnectionUI() {
  if (!snapshot) return;
  const { settings, chatgpt, hasApiKey } = snapshot;
  const ready = settings.provider === "chatgpt" ? chatgpt.signedIn : hasApiKey || settings.baseUrl.length > 0;
  elements.providerDot.classList.toggle("ready", ready);
  elements.providerLabel.textContent =
    settings.provider === "chatgpt"
      ? chatgpt.signedIn
        ? `ChatGPT · ${settings.model}`
        : "ChatGPT · Sign in"
      : `Endpoint · ${settings.model}`;

  const searchSupported = settings.provider === "chatgpt" || settings.protocol === "responses";
  elements.webSearch.disabled = !searchSupported;
  if (!searchSupported) elements.webSearch.checked = false;
  elements.capabilityNote.textContent = searchSupported
    ? "Search asks the selected provider to ground the example with live sources."
    : "Chat Completions does not advertise a web-search capability.";
}

async function generate() {
  if (busy) return;
  const prompt = elements.prompt.value.trim();
  if (!prompt) {
    elements.prompt.focus();
    return;
  }

  output = "";
  sourceMap.clear();
  setBusy(true);
  elements.result.classList.remove("empty");
  elements.emptyState.classList.add("hidden");
  elements.markdown.classList.remove("hidden");
  elements.markdown.innerHTML = "";
  elements.sources.classList.add("hidden");
  elements.resultStatus.className = "result-status thinking";
  elements.resultStatus.textContent = "Working through it...";

  const channel = new Channel<ResponseEvent>();
  channel.onmessage = (event) => handleResponseEvent(event);

  try {
    await invoke("generate_example", {
      request: { prompt, webSearch: elements.webSearch.checked },
      onEvent: channel,
    });
  } catch (error) {
    showResultError(messageFrom(error));
    setBusy(false);
  }
}

function handleResponseEvent(event: ResponseEvent) {
  switch (event.event) {
    case "started":
      return;
    case "text_delta":
      output += event.data.delta;
      scheduleMarkdownRender();
      return;
    case "source":
      sourceMap.set(event.data.url, event.data.title);
      renderSources();
      return;
    case "completed":
      elements.resultStatus.className = "result-status complete";
      elements.resultStatus.textContent = "Worked example";
      setBusy(false);
      return;
    case "cancelled":
      elements.resultStatus.className = "result-status";
      elements.resultStatus.textContent = "Stopped";
      setBusy(false);
      return;
    case "failed":
      showResultError(event.data.message);
      setBusy(false);
  }
}

function scheduleMarkdownRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    elements.markdown.innerHTML = renderMarkdown(output);
    secureRenderedLinks(elements.markdown);
  });
}

function renderSources() {
  if (sourceMap.size === 0) return;
  elements.sourceList.replaceChildren(
    ...Array.from(sourceMap, ([url, title]) => {
      const item = document.createElement("li");
      const link = document.createElement("a");
      link.href = url;
      link.textContent = title;
      link.rel = "noopener noreferrer";
      item.append(link);
      return item;
    }),
  );
  elements.sources.classList.remove("hidden");
}

function secureRenderedLinks(root: HTMLElement) {
  for (const link of root.querySelectorAll<HTMLAnchorElement>("a")) {
    try {
      const url = new URL(link.href);
      if (url.protocol !== "http:" && url.protocol !== "https:") link.removeAttribute("href");
    } catch {
      link.removeAttribute("href");
    }
    link.rel = "noopener noreferrer";
  }
}

function openExternalLink(event: MouseEvent) {
  const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
  if (!link) return;
  event.preventDefault();
  const url = new URL(link.href);
  if (url.protocol === "http:" || url.protocol === "https:") void openUrl(url.toString());
}

function setBusy(value: boolean) {
  busy = value;
  elements.generate.disabled = value;
  elements.stop.classList.toggle("hidden", !value);
  elements.prompt.disabled = value;
}

function showResultError(message: string) {
  elements.result.classList.remove("empty");
  elements.emptyState.classList.add("hidden");
  elements.resultStatus.className = "result-status error";
  elements.resultStatus.textContent = message;
}

function selectedProvider(): Provider {
  return document.querySelector<HTMLInputElement>('input[name="provider"]:checked')?.value === "compatible"
    ? "compatible"
    : "chatgpt";
}

function showSettingsError(message: string) {
  elements.settingsError.textContent = message;
  elements.settingsError.classList.remove("hidden");
}

function hideSettingsError() {
  elements.settingsError.classList.add("hidden");
  elements.settingsError.textContent = "";
}

function messageFrom(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function required(id: string): HTMLElement {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing #${id}`);
  return element;
}

function requiredButton(id: string): HTMLButtonElement {
  return required(id) as HTMLButtonElement;
}

function requiredInput(id: string): HTMLInputElement {
  return required(id) as HTMLInputElement;
}

function requiredTextArea(id: string): HTMLTextAreaElement {
  return required(id) as HTMLTextAreaElement;
}

function requiredSelect(id: string): HTMLSelectElement {
  return required(id) as HTMLSelectElement;
}

function requiredForm(id: string): HTMLFormElement {
  return required(id) as HTMLFormElement;
}

function requiredDialog(id: string): HTMLDialogElement {
  return required(id) as HTMLDialogElement;
}
