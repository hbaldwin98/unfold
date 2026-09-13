# Unfold Product Plan

## Product

Unfold is one Rust binary with an immediate-mode Ratatui interface. A learner enters one target problem, fixes either Socratic or Worked Example mode for that in-memory session, and receives a streamed response. Follow-up turns retain prior visible assistant output and learner detail.

Socratic sessions support free-form answers, another hint, a focused step explanation, attempt checking, and an explicit solution reveal. Worked Example sessions produce a complete solution and accept free-form questions afterward. Web search is optional and is unavailable for Chat Completions.

The terminal has one event owner. Provider and OAuth tasks report through Tokio channels; generation IDs reject stale events. `CancellationToken` stops active streams. Ratatui initialization and restoration cover normal and returned-error paths.

## Architecture

- Root binary crate in `src/`
- `main.rs`: event loop, state, key routing, rendering, and service orchestration
- `provider.rs`: request construction, model catalogs, bounded SSE parsing, and normalized `ResponseEvent`
- `auth.rs`: ChatGPT PKCE login, callback, token exchange, and refresh
- `secrets.rs`: Windows-native keyring storage using service `com.workedexamples.desktop`
- `settings.rs`: validated non-secret configuration and legacy Tauri settings fallback
- `learning.rs`: prior-turn projection and untrusted terminal-text sanitization

No Node, browser frontend, WebView, Tauri command, installer, or transcript database remains.

## Security Boundaries

- Non-loopback endpoints must use HTTPS.
- Provider and auth clients reject redirects and enforce timeouts.
- Catalogs, provider errors, SSE lines, and complete streams have byte limits.
- Credentials never enter settings or transcript state.
- Model reasoning event families, reasoning tags, suggestion markers, and term markers are hidden.
- ANSI/OSC and C0/C1 terminal controls are removed from provider-controlled display text.
- Browser authorization uses the `open` crate rather than interpolated shell commands.

## Acceptance

- The release binary starts in an actionable problem-input state.
- All learning modes/actions, settings, auth, catalog refresh, cancellation, scrolling, new problem, help, and quit are keyboard reachable.
- Responses stream while the terminal remains responsive.
- Previous turns are supplied without hidden metadata or controls.
- Terminal state is restored after normal exit and recoverable errors.
- `fmt`, tests, warning-denying Clippy, locked release build, `cargo audit`, and whitespace checks pass in root CI.

## Limits

- Markdown and mathematics are displayed as plain terminal text.
- Source URLs are displayed but are not interactive.
- The model catalog is shown in the status text; custom model IDs are entered in Settings.
- Sessions are intentionally memory-only.
