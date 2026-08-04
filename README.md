# Worked Examples

A focused Windows desktop application that helps a learner approach a question
through guidance and a fully solved analogous example.

The app supports:

- ChatGPT subscription access through browser authorization.
- OpenAI-compatible Responses and Chat Completions endpoints.
- Optional provider-native web search.
- Streaming Markdown with KaTeX mathematics.
- Source links, cancellation, and responsive desktop/mobile-width layouts.
- A bounded application viewport with independently scrolling input and output.
- Progressive hints that preserve earlier guidance.
- Focused step explanations and feedback on the learner's attempt.
- An explicit target-solution reveal rather than an automatic answer.

See [PLAN.md](PLAN.md) for the architecture, security rules, scope, and
acceptance criteria for this vertical slice.

## Use The App

1. Open **Worked Examples**.
2. Select the connection button in the upper-right corner.
3. Choose a provider.
4. Save the connection.
5. Enter a problem and select **Work it out**.
6. Continue with **Another hint**, **Explain a step**, or **Check my attempt**.
7. Use **Show solution** only when you intentionally want the target answer.

The active learning session is kept in memory while the app is open. Select
**New problem** to clear its turns and begin again; persistent session history
is not part of this milestone.

### ChatGPT

Select **Sign in**. The app opens OpenAI authorization in the default browser
and listens for the callback on `http://localhost:1455/auth/callback`.

The app uses OAuth Authorization Code with PKCE. Access and refresh tokens are
stored as one secret in Windows Credential Manager and are never sent to the
frontend. The app refreshes an expired access token automatically.

No Codex executable is installed, bundled, or launched.

### Compatible Endpoint

Configure:

- **Responses** for `/v1/responses`, streaming, and optional provider-native
  `web_search`.
- **Chat Completions** for `/v1/chat/completions` and streaming text without a
  claimed search capability.
- A base URL, model identifier, and optional API key.

The API key is stored in Windows Credential Manager. To remove a stored key,
enter `CLEAR` in the API key field and save.

An OpenAI-compatible endpoint may implement only part of either protocol. The
app reports unsupported request shapes and events as provider errors.

## Development

Prerequisites:

- Node.js 20 or newer.
- Rust with the `x86_64-pc-windows-msvc` toolchain.
- Microsoft C++ Build Tools.
- WebView2 Runtime.

Install and run:

```powershell
npm install
npm run tauri dev
```

Run verification:

```powershell
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build
```

Release artifacts are written to:

- `src-tauri/target/release/worked-examples.exe`
- `src-tauri/target/release/bundle/msi/Worked Examples_0.2.0_x64_en-US.msi`
- `src-tauri/target/release/bundle/nsis/Worked Examples_0.2.0_x64-setup.exe`

## Security

- OAuth callbacks are bound only to localhost, expire after five minutes, and
  require an exact random `state` value.
- Secrets are handled only by Rust and Windows Credential Manager.
- Model-produced HTML is sanitized with DOMPurify.
- Rendered links are restricted to HTTP and HTTPS and open in the system
  browser.
- The WebView has a restrictive Content Security Policy and cannot make model
  provider requests directly.
- The application exposes no general shell or arbitrary HTTP Tauri command.

## Stability Notice

ChatGPT subscription mode sends authenticated requests to the ChatGPT Codex
responses backend used by Codex-capable clients. This is not the public OpenAI
Platform API and may change without the compatibility guarantees of the public
Responses API. The implementation is intentionally isolated in
`src-tauri/src/provider.rs` and `src-tauri/src/auth.rs`.

Model availability depends on the signed-in subscription, workspace policy,
and OpenAI's current Codex model catalog. If this integration changes, the
OpenAI-compatible endpoint remains an independent provider path.
