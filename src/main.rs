mod auth;
mod learning;
mod provider;
mod secrets;
mod settings;

use std::{io, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use learning::Turn;
use provider::{GenerateRequest, LearningAction, LearningMode, ResponseEvent};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use settings::{Protocol, Provider, ReasoningEffort, Settings};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Settings,
    Help,
}

struct App {
    settings: Settings,
    screen: Screen,
    target: String,
    input: String,
    mode: LearningMode,
    web_search: bool,
    turns: Vec<Turn>,
    active: Option<(u64, CancellationToken)>,
    generation: u64,
    events: mpsc::UnboundedReceiver<AppEvent>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    status: String,
    scroll: u16,
    settings_field: usize,
    api_key: String,
}

enum AppEvent {
    Generation(u64, ResponseEvent),
    Status(String),
}

impl App {
    fn new() -> Self {
        let (event_tx, events) = mpsc::unbounded_channel();
        let (settings, status) = match settings::load() {
            Ok(value) => (value, "Ready".to_owned()),
            Err(error) => (Settings::default(), error),
        };
        let status = auth::status()
            .map(|value| {
                if value.signed_in {
                    format!(
                        "Signed in{}",
                        value
                            .email
                            .map(|email| format!(" as {email}"))
                            .unwrap_or_default()
                    )
                } else {
                    "Ready; ChatGPT signed out".to_owned()
                }
            })
            .unwrap_or(status);
        Self {
            settings,
            screen: Screen::Main,
            target: String::new(),
            input: String::new(),
            mode: LearningMode::Socratic,
            web_search: false,
            turns: Vec::new(),
            active: None,
            generation: 0,
            events,
            event_tx,
            status,
            scroll: 0,
            settings_field: 0,
            api_key: String::new(),
        }
    }

