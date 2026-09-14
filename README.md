# Unfold

Unfold is a Windows-only terminal app for guided learning. It supports two modes:

- **Socratic** asks one purposeful question at a time without revealing the target answer until requested.
- **Worked Example** produces a complete solution and accepts follow-up questions about it.

Completed and meaningful sessions are saved locally and can be continued later.

## Requirements And Run

- Windows
- Rust 1.85 or newer on the stable toolchain
- `rustfmt` and `clippy` components
- Microsoft Visual C++ (MSVC) build toolchain and a Windows SDK

Build and run the release configuration:

```powershell
cargo run --release
```

The resulting executable is `target\release\unfold.exe`. This repository does not currently publish prebuilt artifacts.

## First Run

Unfold starts in **Socratic** mode with web search off and these provider defaults:

| Setting | Default |
| --- | --- |
| Provider | ChatGPT |
| Protocol | Responses |
| Endpoint | `http://localhost:11434` |
| Model | `gpt-5.4-mini` |
| Reasoning effort | Default, omitted from requests |
| Theme | Mocha |

Press `Ctrl+P`, then choose a configuration command. ChatGPT authentication is browser sign-in only; there is no place to paste a ChatGPT API key. It requires an account entitled to the private Codex backend, and not every ChatGPT subscription is guaranteed to work.

Alternatively, choose **OpenAI-compatible** and configure an endpoint. Its bearer API key is optional, which supports local providers that do not require authentication.

Configuration pickers use arrows or `j`/`k`; text settings accept typing and paste. `Enter` saves the selected setting and returns to the palette. `Esc` cancels the setting and returns to the palette.

## Providers

| Provider | Requests | Model discovery | Web search | Catalog and effort metadata |
| --- | --- | --- | --- | --- |
| ChatGPT | Private Responses backend at `chatgpt.com/backend-api/codex/responses` | Private Codex models endpoint | Available; adds the Responses web-search tool | Uses catalog display names, visibility, priority, default effort, and supported effort choices |
| OpenAI-compatible Responses | `{base}/v1/responses` | `{base}/v1/models` | Available; adds the Responses web-search tool | Uses model IDs only; no effort choices or defaults are inferred |
| OpenAI-compatible Chat Completions | `{base}/v1/chat/completions` | `{base}/v1/models` | Unavailable and forced off | Uses model IDs only; no effort choices or defaults are inferred |

If the configured base URL already ends in `/v1`, Unfold appends only `responses`, `chat/completions`, or `models`. Otherwise it inserts `/v1/` before that path.

`F8` or `Ctrl+M` fetches the current provider's catalog. Select a model with arrows or `j`/`k` and `Enter`; press `R` in the model list to retry discovery. ChatGPT models with advertised effort choices continue to an effort picker. Compatible catalogs expose no effort metadata, so Unfold keeps a valid current effort or falls back to **Default**. **Default** omits the reasoning-effort field; explicit efforts are sent in the protocol-appropriate field.

The ChatGPT integration uses a private backend intended for Codex-capable clients. It is not the public OpenAI API and may change without notice.

## Learning Flow

1. Type a problem or topic and press `Enter`.
2. In Socratic mode, type an answer and press `Enter` to receive the next question.
3. In Worked Example mode, type a question and press `Enter` to ask about the example.
4. Press `Ctrl+N` outside the palette to save the current session and start another problem.
5. Enter the exact command `/sessions`, or choose **Browse sessions**, to restore a saved session. Continuation sends its visible prior turns using the currently configured provider and model.

`Enter` submits immediately. Use `Ctrl+Enter` to insert a newline in the main input. Bracketed paste events and `Ctrl+V` insert sanitized multiline text atomically.

Choose the mode before the first turn with `Tab`, `F2`, or **Choose learning mode**. The mode is locked after the first turn; pressing `Tab` or `F2` then reports the lock instead of changing the session. Start a new session to change it.

Socratic actions become available after the first turn:

