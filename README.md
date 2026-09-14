# Unfold

Unfold is a Rust terminal application for guided Socratic practice and complete worked examples. It streams Markdown-formatted responses from ChatGPT subscription access or an OpenAI-compatible endpoint.

## Build And Run

Prerequisites are a current stable Rust toolchain and, on Windows, Microsoft C++ Build Tools.

```powershell
cargo run --release
```

Non-secret settings are stored in the operating system configuration directory. Existing Tauri settings under `%APPDATA%\com.workedexamples.desktop\settings.json` are read when present. OAuth credentials and compatible-provider API keys remain in Windows Credential Manager under `com.workedexamples.desktop`. Sessions and transcripts stay in memory and are never persisted.

## Keys

- `Enter`: start a problem or submit a free-form response/follow-up
- `F1`: help
- `F2`: switch Socratic/Worked Example mode before starting
- `F3`: toggle provider web search
- `Ctrl+P` / `F4`: open the searchable command palette; type to filter, navigate with arrows, `j`/`k`, `Ctrl+N`/`Ctrl+P`, or `Ctrl+J`/`Ctrl+K`, then press `Enter`
- `F5`: another Socratic hint
- `F6` / `F7`: ChatGPT browser login / logout
- `F8` / `Ctrl+M`: open the provider model catalog, then choose supported reasoning effort
- `F9`: explain the step typed in the input
- `F10`: check the attempt typed in the input
- `F12`: reveal the Socratic solution
- `Esc`: cancel generation
- `PageUp` / `PageDown` or mouse wheel: scroll; streaming follows the bottom until you scroll up
- `Ctrl+Home` / `Ctrl+End`: jump to the top / resume following streamed output
- `Shift+Left` / `Shift+Right`: extend the transcript selection
- Mouse drag: select transcript text
- `Ctrl+C` / `Ctrl+V`: copy selected text / paste into the active editor
- `Ctrl+N`: clear the in-memory session and start a new problem
- `Ctrl+Q`: quit

Configuration commands open compact pickers or editors. `Enter` explicitly saves a setting and `Esc` cancels it. Enter `CLEAR` in the API key editor to remove the stored key; an empty value leaves it unchanged. Model selection continues into its supported reasoning-effort picker. Catppuccin Latte, Frappe, Macchiato, and Mocha themes persist with the other non-secret settings.

Responses are rendered through the `daat-locus-md` Ratatui 0.30-compatible fork of `ratatui-markdown`, preserving styled Markdown and one deterministic visible-text projection for selection and copying.

Bracketed paste, focus events, and mouse capture are enabled while Unfold runs and disabled during terminal restoration. Pasted and copied transcript content passes through the same terminal-control sanitization as provider output.

## Verification

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
cargo audit
git diff --check
```

## Security

- Remote compatible endpoints require HTTPS; HTTP is accepted only on loopback.
- HTTP clients reject redirects and enforce connect, request, error-body, catalog, SSE-event, and total-stream limits.
- OAuth uses Authorization Code with PKCE, exact state validation, a five-minute localhost callback, and a safely launched browser URL without shell interpolation.
- Provider text has ANSI, OSC, C0, and C1 terminal controls removed before display. Suggestion, term, and hidden-reasoning metadata is excluded from visible text and future turns.
- Secrets stay in Windows Credential Manager. The app does not persist transcripts or log sensitive values.

ChatGPT mode uses the private Codex responses backend used by Codex-capable clients. It may change independently of the public OpenAI API.

Report suspected vulnerabilities according to [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