    fn start(&mut self, action: LearningAction, detail: Option<String>) {
        if self.active.is_some() {
            self.status = "Generation already active; Esc cancels".into();
            return;
        }
        if self.target.trim().is_empty() {
            self.status = "Enter a problem first".into();
            return;
        }
        self.generation += 1;
        let id = self.generation;
        let cancellation = CancellationToken::new();
        self.active = Some((id, cancellation.clone()));
        let settings = self.settings.clone();
        let request = GenerateRequest {
            target: self.target.clone(),
            mode: self.mode,
            action,
            detail: detail.clone(),
            previous_turns: learning::previous_turns(&self.turns),
            web_search: self.web_search,
        };
        self.turns.push(Turn {
            label: learning::label(action, self.mode).to_owned(),
            content: String::new(),
            detail,
        });
        let output = self.event_tx.clone();
        tokio::spawn(async move {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let forward = tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let _ = output.send(AppEvent::Generation(id, event));
                }
            });
            if let Err(message) =
                provider::generate(settings, request, tx.clone(), cancellation).await
            {
                let _ = tx.send(ResponseEvent::Failed { message });
            }
            drop(tx);
            let _ = forward.await;
        });
        self.status = "Generating... Esc cancels".into();
    }

    fn handle_response(&mut self, id: u64, event: ResponseEvent) {
        if self.active.as_ref().map(|active| active.0) != Some(id) {
            return;
        }
        match event {
            ResponseEvent::Started => self.status = "Streaming response...".into(),
            ResponseEvent::TextDelta { delta } => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.content.push_str(&learning::strip_controls(&delta));
                }
            }
            ResponseEvent::Source { title, url } => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.content.push_str(&format!("\nSource: {title} - {url}"));
                }
            }
            ResponseEvent::Completed => {
                self.active = None;
                self.status = "Complete".into();
            }
            ResponseEvent::Cancelled => {
                self.remove_empty_active_turn();
                self.active = None;
                self.status = "Cancelled".into();
            }
            ResponseEvent::Failed { message } => {
                self.remove_empty_active_turn();
                self.active = None;
                self.status = message;
            }
        }
    }

    fn remove_empty_active_turn(&mut self) {
        if self
            .turns
            .last()
            .is_some_and(|turn| turn.content.trim().is_empty())
        {
            self.turns.pop();
        }
    }

    fn submit(&mut self) {
        if self.turns.is_empty() {
            self.target = std::mem::take(&mut self.input);
            self.start(LearningAction::Initial, None);
        } else if !self.input.trim().is_empty() {
            let detail = Some(std::mem::take(&mut self.input));
            self.start(
                if self.mode == LearningMode::Socratic {
                    LearningAction::SocraticResponse
                } else {
                    LearningAction::FollowUp
                },
                detail,
            );
        }
    }

    fn action_with_input(&mut self, action: LearningAction) {
        let detail = (!self.input.trim().is_empty()).then(|| std::mem::take(&mut self.input));
        self.start(action, detail);
    }

    fn cancel(&mut self) {
        if let Some((_, token)) = &self.active {
            token.cancel();
            self.status = "Cancelling...".into();
        }
    }
    fn new_problem(&mut self) {
        self.cancel();
        self.generation += 1;
        self.active = None;
        self.target.clear();
        self.input.clear();
        self.turns.clear();
        self.scroll = 0;
        self.status = "New problem".into();
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return true;
        }
        match self.screen {
            Screen::Help => {
                self.screen = Screen::Main;
            }
            Screen::Settings => self.handle_settings_key(key),
            Screen::Main => self.handle_main_key(key),
        }
        false
    }

    fn handle_main_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::F(1) => self.screen = Screen::Help,
            KeyCode::F(2) if self.turns.is_empty() => {
                self.mode = if self.mode == LearningMode::Socratic {
                    LearningMode::WorkedExample
                } else {
                    LearningMode::Socratic
                }
            }
            KeyCode::F(3) => {
                self.web_search = !self.web_search;
                if self.settings.provider == Provider::Compatible
                    && self.settings.protocol == Protocol::ChatCompletions
                {
                    self.web_search = false;
                    self.status = "Search unavailable for Chat Completions".into();
                }
            }
            KeyCode::F(4) => self.screen = Screen::Settings,
            KeyCode::F(5) if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.start(LearningAction::AnotherHint, None)
            }
            KeyCode::F(6) | KeyCode::F(7) | KeyCode::F(8) => self.handle_account_key(key.code),
            KeyCode::F(9) if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.action_with_input(LearningAction::ExplainStep)
            }
            KeyCode::F(10) if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.action_with_input(LearningAction::CheckAttempt)
            }
            KeyCode::F(12) if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.start(LearningAction::RevealSolution, None)
            }
            KeyCode::Esc => self.cancel(),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.new_problem()
            }
            KeyCode::Char(character) => self.input.push(character),
            _ => {}
        }
    }

    fn handle_account_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::F(6) => self.login(),
            KeyCode::F(7) => self.logout(),
            KeyCode::F(8) => self.refresh_models(),
            _ => {}
        }
    }

    fn logout(&mut self) {
        self.status = match secrets::delete_oauth() {
            Ok(()) => "Signed out of ChatGPT".into(),
            Err(error) => error,
        };
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Tab => self.settings_field = (self.settings_field + 1) % 6,
            KeyCode::BackTab => self.settings_field = (self.settings_field + 5) % 6,
            KeyCode::F(2) => {
                let result = settings::save(&self.settings).and_then(|()| {
                    api_key_update(&self.api_key)
                        .map_or(Ok(()), |value| secrets::save_api_key(value))
                });
                match result {
                    Ok(()) => {
                        self.api_key.clear();
                        self.status = "Settings saved".into();
                        self.screen = Screen::Main;
                    }
                    Err(error) => self.status = error,
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => self.cycle_setting(),
            KeyCode::Backspace => self.edit_setting(None),
            KeyCode::Char(character) => self.edit_setting(Some(character)),
            _ => {}
        }
    }

    fn cycle_setting(&mut self) {
        match self.settings_field {
            0 => {
                self.settings.provider = if self.settings.provider == Provider::Chatgpt {
                    Provider::Compatible
                } else {
                    Provider::Chatgpt
                }
            }
            1 => {
                self.settings.protocol = if self.settings.protocol == Protocol::Responses {
                    Protocol::ChatCompletions
                } else {
                    Protocol::Responses
                }
            }
            4 => {
                self.settings.reasoning_effort = match self.settings.reasoning_effort {
                    ReasoningEffort::Default => ReasoningEffort::None,
                    ReasoningEffort::None => ReasoningEffort::Minimal,
                    ReasoningEffort::Minimal => ReasoningEffort::Low,
                    ReasoningEffort::Low => ReasoningEffort::Medium,
                    ReasoningEffort::Medium => ReasoningEffort::High,
                    ReasoningEffort::High => ReasoningEffort::Xhigh,
                    ReasoningEffort::Xhigh => ReasoningEffort::Max,
                    ReasoningEffort::Max => ReasoningEffort::Default,
                }
            }
            _ => {}
        }
    }

    fn edit_setting(&mut self, character: Option<char>) {
        let field = match self.settings_field {
            2 => &mut self.settings.base_url,
            3 => &mut self.settings.model,
            5 => &mut self.api_key,
            _ => return,
        };
        match character {
            Some(value) => field.push(value),
            None => {
                field.pop();
            }
        }
    }

    fn login(&mut self) {
        self.status = "Starting ChatGPT login...".into();
        let output = self.event_tx.clone();
        tokio::spawn(async move {
            let message = login_message().await;
            let _ = output.send(AppEvent::Status(message));
        });
    }

    fn refresh_models(&mut self) {
        let settings = self.settings.clone();
        let output = self.event_tx.clone();
        tokio::spawn(async move {
            let message = match provider::list_models(settings).await {
                Ok(models) => format!(
                    "Models: {}",
                    models
                        .into_iter()
                        .map(|m| m.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Err(error) => error,
            };
            let _ = output.send(AppEvent::Status(message));
        });
        self.status = "Refreshing models...".into();
    }
}

async fn login_message() -> String {
    let (url, receiver) = match auth::start_login().await {
        Ok(login) => login,
        Err(error) => return error,
    };
    if let Err(error) = open::that_detached(&url) {
        return format!("Could not open browser: {error}. Open: {url}");
    }
    login_result_message(receiver.await)
}

fn login_result_message(
    result: Result<Result<auth::AuthStatus, String>, tokio::sync::oneshot::error::RecvError>,
) -> String {
    match result {
        Ok(Ok(status)) => format!(
            "Signed in{}",
            status
                .email
                .map(|email| format!(" as {email}"))
                .unwrap_or_default()
        ),
        Ok(Err(error)) => error,
        Err(_) => "Login task ended unexpectedly".into(),
    }
}

fn api_key_update(input: &str) -> Option<Option<&str>> {
    match input.trim() {
        "" => None,
        "CLEAR" => Some(None),
        value => Some(Some(value)),
    }
}

fn render(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Help => render_help(frame),
        Screen::Settings => render_settings(frame, app),
        Screen::Main => render_main(frame, app),
    }
}

fn render_main(frame: &mut Frame, app: &App) {
    let [header, body, input, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    let mode = if app.mode == LearningMode::Socratic {
        "Socratic"
    } else {
        "Worked example"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "UNFOLD",
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {mode}  Search: {}  Model: {}",
                if app.web_search { "on" } else { "off" },
                app.settings.model
            )),
        ]))
        .block(Block::new().borders(Borders::BOTTOM)),
        header,
    );
    let mut text = String::new();
    if !app.target.is_empty() {
        text.push_str(&format!("Problem\n{}\n\n", app.target));
    }
    for turn in &app.turns {
        if let Some(detail) = &turn.detail {
            text.push_str(&format!("You\n{detail}\n\n"));
        }
        text.push_str(&format!(
            "{}\n{}\n\n",
            turn.label,
            learning::visible_content(&turn.content)
        ));
    }
    if text.is_empty() {
        text = "Type a problem below. Choose the mode before starting with F2.".into();
    }
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title("Session"))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        body,
    );
    let title = if app.turns.is_empty() {
        "Problem (Enter to start)"
    } else if app.mode == LearningMode::Socratic {
        "Your response / action detail"
    } else {
        "Follow-up question"
    };
    frame.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::bordered().title(title))
            .wrap(Wrap { trim: false }),
        input,
    );
    frame.render_widget(Paragraph::new(Text::from(vec![Line::from(app.status.as_str()), Line::from("F1 help  F2 mode  F3 search  F4 settings  F5 hint  F6 login  F7 logout  F8 models  Ctrl+N new  Ctrl+Q quit")])), footer);
}