| Key | Action | Prerequisite |
| --- | --- | --- |
| `F5` | Ask another guiding question | Socratic mode with an existing turn |
| `F9` | Explain a step | Socratic mode with an existing turn; type or paste the step first |
| `F10` | Check an attempt | Socratic mode with an existing turn; type or paste the attempt first |
| `F12` | Reveal the target solution | Socratic mode with an existing turn |

## Command Palette

Open the palette with `Ctrl+P` or `F4`. Type to filter the labels, use `Up`/`Down`, `j`/`k`, `Ctrl+N`/`Ctrl+P`, or `Ctrl+J`/`Ctrl+K` to move, and press `Enter` to run the selected command. A mouse click runs a visible command. `Esc` closes the palette.

The palette contains exactly these 14 commands:

| Label | Effect |
| --- | --- |
| Choose model and reasoning | Fetch the model catalog, then choose a model and supported effort |
| Choose provider | Choose ChatGPT or OpenAI-compatible |
| Choose protocol | Choose Responses or Chat Completions for the compatible provider |
| Edit endpoint | Edit the compatible base URL |
| Set or clear API key | Update the compatible bearer credential |
| Choose theme | Choose Latte, Frappe, Macchiato, or Mocha |
| Choose learning mode | Choose Socratic or Worked Example before the first turn |
| Toggle web search | Toggle search when the selected protocol supports it |
| Log in to ChatGPT | Start browser OAuth login |
| Log out of ChatGPT | Remove only stored ChatGPT OAuth credentials |
| New session | Save the current session, cancel generation, and start with a fresh identity |
| Browse sessions | Search saved-session metadata and restore a session for continuation |
| Help | Open the built-in key reference |
| Quit | Exit Unfold |

## Keyboard

| Key | Scope and action |
| --- | --- |
| `Enter` | Start or send immediately on the main screen; confirm a dialog elsewhere |
| `Ctrl+Enter` | Insert a newline in the main input without submitting |
| `Alt+Up` / `Alt+Down` | Scroll the wrapped main input without moving or editing text |
| `F1` | Open help from the main screen; any key closes help |
| `Tab` or `F2` | Toggle mode on the main screen before the first turn; report that mode is locked after the session starts |
| `F3` | Toggle web search on the main screen |
| `Ctrl+P` or `F4` | Open the command palette; `Ctrl+P` navigates upward while the palette is open |
| `F5` | Request another Socratic hint after the first turn |
| `F6` | Start ChatGPT browser login from the main screen, regardless of selected provider |
| `F7` | Log out ChatGPT OAuth from the main screen, regardless of selected provider |
| `F8` or `Ctrl+M` | Discover models from the main screen |
| `F9` | Explain the typed Socratic step after the first turn |
| `F10` | Check the typed Socratic attempt after the first turn |
| `F12` | Reveal the Socratic solution after the first turn |
| `Esc` | Cancel active generation and clear selection on the main screen; close a palette/model dialog; return a setting dialog to the palette without saving |
| `PageUp` / `PageDown` | Scroll five rendered rows |
| Mouse wheel | Scroll three input rows when over the input; otherwise scroll three transcript rows |
| `Ctrl+Home` / `Ctrl+End` | Jump to the top / resume following the latest output |
| `Shift+Left` / `Shift+Right` | Extend the transcript selection by one rendered character |
| `Ctrl+C` | Copy only when transcript text is selected; it does not quit the app |
| `Ctrl+V` | Paste only into the active problem/response, endpoint, model, or API-key editor |
| `Ctrl+N` | Start a new session outside the palette; move down inside the palette |
| `R` | Retry while the model catalog is open |
| `Ctrl+Q` | Save and quit; after a reported save failure, press it again to force quit without saving |

## Scrolling And Loading

Streaming follows the tail until you scroll upward. While follow-tail is off, the session title counts newly added **wrapped display rows**, not source lines or tokens. Scrolling back to the bottom or pressing `Ctrl+End` resumes follow-tail and clears the count. Rewrapping after a terminal resize updates row counts and clamps the viewport.

The main input grows from 4 to 12 rows as space and content permit while preserving transcript space. It follows the wrapped-text tail after typing, backspace, paste, or newline insertion. `Alt+Up`/`Alt+Down` and the mouse wheel over the input scroll it without changing the draft; arrows in the input title show hidden content above or below.

