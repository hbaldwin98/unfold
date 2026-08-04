import "katex/dist/katex.min.css";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  extractSuggestions,
  isLearningMode,
  labelForAction,
  previousTurns,
  renderLearningTurns,
  supportsLearningActions,
  type LearningAction,
  type LearningMode,
  type LearningTurn,
  visibleLearningContent,
} from "./learning";
import { renderMarkdown } from "./markdown";
import "./styles.css";

type Provider = "chatgpt" | "compatible";
type Protocol = "responses" | "chat_completions";
type ReasoningEffort = "default" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

type Settings = {
  provider: Provider;
  protocol: Protocol;
  baseUrl: string;
  model: string;
  reasoningEffort: ReasoningEffort;
};

type ModelOption = {
  id: string;
  name: string;
  isDefault: boolean;
  defaultReasoningEffort: string | null;
  reasoningEfforts: Array<{ effort: string; description: string }>;
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

type ComposerMode =
  | "new_problem"
  | "idle"
  | "socratic_response"
  | "follow_up"
  | "explain_step"
  | "check_attempt";

const app = document.querySelector<HTMLElement>("#app");
// Keep the pre-rename key so existing installations retain their selected mode.
const MODE_STORAGE_KEY = "worked-examples.learning-mode";

if (!app) {
  throw new Error("Application root was not found");
}

app.innerHTML = `
  <section class="shell">
    <header class="masthead">
      <button class="provider-button" id="open-settings" type="button">
        <span id="provider-dot" class="status-dot"></span>
        <span id="provider-label">Loading...</span>
      </button>
    </header>

    <section class="workspace">
      <form class="composer" id="prompt-form">
        <label for="prompt">What are you working on?</label>
        <textarea id="prompt" rows="2" placeholder="Ask for guidance and a worked example..."></textarea>
        <div class="composer-footer">
          <div class="mode-picker" id="mode-picker" role="group" aria-label="Learning mode">
            <span>Mode</span>
            <button class="mode-option" type="button" data-learning-mode="socratic" title="Hints, questions, and attempt feedback before revealing the solution">Socratic</button>
            <button class="mode-option" type="button" data-learning-mode="worked_example" title="A complete step-by-step solution in one response">Worked example</button>
          </div>
          <div class="composer-actions">
            <label class="search-option" id="search-option">
              <input id="web-search" type="checkbox">
              <span>Web</span>
            </label>
            <div class="action-buttons">
              <button class="secondary hidden" id="stop" type="button">Stop</button>
              <button class="primary" id="generate" type="submit">Work it out</button>
            </div>
          </div>
        </div>
        <p class="capability-note" id="capability-note"></p>
      </form>

      <article class="result empty" id="result" aria-live="polite">
        <div class="empty-state" id="empty-state">
          <p>Your guidance and worked example will appear here.</p>
        </div>
        <section class="target-message hidden" id="target-message">
          <span id="target-label">You</span>
          <p id="target-text"></p>
        </section>
        <div class="result-status hidden" id="result-status"></div>
        <div class="markdown hidden" id="markdown"></div>
        <section class="sources hidden" id="sources">
          <h2>Sources</h2>
          <ol id="source-list"></ol>
        </section>
        <section class="suggested-responses hidden" id="suggested-responses" aria-label="Suggested responses">
          <p>Possible next moves</p>
          <div id="suggestion-list"></div>
        </section>
        <section class="learning-actions hidden" id="learning-actions" aria-label="Learning actions">
          <div class="learning-action-list" id="learning-action-list">
            <button class="learning-action" id="another-hint" type="button">Give me a hint</button>
            <button class="learning-action" id="explain-step" type="button">Explain a step</button>
            <button class="learning-action" id="check-attempt" type="button">Check my attempt</button>
            <button class="learning-action reveal-action" id="reveal-solution" type="button">Show solution</button>
          </div>
          <button class="new-problem" id="new-problem" type="button">New problem</button>
        </section>
      </article>
    </section>
  </section>

  <aside class="term-panel hidden" id="term-panel" tabindex="-1" aria-live="polite" aria-label="Contextual term explanation">
    <header>
      <div>
        <h2 id="term-panel-title"></h2>
      </div>
      <button class="icon-button" id="close-term-panel" type="button" aria-label="Close term explanation">&times;</button>
    </header>
    <div class="term-panel-status" id="term-panel-status"></div>
    <div class="markdown term-panel-content" id="term-panel-content"></div>
  </aside>
  <button class="term-panel-tab hidden" id="reopen-term-panel" type="button" aria-label="Reopen contextual term explanation">Context</button>

  <dialog id="settings-dialog">
    <form class="settings" id="settings-form" method="dialog">
      <div class="settings-heading">
        <div>
          <h2>Connection</h2>
          <p>Choose the model used for worked examples.</p>
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

      <div class="model-field">
        <div class="field-heading">
          <label for="model">Model</label>
          <button class="text-button" id="refresh-models" type="button">Refresh models</button>
        </div>
        <select id="model-select" aria-label="Model"></select>
        <input class="hidden" id="model" type="text" required autocomplete="off" placeholder="Enter a model ID">
        <small id="model-help">Enter a model ID or load models from the saved connection.</small>
      </div>

      <label class="reasoning-field">Reasoning effort
        <select id="reasoning-effort"></select>
        <small>Controls internal reasoning depth. Reasoning text is never displayed.</small>
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
  modePicker: required("mode-picker"),
  webSearch: requiredInput("web-search"),
  generate: requiredButton("generate"),
  stop: requiredButton("stop"),
  searchOption: required("search-option"),
  capabilityNote: required("capability-note"),
  result: required("result"),
  emptyState: required("empty-state"),
  targetMessage: required("target-message"),
  targetLabel: required("target-label"),
  targetText: required("target-text"),
  resultStatus: required("result-status"),
  markdown: required("markdown"),
  sources: required("sources"),
  sourceList: required("source-list"),
  suggestedResponses: required("suggested-responses"),
  suggestionList: required("suggestion-list"),
  learningActions: required("learning-actions"),
  learningActionList: required("learning-action-list"),
  anotherHint: requiredButton("another-hint"),
  explainStep: requiredButton("explain-step"),
  checkAttempt: requiredButton("check-attempt"),
  revealSolution: requiredButton("reveal-solution"),
  newProblem: requiredButton("new-problem"),
  termPanel: required("term-panel"),
  termPanelTitle: required("term-panel-title"),
  termPanelStatus: required("term-panel-status"),
  termPanelContent: required("term-panel-content"),
  closeTermPanel: requiredButton("close-term-panel"),
  reopenTermPanel: requiredButton("reopen-term-panel"),
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
  modelSelect: requiredSelect("model-select"),
  modelHelp: required("model-help"),
  refreshModels: requiredButton("refresh-models"),
  reasoningEffort: requiredSelect("reasoning-effort"),
  settingsError: required("settings-error"),
};

let snapshot: Snapshot | null = null;
let modelCatalog: ModelOption[] = [];
let learningMode = loadLearningMode();
let sessionMode: LearningMode | null = null;
let renderQueued = false;
let busy = false;
let targetProblem = "";
let sessionWebSearch = false;
let composerMode: ComposerMode = "new_problem";
let activeTurnIndex = -1;
let termPanelMarkdown = "";
let termPanelTrigger: HTMLButtonElement | null = null;
const learningTurns: LearningTurn[] = [];
const sourceMap = new Map<string, string>();

updateModeUI();
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
  input.addEventListener("change", () => {
    updateSettingsSections();
    if (snapshot && selectedProvider() !== snapshot.settings.provider) {
      elements.modelHelp.textContent = "Save this provider before loading its model catalog.";
    }
  });
}
elements.refreshModels.addEventListener("click", () => void refreshModelCatalog());
elements.model.addEventListener("input", updateReasoningOptions);
elements.modelSelect.addEventListener("change", () => {
  const custom = elements.modelSelect.value === "__custom__";
  elements.model.classList.toggle("hidden", !custom);
  if (custom) elements.model.focus();
  updateReasoningOptions();
});

elements.settingsForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void saveConnection();
});

elements.accountAction.addEventListener("click", () => void changeChatgptAccount());
for (const button of elements.modePicker.querySelectorAll<HTMLButtonElement>("[data-learning-mode]")) {
  button.addEventListener("click", () => {
    const mode = button.dataset.learningMode ?? null;
    if (busy || targetProblem || !isLearningMode(mode)) return;
    learningMode = mode;
    try {
      localStorage.setItem(MODE_STORAGE_KEY, mode);
    } catch {
      // The mode still applies to this window if persistent WebView storage is unavailable.
    }
    updateModeUI();
    updateComposerUI();
  });
}
elements.promptForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void submitComposer();
});
elements.prompt.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    void submitComposer();
  } else if (event.key === "Escape" && !busy) {
    setComposerMode(targetProblem ? restingComposerMode() : "new_problem");
  }
});
elements.prompt.addEventListener("input", resizePrompt);
elements.stop.addEventListener("click", () => {
  if (busy) {
    void invoke("cancel_generation");
  } else {
    setComposerMode(targetProblem ? restingComposerMode() : "new_problem");
  }
});
elements.anotherHint.addEventListener("click", () => void startGeneration("another_hint"));
elements.explainStep.addEventListener("click", () => prepareDetail("explain_step"));
elements.checkAttempt.addEventListener("click", () => prepareDetail("check_attempt"));
elements.revealSolution.addEventListener("click", () => void startGeneration("reveal_solution"));
elements.newProblem.addEventListener("click", startNewProblem);

elements.markdown.addEventListener("click", openExternalLink);
elements.markdown.addEventListener("click", (event) => {
  const button = (event.target as Element | null)?.closest<HTMLButtonElement>(
    "button.learning-term[data-term]",
  );
  const term = button?.dataset.term?.trim();
  if (!term || busy || !targetProblem) return;
  termPanelTrigger = button ?? null;
  void startTermExplanation(term);
});
elements.closeTermPanel.addEventListener("click", closeTermPanel);
elements.reopenTermPanel.addEventListener("click", () => {
  elements.reopenTermPanel.classList.add("hidden");
  elements.termPanel.classList.remove("hidden");
  elements.termPanel.focus();
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !elements.termPanel.classList.contains("hidden")) {
    event.preventDefault();
    closeTermPanel();
  }
});
elements.sourceList.addEventListener("click", openExternalLink);
elements.termPanelContent.addEventListener("click", openExternalLink);
elements.suggestionList.addEventListener("click", (event) => {
  const button = (event.target as Element | null)?.closest<HTMLButtonElement>("button[value]");
  if (!button || busy || composerMode !== "socratic_response") return;
  elements.prompt.value = button.value;
  void submitComposer();
});

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
  elements.reasoningEffort.value = settings.reasoningEffort;
  elements.apiKey.value = "";
  elements.apiKey.placeholder = snapshot.hasApiKey ? "Leave unchanged" : "Optional";
  elements.apiKeyHelp.textContent = snapshot.hasApiKey
    ? "An API key is stored in Windows Credential Manager. Enter a replacement, or type CLEAR to remove it."
    : "No API key is stored.";
  hideSettingsError();
  updateSettingsSections();
  updateAccountUI();
  showCurrentModelOnly();
  void refreshModelCatalog();
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
  const provider = selectedProvider();
  const settings: Settings = {
    provider,
    protocol: provider === "chatgpt" ? "responses" : (elements.protocol.value as Protocol),
    baseUrl: elements.baseUrl.value.trim(),
    model: selectedModelId(),
    reasoningEffort: elements.reasoningEffort.value as ReasoningEffort,
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

async function refreshModelCatalog() {
  if (!snapshot) return;
  if (selectedProvider() !== snapshot.settings.provider) {
    elements.modelHelp.textContent = "Save this provider before loading its model catalog.";
    return;
  }

  elements.refreshModels.disabled = true;
  elements.refreshModels.textContent = "Loading...";
  elements.modelHelp.textContent = "Loading models from the saved connection...";
  try {
    modelCatalog = await invoke<ModelOption[]>("list_models");
    renderModelCatalog();
    elements.modelHelp.textContent = modelCatalog.length
      ? `${modelCatalog.length} models available. You can still enter a custom model ID.`
      : "The provider returned no selectable models; enter a model ID manually.";
  } catch (error) {
    modelCatalog = [];
    showCurrentModelOnly();
    elements.modelHelp.textContent = `${messageFrom(error)} You can still enter a model ID manually.`;
  } finally {
    elements.refreshModels.disabled = false;
    elements.refreshModels.textContent = "Refresh models";
  }
}

function showCurrentModelOnly() {
  modelCatalog = [];
  const current = elements.model.value.trim();
  const currentOption = document.createElement("option");
  currentOption.value = current;
  currentOption.textContent = `${current} (current)`;
  const customOption = document.createElement("option");
  customOption.value = "__custom__";
  customOption.textContent = "Custom model ID...";
  elements.modelSelect.replaceChildren(currentOption, customOption);
  elements.modelSelect.value = current;
  elements.model.classList.add("hidden");
  updateReasoningOptions();
}

function renderModelCatalog() {
  const current = elements.model.value.trim();
  const options = modelCatalog.map((model) => {
    const option = document.createElement("option");
    option.value = model.id;
    option.textContent = `${model.name}${model.isDefault ? " - Recommended" : ""}`;
    return option;
  });
  if (current && !modelCatalog.some((model) => model.id === current)) {
    const currentOption = document.createElement("option");
    currentOption.value = current;
    currentOption.textContent = `${current} - Current custom model`;
    options.push(currentOption);
  }
  const customOption = document.createElement("option");
  customOption.value = "__custom__";
  customOption.textContent = "Custom model ID...";
  elements.modelSelect.replaceChildren(
    ...options,
    customOption,
  );
  elements.modelSelect.value = current || modelCatalog[0]?.id || "__custom__";
  elements.model.classList.toggle("hidden", elements.modelSelect.value !== "__custom__");
  updateReasoningOptions();
}

function updateReasoningOptions() {
  const current = (elements.reasoningEffort.value || snapshot?.settings.reasoningEffort || "default") as ReasoningEffort;
  const model = modelCatalog.find((candidate) => candidate.id === selectedModelId());
  const supported = model?.reasoningEfforts.filter((option) => isReasoningEffort(option.effort)) ?? [];
  const fallback = ["none", "minimal", "low", "medium", "high", "xhigh", "max"].map(
    (effort) => ({ effort, description: "" }),
  );
  const efforts = supported.length ? supported : fallback;
  const defaultLabel = model?.defaultReasoningEffort
    ? `Model default (${model.defaultReasoningEffort})`
    : "Model default";

  const defaultOption = document.createElement("option");
  defaultOption.value = "default";
  defaultOption.textContent = defaultLabel;
  elements.reasoningEffort.replaceChildren(
    defaultOption,
    ...efforts.map(({ effort, description }) => {
      const option = document.createElement("option");
      option.value = effort;
      option.textContent = description
        ? `${formatEffort(effort)} - ${description}`
        : formatEffort(effort);
      return option;
    }),
  );
  elements.reasoningEffort.value = Array.from(elements.reasoningEffort.options).some(
    (option) => option.value === current,
  )
    ? current
    : "default";
}

function selectedModelId(): string {
  return elements.modelSelect.value === "__custom__"
    ? elements.model.value.trim()
    : elements.modelSelect.value;
}

function isReasoningEffort(value: string): value is Exclude<ReasoningEffort, "default"> {
  return ["none", "minimal", "low", "medium", "high", "xhigh", "max"].includes(value);
}

function formatEffort(value: string): string {
  return value === "xhigh" ? "Extra high" : `${value.charAt(0).toUpperCase()}${value.slice(1)}`;
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

async function submitComposer() {
  if (busy || composerMode === "idle") return;
  const detail = elements.prompt.value.trim();
  if (!detail) {
    elements.prompt.focus();
    return;
  }

  if (composerMode === "new_problem") {
    targetProblem = detail;
    sessionMode = learningMode;
    sessionWebSearch = elements.webSearch.checked;
    sourceMap.clear();
    learningTurns.length = 0;
    elements.targetText.textContent = targetProblem;
    elements.targetLabel.textContent =
      sessionMode === "worked_example" ? "You · Worked example" : "You · Socratic";
    elements.targetMessage.classList.remove("hidden");
    renderSources();
    await startGeneration("initial");
    return;
  }

  await startGeneration(composerMode, detail);
}

async function startGeneration(action: LearningAction, detail?: string) {
  if (busy || !targetProblem || !sessionMode) return;
  const context = previousTurns(learningTurns);
  learningTurns.push({
    action,
    label: labelForAction(action, sessionMode),
    content: "",
    detail,
  });
  activeTurnIndex = learningTurns.length - 1;
  elements.prompt.value = "";
  resizePrompt();
  scheduleMarkdownRender();

  setBusy(true);
  elements.result.classList.remove("empty");
  elements.emptyState.classList.add("hidden");
  elements.markdown.classList.remove("hidden");
  elements.resultStatus.className = "result-status thinking";
  elements.resultStatus.textContent = statusForAction(action);
  renderLearningActions();

  const channel = new Channel<ResponseEvent>();
  channel.onmessage = (event) => handleResponseEvent(event);

  try {
    await invoke("generate_example", {
      request: {
        target: targetProblem,
        mode: sessionMode,
        action,
        detail: detail ?? null,
        previousTurns: context,
        webSearch: sessionWebSearch,
      },
      onEvent: channel,
    });
  } catch (error) {
    discardEmptyActiveTurn();
    showResultError(messageFrom(error));
    setBusy(false);
  }
}

async function startTermExplanation(term: string) {
  if (busy || !targetProblem || !sessionMode) return;
  termPanelMarkdown = "";
  elements.termPanelTitle.textContent = term;
  elements.termPanelStatus.textContent = "Explaining in context...";
  elements.termPanelStatus.classList.remove("error");
  elements.termPanelContent.replaceChildren();
  elements.reopenTermPanel.classList.add("hidden");
  elements.termPanel.classList.remove("hidden");
  elements.termPanel.focus();
  setBusy(true);

  const channel = new Channel<ResponseEvent>();
  channel.onmessage = (event) => {
    switch (event.event) {
      case "started":
      case "source":
        return;
      case "text_delta":
        termPanelMarkdown += event.data.delta;
        elements.termPanelContent.innerHTML = renderMarkdown(visibleLearningContent(termPanelMarkdown));
        secureRenderedLinks(elements.termPanelContent);
        elements.termPanelContent.scrollTop = elements.termPanelContent.scrollHeight;
        return;
      case "completed":
        elements.termPanelStatus.textContent = "Contextual explanation";
        setBusy(false);
        return;
      case "cancelled":
        elements.termPanelStatus.textContent = "Stopped";
        setBusy(false);
        return;
      case "failed":
        elements.termPanelStatus.textContent = event.data.message;
        elements.termPanelStatus.classList.add("error");
        setBusy(false);
    }
  };

  try {
    await invoke("generate_example", {
      request: {
        target: targetProblem,
        mode: sessionMode,
        action: "explain_term",
        detail: term,
        previousTurns: previousTurns(learningTurns),
        webSearch: sessionWebSearch,
      },
      onEvent: channel,
    });
  } catch (error) {
    elements.termPanelStatus.textContent = messageFrom(error);
    elements.termPanelStatus.classList.add("error");
    setBusy(false);
  }
}

function handleResponseEvent(event: ResponseEvent) {
  switch (event.event) {
    case "started":
      return;
    case "text_delta":
      if (activeTurnIndex >= 0) {
        const turn = learningTurns[activeTurnIndex];
        if (turn) turn.content += event.data.delta;
      }
      scheduleMarkdownRender();
      return;
    case "source":
      sourceMap.set(event.data.url, event.data.title);
      renderSources();
      return;
    case "completed":
      elements.resultStatus.className = "result-status complete";
      elements.resultStatus.textContent = "Ready";
      activeTurnIndex = -1;
      setBusy(false);
      return;
    case "cancelled":
      discardEmptyActiveTurn();
      elements.resultStatus.className = "result-status";
      elements.resultStatus.textContent = "Stopped";
      setBusy(false);
      return;
    case "failed":
      discardEmptyActiveTurn();
      showResultError(event.data.message);
      setBusy(false);
  }
}

function scheduleMarkdownRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    const distanceFromBottom =
      elements.result.scrollHeight - elements.result.scrollTop - elements.result.clientHeight;
    elements.markdown.replaceChildren(renderLearningTurns(learningTurns));
    secureRenderedLinks(elements.markdown);
    if (distanceFromBottom < 80) {
      elements.result.scrollTop = elements.result.scrollHeight;
    }
  });
}

function prepareDetail(mode: "explain_step" | "check_attempt") {
  if (busy || !targetProblem || sessionMode !== "socratic") return;
  setComposerMode(mode);
  if (mode === "explain_step") {
    const selection = window.getSelection()?.toString().trim();
    if (selection && selection.length <= 2_000) {
      elements.prompt.value = selection;
      elements.prompt.select();
    }
  }
  elements.prompt.focus();
}

function startNewProblem() {
  if (busy) return;
  targetProblem = "";
  sessionMode = null;
  sessionWebSearch = false;
  activeTurnIndex = -1;
  learningTurns.length = 0;
  sourceMap.clear();
  elements.markdown.replaceChildren();
  elements.targetText.textContent = "";
  elements.targetMessage.classList.add("hidden");
  elements.sourceList.replaceChildren();
  elements.sources.classList.add("hidden");
  elements.suggestionList.replaceChildren();
  elements.suggestedResponses.classList.add("hidden");
  elements.resultStatus.classList.add("hidden");
  elements.markdown.classList.add("hidden");
  elements.emptyState.classList.remove("hidden");
  elements.result.classList.add("empty");
  elements.termPanel.classList.add("hidden");
  elements.reopenTermPanel.classList.add("hidden");
  termPanelTrigger = null;
  setComposerMode("new_problem");
  elements.prompt.focus();
}

function discardEmptyActiveTurn() {
  if (activeTurnIndex >= 0 && !learningTurns[activeTurnIndex]?.content.trim()) {
    const [discarded] = learningTurns.splice(activeTurnIndex, 1);
    if (discarded?.detail) elements.prompt.value = discarded.detail;
    elements.markdown.replaceChildren(renderLearningTurns(learningTurns));
  }
  activeTurnIndex = -1;
}

function statusForAction(action: LearningAction): string {
  switch (action) {
    case "initial":
      return sessionMode === "worked_example"
        ? "Working through the complete example..."
        : "Building your guidance...";
    case "socratic_response":
      return "Considering your reasoning...";
    case "follow_up":
      return "Answering your question...";
    case "explain_term":
      return "Explaining that term...";
    case "another_hint":
      return "Finding a guiding question...";
    case "explain_step":
      return "Explaining that step...";
    case "check_attempt":
      return "Checking your attempt...";
    case "reveal_solution":
      return "Working the target solution...";
  }
}

function renderSources() {
  if (sourceMap.size === 0) {
    elements.sourceList.replaceChildren();
    elements.sources.classList.add("hidden");
    return;
  }
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
  elements.result.setAttribute("aria-busy", String(value && activeTurnIndex >= 0));
  elements.termPanel.setAttribute(
    "aria-busy",
    String(value && activeTurnIndex < 0 && !elements.termPanel.classList.contains("hidden")),
  );
  if (!value) {
    if (learningTurns.some((turn) => turn.content.trim())) {
      composerMode = restingComposerMode();
    } else {
      elements.prompt.value = targetProblem;
      targetProblem = "";
      sessionMode = null;
      composerMode = "new_problem";
    }
  }
  updateComposerUI();
  renderLearningActions();
  renderSuggestedResponses();
}

function closeTermPanel() {
  elements.termPanel.classList.add("hidden");
  if (elements.termPanelTitle.textContent) {
    elements.reopenTermPanel.classList.remove("hidden");
  }
  if (busy && activeTurnIndex < 0) void invoke("cancel_generation");
  termPanelTrigger?.focus();
}

function setComposerMode(mode: ComposerMode) {
  if (busy) return;
  composerMode = mode;
  if (mode === "new_problem" || mode === "idle" || mode === "socratic_response") {
    elements.prompt.value = "";
  }
  updateComposerUI();
  renderLearningActions();
  renderSuggestedResponses();
}

function updateComposerUI() {
  const configs: Record<ComposerMode, { placeholder: string; button: string }> = {
    new_problem: {
      placeholder: "Ask for guidance and a worked example...",
      button: "Work it out",
    },
    idle: {
      placeholder: "Choose a learning action below, or start a new problem.",
      button: "Send",
    },
    socratic_response: {
      placeholder: "Answer the question or describe your thinking...",
      button: "Respond",
    },
    follow_up: {
      placeholder: "Ask a question about this worked example...",
      button: "Ask",
    },
    explain_step: {
      placeholder: "Paste or describe the step you want explained...",
      button: "Explain step",
    },
    check_attempt: {
      placeholder: "Enter your answer or working so far...",
      button: "Check attempt",
    },
  };
  const config = configs[composerMode];
  if (composerMode === "new_problem") {
    config.placeholder =
      learningMode === "worked_example"
        ? "Enter a problem to solve completely, step by step..."
        : "Enter a problem for hints and a related example...";
    config.button = learningMode === "worked_example" ? "Work the example" : "Guide me";
  }
  const acceptsInput = composerMode !== "idle" && !busy;
  const collectingDetail = composerMode === "explain_step" || composerMode === "check_attempt";

  elements.prompt.placeholder = config.placeholder;
  elements.prompt.disabled = !acceptsInput;
  elements.generate.textContent = config.button;
  elements.generate.disabled = !acceptsInput;
  elements.generate.classList.toggle("hidden", composerMode === "idle");
  elements.stop.textContent = busy ? "Stop" : "Cancel";
  elements.stop.classList.toggle("hidden", !busy && !collectingDetail);
  elements.searchOption.classList.toggle("hidden", composerMode !== "new_problem" || busy);
  elements.capabilityNote.classList.toggle("hidden", composerMode !== "new_problem" || busy);
  updateModeUI();
  requestAnimationFrame(resizePrompt);
}

function resizePrompt() {
  elements.prompt.style.height = "auto";
  elements.prompt.style.height = `${Math.min(elements.prompt.scrollHeight, 132)}px`;
}

function renderLearningActions() {
  const hasCompletedTurn = learningTurns.some((turn) => turn.content.trim());
  const revealed = learningTurns.some(
    (turn) => turn.action === "reveal_solution" && turn.content.trim(),
  );
  elements.learningActions.classList.toggle("hidden", !hasCompletedTurn || busy);
  elements.learningActionList.classList.toggle(
    "hidden",
    sessionMode !== null && !supportsLearningActions(sessionMode),
  );
  elements.anotherHint.classList.toggle("hidden", revealed);
  elements.checkAttempt.classList.toggle("hidden", revealed);
  elements.revealSolution.classList.toggle("hidden", revealed);

  const actionDisabled = composerMode !== "idle" && composerMode !== "socratic_response";
  elements.anotherHint.disabled = actionDisabled;
  elements.explainStep.disabled = actionDisabled;
  elements.checkAttempt.disabled = actionDisabled;
  elements.revealSolution.disabled = actionDisabled;
}

function renderSuggestedResponses() {
  let latest: LearningTurn | undefined;
  for (let index = learningTurns.length - 1; index >= 0; index -= 1) {
    if (learningTurns[index]?.content.trim()) {
      latest = learningTurns[index];
      break;
    }
  }
  const suggestions = latest ? extractSuggestions(latest.content) : [];
  const visible =
    !busy &&
    sessionMode === "socratic" &&
    composerMode === "socratic_response" &&
    suggestions.length > 0;

  elements.suggestionList.replaceChildren(
    ...suggestions.map((suggestion) => {
      const button = document.createElement("button");
      button.type = "button";
      button.value = suggestion;
      button.textContent = suggestion;
      return button;
    }),
  );
  elements.suggestedResponses.classList.toggle("hidden", !visible);
}

function restingComposerMode(): ComposerMode {
  const revealed = learningTurns.some(
    (turn) => turn.action === "reveal_solution" && turn.content.trim(),
  );
  if (sessionMode === "socratic" && !revealed) return "socratic_response";
  if (sessionMode === "worked_example") return "follow_up";
  return "idle";
}

function loadLearningMode(): LearningMode {
  try {
    const stored = localStorage.getItem(MODE_STORAGE_KEY);
    if (isLearningMode(stored)) return stored;
  } catch {
    // Fall back to Socratic mode when persistent WebView storage is unavailable.
  }
  return "socratic";
}

function updateModeUI() {
  const locked = busy || Boolean(targetProblem);
  for (const button of elements.modePicker.querySelectorAll<HTMLButtonElement>(
    "[data-learning-mode]",
  )) {
    const selected = button.dataset.learningMode === learningMode;
    button.setAttribute("aria-pressed", String(selected));
    button.disabled = locked;
  }
  if (elements.result.classList.contains("empty")) {
    elements.emptyState.textContent =
      learningMode === "worked_example"
        ? "A complete worked solution will appear here."
        : "A guiding question will appear here.";
  }
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
