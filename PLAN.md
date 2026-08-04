# Worked Examples Vertical Slice

## Purpose

Build a focused desktop application that helps a learner approach a question
through guidance and a closely analogous worked example. The result should
explain the method without giving away the target answer, fully solve the
example, and cite sources when web search is used.

## Product Slice

The first usable slice contains one primary screen:

- Choose ChatGPT or an OpenAI-compatible endpoint.
- Sign in to ChatGPT in the system browser, or configure an endpoint and API key.
- Enter a problem or topic.
- Optionally request web search when the selected provider supports it.
- Stream a Markdown response with readable mathematics.
- Stop an in-progress response.
- Follow source links supplied by the model.

Conversation history, attachments, accounts shared between machines, automatic
updates, and a provider marketplace are deliberately out of scope.

## Technical Shape

- Tauri 2 desktop shell.
- Rust backend for OAuth, credentials, provider requests, and streaming.
- Plain TypeScript frontend rendered by the operating system WebView.
- Marked, DOMPurify, and KaTeX for safe Markdown and mathematics.
- Windows Credential Manager through `keyring-rs` for all secrets.
- A small JSON settings file for non-secret preferences.

The frontend never receives refresh tokens or API keys. It receives only account
status, non-secret settings, response events, and user-displayable errors.

## Provider Boundary

Both provider implementations produce the same application events:

1. `started`
2. zero or more `text_delta` events
3. zero or more `source` events
4. exactly one `completed`, `cancelled`, or `failed` event

### ChatGPT

ChatGPT integration follows the browser authorization flow used by Codex-capable
clients without installing or bundling Codex:

1. Generate a PKCE verifier/challenge and cryptographically random state.
2. Listen on `http://localhost:1455/auth/callback`.
3. Open the authorization URL at `https://auth.openai.com/oauth/authorize`.
4. Validate the callback state and exchange the code at `/oauth/token`.
5. Store the access token, refresh token, expiry, and account ID in Windows
   Credential Manager.
6. Refresh expired access tokens with the refresh token.
7. Send Responses requests to
   `https://chatgpt.com/backend-api/codex/responses` with bearer and
   `ChatGPT-Account-Id` headers.

This ChatGPT responses endpoint is not the public OpenAI Platform API. It can
change independently, so all endpoint-specific request and event translation
stays inside one adapter. Authentication failures clear unusable credentials
and return the user to a recoverable signed-out state.

### OpenAI-Compatible Endpoint

The endpoint adapter stores a base URL, model, and protocol preference. Secrets
remain in Credential Manager.

- `Responses` mode uses `/v1/responses`, supports streaming, and may enable the
  built-in `web_search` tool.
- `Chat Completions` mode uses `/v1/chat/completions` and does not claim web
  search support.
- The UI disables search when the configured protocol cannot provide it.

Compatibility means request-shape compatibility only. It does not imply that a
server supports OpenAI models, Responses, tools, citations, or identical event
types.

## Worked-Example Contract

Every request includes application instructions asking for:

1. A concise statement of what the learner is trying to find.
2. The concepts, formulas, facts, or other necessary ingredients.
3. Steps the learner can apply without revealing the target result.
4. A closely analogous problem with different values or details.
5. A complete step-by-step solution and check for that worked example.
6. A final hint or question that returns the learner to their own problem.
7. Sources for factual claims when search is enabled.

The vertical slice uses Markdown rather than a rigid JSON schema so partial
output remains useful while streaming.

## Security Rules

- Use OAuth Authorization Code with PKCE and validate `state` exactly.
- Bind the callback server only to localhost and stop it after success, failure,
  cancellation, or timeout.
- Never log authorization codes, access tokens, refresh tokens, or API keys.
- Store secrets only in Windows Credential Manager.
- Sanitize model-produced HTML before inserting it into the WebView.
- Permit only `http` and `https` links from rendered model output.
- Keep Tauri commands narrow; do not expose a general HTTP or shell command.

## Implementation Order

1. Scaffold Tauri, TypeScript, and test infrastructure.
2. Implement settings and secure credential storage.
3. Implement provider-neutral request/event types.
4. Implement OpenAI-compatible Responses and Chat Completions streaming.
5. Implement ChatGPT PKCE login, refresh, logout, and responses streaming.
6. Build the single-screen interface and safe Markdown/math rendering.
7. Add unit tests for OAuth state, JWT claims, SSE parsing, provider request
   construction, and settings validation.
8. Verify frontend tests, Rust tests, development startup, and a release build.
9. Document setup, usage, and known limitations.

## Acceptance Criteria

- A user can launch the app and see an actionable signed-out/configuration state.
- ChatGPT login opens the default browser and returns to a signed-in app.
- Restarting the app reuses securely stored credentials and refreshes them when
  necessary.
- A compatible endpoint can be configured without exposing its key to the
  frontend after storage.
- A prompt streams into target guidance and a readable solved analogous example.
- Long input and output remain inside independently scrollable application panels.
- Search is available only for a provider/protocol that advertises support.
- Stop cancels the active network request and leaves the UI usable.
- Invalid credentials, occupied callback ports, malformed events, network
  failures, and unsupported endpoint behavior produce recoverable errors.
- Model output is sanitized before rendering.
- Automated tests cover protocol parsing and security-sensitive pure logic.
- The project produces a Windows release build.

## Known Vertical-Slice Risks

- The ChatGPT Codex responses backend is not a stable public API and may change.
- ChatGPT subscription plans and workspace policy determine model availability.
- An OpenAI-compatible endpoint may implement only a subset of either protocol.
- Tauri requires Microsoft C++ Build Tools and WebView2 on Windows.