Before a new turn has visible text, Unfold shows a compact inline status for the active action, such as asking the next question, checking an attempt, or preparing a worked example. It does not clear or cover the transcript or learner detail. Hidden reasoning, source events, and other metadata do not dismiss it; only nonempty sanitized visible text does. The footer spinner remains active throughout generation. Completion, cancellation, and failures update the status, and an empty pending assistant turn is removed.

## Mouse And Clipboard

Unfold enables mouse capture, focus events, and bracketed paste while it runs and restores the terminal modes on exit.

- Drag with the left mouse button to select transcript text. Selection uses Unfold's rendered, sanitized Markdown projection, not the raw provider response.
- Link hit testing and mouse selection share Ratatui's word-wrap coordinate model, including explicit lines, scrolling, clipping, and common Unicode terminal widths. Complex multi-code-point graphemes may still map imprecisely because selection indexes Unicode scalar values.
- Click an explicit Markdown link or a structured source URL to open it with the operating system. Dragging selects text and never opens a link; open popups block transcript links.
- Click visible palette, setting, model, session, or effort choices to select and activate them. Model and session titles are ellipsized to fixed-height rows so narrow-window clicks remain aligned. Click a setting text editor to focus it.
- `Ctrl+C` copies the selected rendered projection only. With no selection, it reports that nothing is selected.
- `Ctrl+V` and terminal paste events affect editors only; they do not mutate choice lists or settings behind a popup.
- Paste strips terminal control sequences and normalizes CRLF and CR line endings to LF before insertion.
- Multiline bracketed paste and `Ctrl+V` are inserted as one sanitized edit; paste never changes `Enter` submission behavior.

## Markdown Rendering

Provider responses are parsed by `daat-locus-md` 0.3.6 and rendered with Ratatui 0.30. Application-owned `Problem`, `You`, and `Guide / action` headers use separate semantic styles and are not interpreted as provider Markdown. The integrated renderer supports styled headings, paragraphs, ordered and unordered lists, task items, block quotes, fenced code blocks, inline code, emphasis, strong emphasis, strikethrough, links, rules, and tables. This is the behavior of the selected library and integration, not a claim of complete CommonMark compatibility.

Explicit Markdown link destinations are extracted with `pulldown-cmark` 0.13 and validated with `url`: only well-formed `http` and `https` URLs without user information are clickable. Other schemes, malformed URLs, controls, and URLs containing credentials are rejected. Provider source events are deduplicated by URL, sanitized, stored outside response content, and rendered once in an application-owned **Sources** section. Sources are not sent back as prior assistant content. Provider-returned query strings and fragments are retained, including useful YouTube parameters, and may contain sensitive values; treat session history as sensitive. Opening uses the operating system's default handler; terminals that do not deliver mouse events cannot activate links, and Unfold does not emit OSC 8 hyperlinks.

LaTeX delimiters such as `$...$` and `$$...$$` remain literal text; Unfold does not typeset mathematics. Selection and copy use the rendered visible-text projection, including role headers and the visible Sources section, not raw Markdown source.

## Persistence

Unfold persists these non-secret settings in `settings.json` under the Windows configuration directory returned by `ProjectDirs::from("com", "workedexamples", "desktop")`:

| Field | Values |
| --- | --- |
| `provider` | `chatgpt`, `compatible` |
| `protocol` | `responses`, `chat_completions` |
| `baseUrl` | Compatible provider base URL |
| `model` | Model ID |
| `reasoningEffort` | `default`, `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` |
| `theme` | `latte`, `frappe`, `macchiato`, `mocha` |

The current `ProjectDirs` path is authoritative. Unfold reads the legacy `%APPDATA%\com.workedexamples.desktop\settings.json` only when the current settings file is absent; later saves go to the current location.

Unfold also stores `sessions.json` in the same current `ProjectDirs` configuration directory. The versioned JSON schema retains at most 100 sessions and 10 MiB total, newest first. Writes use a unique same-directory temporary file, copy the prior main file to `sessions.json.bak`, then replace the main file. The backup is retained after success. If the main file is missing or invalid and the backup is valid, history can be viewed from the backup; an invalid main file remains read-only and is never silently replaced. A failed replacement can leave the main path absent, but the backup remains loadable. Each operation removes only its own temporary files, avoiding shared temporary-name collisions between instances.