fn render_settings(frame: &mut Frame, app: &App) {
    let fields = [
        format!("Provider: {:?}", app.settings.provider),
        format!("Protocol: {:?}", app.settings.protocol),
        format!("Base URL: {}", app.settings.base_url),
        format!("Model: {}", app.settings.model),
        format!("Reasoning: {:?}", app.settings.reasoning_effort),
        format!(
            "API key: {}",
            if app.api_key.is_empty() {
                "(unchanged; type to replace, CLEAR removes)"
            } else {
                "********"
            }
        ),
    ];
    let lines = fields
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Line::styled(
                value,
                if index == app.settings_field {
                    Style::new().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered().title("Settings - Tab field, arrows cycle, F2 save, Esc cancel"),
            )
            .wrap(Wrap { trim: false }),
        frame.area(),
    );
}

fn render_help(frame: &mut Frame) {
    frame.render_widget(Paragraph::new("UNFOLD KEYS\n\nEnter: start or submit response\nF2: switch mode before session\nF3: toggle web search\nF4: settings\nF5: another hint\nF6/F7: ChatGPT login/logout\nF8: refresh model catalog\nF9: explain step (uses typed detail)\nF10: check attempt (uses typed detail)\nF12: reveal solution\nEsc: cancel generation\nPageUp/PageDown: scroll\nCtrl+N: new problem\nCtrl+Q: quit\n\nPress any key to return.").block(Block::bordered().title("Help")).wrap(Wrap { trim: false }), frame.area());
}

async fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    loop {
        drain_events(&mut app);
        terminal.draw(|frame| render(frame, &app))?;
        if poll_for_quit(&mut app)? {
            app.cancel();
            break;
        }
    }
    Ok(())
}

fn drain_events(app: &mut App) {
    while let Ok(event) = app.events.try_recv() {
        match event {
            AppEvent::Generation(id, event) => app.handle_response(id, event),
            AppEvent::Status(status) => app.status = status,
        }
    }
}

fn poll_for_quit(app: &mut App) -> io::Result<bool> {
    if !event::poll(Duration::from_millis(50))? {
        return Ok(false);
    }
    Ok(matches!(event::read()?, Event::Key(key) if app.handle_key(key)))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal).await;
    ratatui::try_restore()?;
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        let (event_tx, events) = mpsc::unbounded_channel();
        App {
            settings: Settings::default(),
            screen: Screen::Main,
            target: String::new(),
            input: String::new(),
            mode: LearningMode::Socratic,
            web_search: false,
            turns: Vec::new(),
            active: None,
            generation: 0,
            events,
            event_tx,
            status: "Ready".into(),
            scroll: 0,
            settings_field: 0,
            api_key: String::new(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn api_key_editor_distinguishes_unchanged_replace_and_clear() {
        assert_eq!(api_key_update(""), None);
        assert_eq!(api_key_update(" new-key "), Some(Some("new-key")));
        assert_eq!(api_key_update("CLEAR"), Some(None));
    }

    #[test]
    fn response_events_update_only_the_active_generation() {
        let mut app = app();
        app.turns.push(Turn {
            label: "Answer".into(),
            content: String::new(),
            detail: None,
        });
        app.active = Some((2, CancellationToken::new()));
        app.handle_response(
            1,
            ResponseEvent::TextDelta {
                delta: "ignored".into(),
            },
        );
        app.handle_response(2, ResponseEvent::Started);
        app.handle_response(
            2,
            ResponseEvent::TextDelta {
                delta: "safe\x1b[2J".into(),
            },
        );
        app.handle_response(
            2,
            ResponseEvent::Source {
                title: "Docs".into(),
                url: "https://example.test".into(),
            },
        );
        assert!(app.turns[0].content.contains("safe\nSource: Docs"));
        app.handle_response(2, ResponseEvent::Completed);
        assert!(app.active.is_none());

        app.active = Some((3, CancellationToken::new()));
        app.turns.push(Turn {
            label: "Empty".into(),
            content: String::new(),
            detail: None,
        });
        app.handle_response(3, ResponseEvent::Cancelled);
        assert_eq!(app.status, "Cancelled");
        app.active = Some((4, CancellationToken::new()));
        app.turns.push(Turn {
            label: "Empty".into(),
            content: String::new(),
            detail: None,
        });
        app.handle_response(
            4,
            ResponseEvent::Failed {
                message: "failed".into(),
            },
        );
        assert_eq!(app.status, "failed");
    }

    #[test]
    fn main_keys_cover_navigation_editing_and_session_controls() {
        let mut app = app();
        assert!(!app.handle_key(KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )));
        assert!(!app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
        assert_eq!(app.input, "x");
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::F(1)));
        assert!(app.screen == Screen::Help);
        app.handle_key(key(KeyCode::Enter));
        assert!(app.screen == Screen::Main);
        app.handle_key(key(KeyCode::F(2)));
        assert_eq!(app.mode, LearningMode::WorkedExample);
        app.handle_key(key(KeyCode::F(3)));
        assert!(app.web_search);
        app.settings.provider = Provider::Compatible;
        app.settings.protocol = Protocol::ChatCompletions;
        app.handle_key(key(KeyCode::F(3)));
        assert!(!app.web_search);
        app.handle_key(key(KeyCode::F(4)));
        assert!(app.screen == Screen::Settings);
        app.handle_key(key(KeyCode::Esc));
        app.scroll = 5;
        app.handle_key(key(KeyCode::PageUp));
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.scroll, 5);
        app.target = "old".into();
        app.turns.push(Turn {
            label: "A".into(),
            content: "B".into(),
            detail: None,
        });
        app.mode = LearningMode::Socratic;
        app.active = Some((1, CancellationToken::new()));
        for code in [
            KeyCode::F(5),
            KeyCode::F(9),
            KeyCode::F(10),
            KeyCode::F(12),
            KeyCode::Esc,
        ] {
            app.handle_key(key(code));
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert!(app.turns.is_empty());
        assert!(app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn settings_keys_cycle_and_edit_every_editable_field() {
        let mut app = app();
        app.screen = Screen::Settings;
        app.handle_settings_key(key(KeyCode::Right));
        assert_eq!(app.settings.provider, Provider::Compatible);
        app.settings_field = 1;
        app.cycle_setting();
        assert_eq!(app.settings.protocol, Protocol::ChatCompletions);
        app.settings_field = 4;
        for expected in [
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
            ReasoningEffort::Max,
            ReasoningEffort::Default,
        ] {
            app.cycle_setting();
            assert_eq!(app.settings.reasoning_effort, expected);
        }
        for field in [2, 3, 5] {
            app.settings_field = field;
            app.edit_setting(Some('z'));
            app.edit_setting(None);
        }
        app.settings_field = 0;
        app.edit_setting(Some('x'));
        app.handle_settings_key(key(KeyCode::Tab));
        app.handle_settings_key(key(KeyCode::BackTab));
        app.settings.provider = Provider::Compatible;
        app.settings.base_url = "file:///invalid".into();
        app.handle_settings_key(key(KeyCode::F(2)));
        assert_eq!(app.status, "Endpoint must use HTTP or HTTPS");
        app.handle_settings_key(key(KeyCode::Esc));
        assert!(app.screen == Screen::Main);
    }

    #[test]
    fn renders_each_screen_and_main_session_variants() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut app = app();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        app.target = "Problem".into();
        app.turns.push(Turn {
            label: "Question".into(),
            content: "Content".into(),
            detail: Some("Detail".into()),
        });
        terminal.draw(|frame| render(frame, &app)).unwrap();
        app.mode = LearningMode::WorkedExample;
        terminal.draw(|frame| render(frame, &app)).unwrap();
        app.screen = Screen::Settings;
        terminal.draw(|frame| render(frame, &app)).unwrap();
        app.screen = Screen::Help;
        terminal.draw(|frame| render(frame, &app)).unwrap();
    }

    #[test]
    fn drains_status_and_generation_events() {
        let mut app = app();
        app.turns.push(Turn {
            label: "A".into(),
            content: String::new(),
            detail: None,
        });
        app.active = Some((7, CancellationToken::new()));
        app.event_tx
            .send(AppEvent::Generation(7, ResponseEvent::Started))
            .unwrap();
        app.event_tx
            .send(AppEvent::Status("status".into()))
            .unwrap();
        drain_events(&mut app);
        assert_eq!(app.status, "status");
    }
}