Saved sessions contain a random session ID, created and updated UNIX timestamps, the sanitized problem, learning mode, web-search choice, and visible turns. Each turn contains its label, action, learner detail, visible response content, and sanitized structured sources. Source URLs must be `http` or `https`, include a host, and contain no user information. Query strings are retained as received after validation and may contain sensitive provider-generated data. History fields are bounded; a session has at most 200 visible turns, text fields are capped, and older sessions are removed to meet the count and byte limits.

Unfold never writes API keys, OAuth credentials, provider headers, hidden reasoning or metadata, raw terminal controls, transient status/errors, active authentication details, draft input, selection, viewport, or active operations to session history. Provider, protocol, endpoint, model, and reasoning effort are not part of saved sessions and are never restored. Continuing an old session always uses the current provider and model settings.

Session history is upserted after generation completes, is cancelled, or fails with meaningful partial visible output; it is also saved before **New session**, restore, and normal quit. Streaming tokens do not cause disk writes. A save failure aborts New session or restore and leaves the active in-memory session intact. The first `Ctrl+Q` also refuses to exit and reports the failure; a deliberate second `Ctrl+Q` force-quits as an escape, so unsaved history can then be lost. `/sessions` must match the trimmed main input exactly and is never stored as a problem or learner message. The searchable picker lists only updated time, mode, and an ellipsized one-line sanitized problem title; arrows, `j`/`k`, `Ctrl+N/P/J/K`, mouse click, unmodified `Enter`, and `Esc` operate it. `Ctrl+Enter` never confirms popup choices.

ChatGPT OAuth tokens and the compatible API key are stored in Windows Credential Manager under service `com.workedexamples.desktop`. In the API-key editor, blank input leaves the existing key unchanged. The exact trimmed value `CLEAR` removes it; any other nonblank value is trimmed and saved. **Log out of ChatGPT** removes OAuth credentials only and does not remove the compatible API key.

These statements describe Unfold's own persistence. They do not control a provider's request/response retention, server logs, or the terminal application's scrollback and command history.

## Security

- Compatible endpoints must use HTTPS, except HTTP on `localhost`, any IPv4 address in `127.0.0.0/8`, or IPv6 `::1`.
- Provider and OAuth HTTP clients disable redirects and use 10-second connect timeouts. Provider requests have a 300-second timeout; OAuth requests have a 30-second timeout.
- Responses are bounded to 2 MiB for model catalogs, 16 KiB for provider error bodies, 1 MiB per SSE event line, and 20 MiB per response stream. Displayed provider error detail is capped at 500 characters. Problems and learner details are capped at 20,000 bytes each; prior context is capped at 20 turns and 80,000 bytes.
- Browser login uses Authorization Code with PKCE S256, random state with exact validation, and a five-minute callback wait on loopback port 1455. The callback listeners bind IPv4 `127.0.0.1` and, when available, IPv6 `::1`; callback reads time out after 10 seconds and are capped at 16 KiB.
- The browser URL is launched without shell interpolation. Callback HTML escapes provider text.
- ANSI, OSC, C0, and C1 terminal controls are removed from provider output and paste. Suggestion, term, and hidden-reasoning metadata is excluded from displayed text and future turns.
- ChatGPT mode depends on a private backend. Treat compatibility and availability as unstable, and review its terms and retention behavior before using sensitive material.

Report suspected vulnerabilities through GitHub private vulnerability reporting as described in [SECURITY.md](SECURITY.md). Do not publish credentials or exploit details in an issue.

## Verification

Windows CI runs exactly these five commands:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
cargo audit
```

Install `cargo-audit` locally before running the final command, for example with `cargo install cargo-audit --locked`.

Run this additional local whitespace check before committing:

```powershell
git diff --check
```

`git diff --check` is a local repository check and is not one of the five CI commands.

## License

MIT. See [LICENSE](LICENSE).
