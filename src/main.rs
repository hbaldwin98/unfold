mod auth;
mod learning;
mod provider;
mod secrets;
mod settings;

use std::{io, time::Duration};

use arboard::Clipboard;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use learning::Turn;
use provider::{
    CredentialOverride, GenerateRequest, LearningAction, LearningMode, ModelOption, ResponseEvent,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use ratatui_markdown::{
    markdown::MarkdownRenderer,
    theme::{Generation, RichTextTheme},
};
use settings::{Protocol, Provider, ReasoningEffort, Settings, Theme};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Main,
    Settings,
    Help,
}

#[derive(Clone)]
struct SettingsDraft {
    values: Settings,
    api_key: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Viewport {
    offset: usize,
    follow_tail: bool,
    unseen: usize,
}

impl Viewport {
    fn new() -> Self {
        Self {
            follow_tail: true,
            ..Self::default()
        }
    }

    fn clamp(&mut self, content: usize, height: usize) {
        let max = content.saturating_sub(height);
        if self.follow_tail {
            self.offset = max;
            self.unseen = 0;
        } else {
            self.offset = self.offset.min(max);
            if self.offset == max {
                self.follow_tail = true;
                self.unseen = 0;
            }
        }
    }

    fn scroll(&mut self, delta: isize, content: usize, height: usize) {
        let max = content.saturating_sub(height);
        self.offset = self.offset.saturating_add_signed(delta).min(max);
        self.follow_tail = self.offset == max;
        if self.follow_tail {
            self.unseen = 0;
        }
    }

    fn appended(&mut self, lines: usize) {
        if !self.follow_tail && lines > 0 {
            self.unseen = self.unseen.saturating_add(lines);
        }
    }
}

enum Popup {
    Palette {
        query: String,
        state: ListState,
    },
    Setting {
        command: Command,
        draft: SettingsDraft,
        input: String,
        state: ListState,
    },
    Models {
        request_id: u64,
        state: ListState,
        models: Vec<ModelOption>,
        loading: bool,
        error: Option<String>,
    },
    Effort {
        model: ModelOption,
        state: ListState,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Model,
    Provider,
    Protocol,
    Endpoint,
    ApiKey,
    Theme,
    LearningMode,
    WebSearch,
    Login,
    Logout,
    NewSession,
    Help,
    Quit,
}

impl Command {
    const ALL: [Self; 13] = [
        Self::Model,
        Self::Provider,
        Self::Protocol,
        Self::Endpoint,
        Self::ApiKey,
        Self::Theme,
        Self::LearningMode,
        Self::WebSearch,
        Self::Login,
        Self::Logout,
        Self::NewSession,
        Self::Help,
        Self::Quit,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Model => "Choose model and reasoning",
            Self::Provider => "Choose provider",
            Self::Protocol => "Choose protocol",
            Self::Endpoint => "Edit endpoint",
            Self::ApiKey => "Set or clear API key",
            Self::Theme => "Choose theme",
            Self::LearningMode => "Choose learning mode",
            Self::WebSearch => "Toggle web search",
            Self::Login => "Log in to ChatGPT",
            Self::Logout => "Log out of ChatGPT",
            Self::NewSession => "New session",
            Self::Help => "Help",
            Self::Quit => "Quit",
        }
    }
}

enum AppEvent {
    Generation(u64, ResponseEvent),
    Models(u64, Result<Vec<ModelOption>, String>),
    Login(u64, Result<secrets::OAuthCredentials, String>),
}

struct App {
    settings: Settings,
    settings_draft: Option<SettingsDraft>,
    screen: Screen,
    popup: Option<Popup>,
    target: String,
    input: String,
    mode: LearningMode,
    web_search: bool,
    turns: Vec<Turn>,
    active: Option<(u64, CancellationToken)>,
    generation: u64,
    tick: u64,
    operation_id: u64,
    login_id: Option<u64>,
    events: mpsc::UnboundedReceiver<AppEvent>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    status: String,
    viewport: Viewport,
    settings_field: usize,
    body_area: Rect,
    popup_area: Rect,
    transcript_lines: usize,
    selection: Option<(usize, usize)>,
    selection_anchor: usize,
    quit_requested: bool,
}

impl App {
    fn new() -> Self {
        let (event_tx, events) = mpsc::unbounded_channel();
        let (settings, load_error) = match settings::load() {
            Ok(value) => (value, None),
            Err(error) => (Settings::default(), Some(error)),
        };
        let auth_status = auth::status().ok().map(|value| {
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
        });
        Self {
            settings,
            settings_draft: None,
            screen: Screen::Main,
            popup: None,
            target: String::new(),
            input: String::new(),
            mode: LearningMode::Socratic,
            web_search: false,
            turns: Vec::new(),
            active: None,
            generation: 0,
            tick: 0,
            operation_id: 0,
            login_id: None,
            events,
            event_tx,
            status: load_error.or(auth_status).unwrap_or_else(|| "Ready".into()),
            viewport: Viewport::new(),
            settings_field: 0,
            body_area: Rect::default(),
            popup_area: Rect::default(),
            transcript_lines: 0,
            selection: None,
            selection_anchor: 0,
            quit_requested: false,
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
        self.status = "Connecting to model...".into();
        self.viewport.follow_tail = true;
    }

    fn handle_response(&mut self, id: u64, event: ResponseEvent) {
        if self.active.as_ref().map(|active| active.0) != Some(id) {
            return;
        }
        match event {
            ResponseEvent::Started => self.status = "Streaming response...".into(),
            ResponseEvent::TextDelta { delta } => {
                let before = self.transcript_display_rows();
                if let Some(turn) = self.turns.last_mut() {
                    turn.content.push_str(&learning::strip_controls(&delta));
                }
                let added = self.transcript_display_rows().saturating_sub(before);
                self.viewport.appended(added);
            }
            ResponseEvent::Source { title, url } => {
                let before = self.transcript_display_rows();
                if let Some(turn) = self.turns.last_mut() {
                    turn.content.push_str(&format!("\nSource: {title} - {url}"));
                }
                let added = self.transcript_display_rows().saturating_sub(before);
                self.viewport.appended(added);
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
            .is_some_and(|turn| learning::visible_content(&turn.content).trim().is_empty())
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
            self.status = "Cancelling response...".into();
        }
    }

    #[cfg(test)]
    fn open_settings(&mut self) {
        self.settings_draft = Some(SettingsDraft {
            values: self.settings.clone(),
            api_key: String::new(),
        });
        self.settings_field = 0;
        self.screen = Screen::Settings;
    }

    fn open_palette(&mut self) {
        let mut state = ListState::default();
        state.select(Some(0));
        self.popup = Some(Popup::Palette {
            query: String::new(),
            state,
        });
    }

    fn filtered_commands(query: &str) -> Vec<Command> {
        let query = query.to_ascii_lowercase();
        Command::ALL
            .into_iter()
            .filter(|command| command.label().to_ascii_lowercase().contains(&query))
            .collect()
    }

    fn open_setting(&mut self, command: Command) {
        let mut state = ListState::default();
        state.select(Some(match command {
            Command::Provider => usize::from(self.settings.provider == Provider::Compatible),
            Command::Protocol => usize::from(self.settings.protocol == Protocol::ChatCompletions),
            Command::Theme => match self.settings.theme {
                Theme::Latte => 0,
                Theme::Frappe => 1,
                Theme::Macchiato => 2,
                Theme::Mocha => 3,
            },
            Command::LearningMode => usize::from(self.mode == LearningMode::WorkedExample),
            _ => 0,
        }));
        let input = match command {
            Command::Endpoint => self.settings.base_url.clone(),
            Command::ApiKey => String::new(),
            _ => String::new(),
        };
        self.popup = Some(Popup::Setting {
            command,
            draft: SettingsDraft {
                values: self.settings.clone(),
                api_key: String::new(),
            },
            input,
            state,
        });
    }

    fn execute_command(&mut self, command: Command) {
        match command {
            Command::Model => {
                self.popup = None;
                self.refresh_models();
            }
            Command::Provider
            | Command::Protocol
            | Command::Endpoint
            | Command::ApiKey
            | Command::Theme
            | Command::LearningMode => self.open_setting(command),
            Command::WebSearch => {
                self.toggle_search();
                self.popup = None;
            }
            Command::Login => {
                self.popup = None;
                self.login();
            }
            Command::Logout => {
                self.popup = None;
                self.logout();
            }
            Command::NewSession => {
                self.popup = None;
                self.new_problem();
            }
            Command::Help => {
                self.popup = None;
                self.screen = Screen::Help;
            }
            Command::Quit => self.quit_requested = true,
        }
    }

    fn confirm_setting(&mut self) {
        let Some(Popup::Setting {
            command,
            mut draft,
            input,
            state,
        }) = self.popup.take()
        else {
            return;
        };
        let selected = state.selected();
        let selected_mode = selected_learning_mode(command, selected);
        let editor_input = input.clone();
        apply_setting_draft(&mut draft, command, input, selected);
        if self.settings_draft.is_some() {
            self.settings_draft = Some(draft);
            if let Some(mode) = selected_mode {
                self.mode = mode;
            }
            self.reconcile_web_search();
            self.status = "Setting updated in draft".into();
            return;
        }
        match save_settings_transaction(
            &self.settings,
            &draft.values,
            api_key_update(&draft.api_key),
            settings::save,
            secrets::save_api_key,
        ) {
            Ok(()) => {
                self.settings = draft.values;
                if let Some(mode) = selected_mode {
                    self.mode = mode;
                }
                self.reconcile_web_search();
                self.status = "Setting saved".into();
                self.open_palette();
            }
            Err(error) => {
                self.status = error;
                self.popup = Some(Popup::Setting {
                    command,
                    draft,
                    input: editor_input,
                    state,
                });
            }
        }
    }

    fn close_settings(&mut self) {
        self.settings_draft = None;
        self.screen = Screen::Main;
        self.popup = None;
    }

    fn save_settings(&mut self) {
        let Some(draft) = self.settings_draft.clone() else {
            return;
        };
        let result = save_settings_transaction(
            &self.settings,
            &draft.values,
            api_key_update(&draft.api_key),
            settings::save,
            secrets::save_api_key,
        );
        match result {
            Ok(()) => {
                self.settings = draft.values;
                self.status = "Settings saved".into();
                self.close_settings();
            }
            Err(error) => self.status = error,
        }
    }

    fn refresh_models(&mut self) {
        self.operation_id += 1;
        let request_id = self.operation_id;
        let draft = self.settings_draft.as_ref();
        let settings = draft.map_or_else(|| self.settings.clone(), |draft| draft.values.clone());
        let credential = draft.map_or(CredentialOverride::Unchanged, |draft| {
            credential_override(&draft.api_key)
        });
        let mut state = ListState::default();
        state.select(Some(0));
        self.popup = Some(Popup::Models {
            request_id,
            state,
            models: Vec::new(),
            loading: true,
            error: None,
        });
        self.status = "Loading model catalog...".into();
        let output = self.event_tx.clone();
        tokio::spawn(async move {
            let result = provider::list_models(settings, credential).await;
            let _ = output.send(AppEvent::Models(request_id, result));
        });
    }

    fn handle_models(&mut self, id: u64, result: Result<Vec<ModelOption>, String>) {
        let Some(Popup::Models {
            request_id,
            state,
            models,
            loading,
            error,
        }) = &mut self.popup
        else {
            return;
        };
        if *request_id != id {
            return;
        }
        *loading = false;
        match result {
            Ok(values) => {
                *models = values;
                state.select((!models.is_empty()).then_some(0));
                self.status = if models.is_empty() {
                    "Provider returned no models"
                } else {
                    "Choose a model"
                }
                .into();
            }
            Err(message) => {
                *error = Some(message.clone());
                self.status = message;
            }
        }
    }

    fn select_model(&mut self) {
        let model = match &self.popup {
            Some(Popup::Models {
                state,
                models,
                loading: false,
                error: None,
                ..
            }) => state
                .selected()
                .and_then(|index| models.get(index))
                .cloned(),
            _ => None,
        };
        let Some(model) = model else { return };
        if model.reasoning_efforts.is_empty() {
            self.confirm_model(
                &model,
                reconcile_effort(&model, self.current_settings().reasoning_effort),
            );
        } else {
            let effort = reconcile_effort(&model, self.current_settings().reasoning_effort);
            let selected = effort_index(&model, effort);
            let mut state = ListState::default();
            state.select(Some(selected));
            self.popup = Some(Popup::Effort { model, state });
        }
    }

    fn confirm_effort(&mut self) {
        let choice = match &self.popup {
            Some(Popup::Effort { model, state }) => state.selected().map(|index| {
                let effort = if index == 0 {
                    ReasoningEffort::Default
                } else {
                    parse_effort(&model.reasoning_efforts[index - 1].effort)
                        .unwrap_or(ReasoningEffort::Default)
                };
                (model.clone(), effort)
            }),
            _ => None,
        };
        if let Some((model, effort)) = choice {
            self.confirm_model(&model, effort);
        }
    }

    fn confirm_model(&mut self, model: &ModelOption, effort: ReasoningEffort) {
        if let Some(draft) = &mut self.settings_draft {
            draft.values.model = model.id.clone();
            draft.values.reasoning_effort = effort;
        } else {
            self.settings.model = model.id.clone();
            self.settings.reasoning_effort = effort;
            if let Err(error) = settings::save(&self.settings) {
                self.status = error;
                return;
            }
        }
        self.popup = None;
        self.status = format!("Selected {} ({effort:?})", model.name);
    }

    fn current_settings(&self) -> &Settings {
        self.settings_draft
            .as_ref()
            .map_or(&self.settings, |draft| &draft.values)
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Mouse(mouse) => {
                self.handle_mouse(mouse);
                false
            }
            Event::Paste(text) => {
                self.insert_paste(&text);
                false
            }
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') if self.popup.is_none() => {
                    self.open_palette();
                    return false;
                }
                KeyCode::Char('q') => return true,
                KeyCode::Char('c') if self.selection.is_some() => {
                    self.copy_selection();
                    return false;
                }
                KeyCode::Char('c') => {
                    self.status = "Nothing selected; drag or use Shift+arrows first".into();
                    return false;
                }
                KeyCode::Char('v') => {
                    self.paste_clipboard();
                    return false;
                }
                _ => {}
            }
        }
        if self.popup.is_some() {
            self.handle_popup_key(key);
            return self.quit_requested;
        }
        match self.screen {
            Screen::Help => self.screen = Screen::Main,
            Screen::Settings => self.handle_settings_key(key),
            Screen::Main => self.handle_main_key(key),
        }
        false
    }

    fn handle_main_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            self.extend_selection(if key.code == KeyCode::Left { -1 } else { 1 });
            return;
        }
        if let KeyCode::F(number) = key.code {
            self.handle_main_function_key(number);
            return;
        }
        match key.code {
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh_models()
            }
            KeyCode::Esc => {
                self.cancel();
                self.selection = None;
            }
            KeyCode::PageUp => self.scroll(-5),
            KeyCode::PageDown => self.scroll(5),
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.viewport.offset = 0;
                self.viewport.follow_tail = false;
            }
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.viewport.follow_tail = true;
                self.viewport
                    .clamp(self.transcript_lines, self.body_height());
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.new_problem()
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(character)
            }
            _ => {}
        }
    }

    fn handle_main_function_key(&mut self, number: u8) {
        match number {
            1 => self.screen = Screen::Help,
            2 if self.turns.is_empty() => {
                self.mode = if self.mode == LearningMode::Socratic {
                    LearningMode::WorkedExample
                } else {
                    LearningMode::Socratic
                }
            }
            3 => self.toggle_search(),
            4 => self.open_palette(),
            5 if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.start(LearningAction::AnotherHint, None)
            }
            9 if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.action_with_input(LearningAction::ExplainStep)
            }
            10 if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.action_with_input(LearningAction::CheckAttempt)
            }
            12 if self.mode == LearningMode::Socratic && !self.turns.is_empty() => {
                self.start(LearningAction::RevealSolution, None)
            }
            6..=8 => self.handle_service_function_key(number),
            _ => {}
        }
    }

    fn handle_service_function_key(&mut self, number: u8) {
        match number {
            6 => self.login(),
            7 => self.logout(),
            8 => self.refresh_models(),
            _ => {}
        }
    }

    fn handle_popup_key(&mut self, key: KeyEvent) {
        if matches!(self.popup, Some(Popup::Palette { .. })) {
            self.handle_palette_key(key);
        } else if matches!(self.popup, Some(Popup::Setting { .. })) {
            self.handle_setting_key(key);
        } else {
            self.handle_model_popup_key(key);
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        if let Some(delta) = palette_navigation(key) {
            self.move_popup(delta);
            return;
        }
        match key.code {
            KeyCode::Esc => self.popup = None,
            KeyCode::Enter => {
                if let Some(command) = self.selected_palette_command() {
                    self.execute_command(command);
                }
            }
            KeyCode::Backspace => self.edit_palette_query(None),
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_palette_query(Some(character));
            }
            _ => {}
        }
    }

    fn selected_palette_command(&self) -> Option<Command> {
        match &self.popup {
            Some(Popup::Palette { query, state }) => state
                .selected()
                .and_then(|index| Self::filtered_commands(query).get(index).copied()),
            _ => None,
        }
    }

    fn edit_palette_query(&mut self, character: Option<char>) {
        if let Some(Popup::Palette { query, state }) = &mut self.popup {
            if let Some(character) = character {
                query.push(character);
            } else {
                query.pop();
            }
            state.select(Some(0));
        }
    }

    fn handle_setting_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.open_palette(),
            KeyCode::Enter => self.confirm_setting(),
            KeyCode::Up | KeyCode::Char('k') => self.move_popup(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_popup(1),
            KeyCode::Backspace => self.edit_popup_setting(None),
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_popup_setting(Some(character));
            }
            _ => {}
        }
    }

    fn edit_popup_setting(&mut self, character: Option<char>) {
        if let Some(Popup::Setting {
            command: Command::Endpoint | Command::ApiKey,
            input,
            ..
        }) = &mut self.popup
        {
            if let Some(character) = character {
                input.push(character);
            } else {
                input.pop();
            }
        }
    }

    fn handle_model_popup_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.popup = None,
            KeyCode::Up | KeyCode::Char('k') => self.move_popup(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_popup(1),
            KeyCode::Enter if matches!(self.popup, Some(Popup::Effort { .. })) => {
                self.confirm_effort()
            }
            KeyCode::Enter => self.select_model(),
            KeyCode::Char('r') if matches!(self.popup, Some(Popup::Models { .. })) => {
                self.refresh_models()
            }
            _ => {}
        }
    }

    fn move_popup(&mut self, delta: isize) {
        match &mut self.popup {
            Some(Popup::Palette { query, state }) => {
                move_list(state, Self::filtered_commands(query).len(), delta)
            }
            Some(Popup::Setting { command, state, .. }) => {
                move_list(state, setting_choices(*command).len(), delta)
            }
            Some(Popup::Models { state, models, .. }) => move_list(state, models.len(), delta),
            Some(Popup::Effort { state, model }) => {
                move_list(state, model.reasoning_efforts.len() + 1, delta)
            }
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Tab => self.settings_field = (self.settings_field + 1) % 7,
            KeyCode::BackTab => self.settings_field = (self.settings_field + 6) % 7,
            KeyCode::Enter if self.settings_field == 3 => self.refresh_models(),
            KeyCode::Enter | KeyCode::F(2) => self.save_settings(),
            KeyCode::Left | KeyCode::Up => self.cycle_setting(-1),
            KeyCode::Right | KeyCode::Down => self.cycle_setting(1),
            KeyCode::Backspace => self.edit_setting(None),
            KeyCode::Char(character) => self.edit_setting(Some(character)),
            _ => {}
        }
    }

    fn cycle_setting(&mut self, direction: isize) {
        let field = self.settings_field;
        let Some(draft) = &mut self.settings_draft else {
            return;
        };
        match field {
            0 => {
                draft.values.provider = if draft.values.provider == Provider::Chatgpt {
                    Provider::Compatible
                } else {
                    Provider::Chatgpt
                };
            }
            1 => {
                draft.values.protocol = if draft.values.protocol == Protocol::Responses {
                    Protocol::ChatCompletions
                } else {
                    Protocol::Responses
                };
            }
            4 => {
                draft.values.reasoning_effort =
                    cycle_effort(draft.values.reasoning_effort, direction)
            }
            5 => draft.values.theme = cycle_theme(draft.values.theme, direction),
            _ => {}
        }
        self.reconcile_web_search();
    }

    fn edit_setting(&mut self, character: Option<char>) {
        let Some(draft) = &mut self.settings_draft else {
            return;
        };
        let field = match self.settings_field {
            2 => &mut draft.values.base_url,
            3 => &mut draft.values.model,
            6 => &mut draft.api_key,
            _ => return,
        };
        if let Some(value) = character {
            field.push(value);
        } else {
            field.pop();
        }
    }

    fn insert_paste(&mut self, text: &str) {
        let clean = sanitize_paste(text);
        if let Some(Popup::Setting {
            command: Command::Endpoint | Command::ApiKey,
            input,
            ..
        }) = &mut self.popup
        {
            input.push_str(&clean);
            return;
        }
        if self.popup.is_some() {
            return;
        }
        if self.screen == Screen::Settings {
            for character in clean.chars() {
                self.edit_setting(Some(character));
            }
        } else if self.screen == Screen::Main && self.popup.is_none() {
            self.input.push_str(&clean);
        }
    }

    fn paste_clipboard(&mut self) {
        self.paste_clipboard_with(|| Clipboard::new().and_then(|mut value| value.get_text()));
    }

    fn paste_clipboard_with(&mut self, read: impl FnOnce() -> Result<String, arboard::Error>) {
        match read() {
            Ok(text) => self.insert_paste(&text),
            Err(error) => self.status = format!("Clipboard paste failed: {error}"),
        }
    }

    fn copy_selection(&mut self) {
        let Some(selected) = self.selected_text() else {
            return;
        };
        if selected.is_empty() {
            self.status = "Selection is empty".into();
            return;
        }
        match Clipboard::new().and_then(|mut value| value.set_text(selected)) {
            Ok(()) => self.status = "Selection copied".into(),
            Err(error) => self.status = format!("Clipboard copy failed: {error}"),
        }
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection?;
        Some(
            self.transcript_rendered_plain()
                .chars()
                .skip(start.min(end))
                .take(start.abs_diff(end))
                .collect(),
        )
    }

    fn extend_selection(&mut self, delta: isize) {
        let length = self.transcript_rendered_plain().chars().count();
        let current = self.selection.map_or(self.selection_anchor, |(_, end)| end);
        let next = current.saturating_add_signed(delta).min(length);
        if self.selection.is_none() {
            self.selection_anchor = current;
        }
        self.selection = Some((self.selection_anchor, next));
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.popup.is_some() {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.handle_popup_click(mouse.column, mouse.row);
            }
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll(-3),
            MouseEventKind::ScrollDown => self.scroll(3),
            MouseEventKind::Down(MouseButton::Left)
                if self.body_area.contains((mouse.column, mouse.row).into()) =>
            {
                let index = self.mouse_text_index(mouse.column, mouse.row);
                self.selection_anchor = index;
                self.selection = Some((index, index));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let index = self.mouse_text_index(mouse.column, mouse.row);
                self.selection = Some((self.selection_anchor, index));
            }
            _ => {}
        }
    }

    fn handle_popup_click(&mut self, column: u16, row: u16) {
        let area = self.popup_area;
        if !area.contains((column, row).into()) {
            return;
        }
        if row <= area.y || row >= area.bottom().saturating_sub(1) {
            return;
        }
        let item_row = usize::from(row - area.y - 1);
        match &mut self.popup {
            Some(Popup::Palette { query, state })
                if state.offset() + item_row < Self::filtered_commands(query).len() =>
            {
                let index = state.offset() + item_row;
                state.select(Some(index));
                let command = Self::filtered_commands(query)[index];
                self.execute_command(command);
            }
            Some(Popup::Setting { command, state, .. })
                if state.offset() + item_row < setting_choices(*command).len() =>
            {
                state.select(Some(state.offset() + item_row));
                self.confirm_setting();
            }
            Some(Popup::Models { state, models, .. })
                if state.offset() + item_row / 2 < models.len() =>
            {
                let index = state.offset() + item_row / 2;
                state.select(Some(index));
                self.select_model();
            }
            Some(Popup::Effort { state, model })
                if state.offset() + item_row <= model.reasoning_efforts.len() =>
            {
                state.select(Some(state.offset() + item_row));
                self.confirm_effort();
            }
            _ => {}
        }
    }

    fn mouse_text_index(&self, column: u16, row: u16) -> usize {
        // Mouse cells are an approximation for wide glyphs, but indexes always target rendered text.
        let width = usize::from(self.body_area.width.saturating_sub(2)).max(1);
        let line = self.viewport.offset + usize::from(row.saturating_sub(self.body_area.y + 1));
        line.saturating_mul(width)
            .saturating_add(usize::from(column.saturating_sub(self.body_area.x + 1)))
            .min(self.transcript_rendered_plain().chars().count())
    }

    fn scroll(&mut self, delta: isize) {
        self.viewport
            .scroll(delta, self.transcript_lines, self.body_height());
    }

    fn body_height(&self) -> usize {
        usize::from(self.body_area.height.saturating_sub(2))
    }

    fn toggle_search(&mut self) {
        self.web_search = !self.web_search;
        if self.settings.provider == Provider::Compatible
            && self.settings.protocol == Protocol::ChatCompletions
        {
            self.web_search = false;
            self.status = "Search unavailable for Chat Completions".into();
        }
    }

    fn new_problem(&mut self) {
        self.cancel();
        self.generation += 1;
        self.active = None;
        self.target.clear();
        self.input.clear();
        self.turns.clear();
        self.viewport = Viewport::new();
        self.selection = None;
        self.status = "New problem".into();
    }

    fn login(&mut self) {
        self.operation_id += 1;
        let id = self.operation_id;
        self.login_id = Some(id);
        self.status = "Starting ChatGPT login...".into();
        let output = self.event_tx.clone();
        tokio::spawn(async move {
            let _ = output.send(AppEvent::Login(id, login_result().await));
        });
    }

    fn logout(&mut self) {
        self.login_id = None;
        self.status = match secrets::delete_oauth() {
            Ok(()) => "Signed out of ChatGPT".into(),
            Err(error) => error,
        };
    }

    fn reconcile_web_search(&mut self) {
        if self.current_settings().provider == Provider::Compatible
            && self.current_settings().protocol == Protocol::ChatCompletions
        {
            self.web_search = false;
        }
    }

    fn transcript_source(&self) -> String {
        let mut text = String::new();
        if !self.target.is_empty() {
            text.push_str(&format!(
                "Problem\n{}\n\n",
                learning::strip_controls(&self.target)
            ));
        }
        for (index, turn) in self.turns.iter().enumerate() {
            if let Some(detail) = &turn.detail {
                text.push_str(&format!("You\n{}\n\n", learning::strip_controls(detail)));
            }
            let visible = learning::visible_content(&turn.content);
            if self.active.is_some() && index + 1 == self.turns.len() && visible.trim().is_empty() {
                continue;
            }
            text.push_str(&format!("{}\n{}\n\n", turn.label, visible));
        }
        if text.is_empty() {
            "Type a problem below. Choose the mode before starting with F2.".into()
        } else {
            text
        }
    }

    fn transcript_rendered(&self) -> Text<'static> {
        markdown_text(
            &self.transcript_source(),
            palette(self.current_settings().theme),
        )
    }

    fn transcript_rendered_plain(&self) -> String {
        text_plain(&markdown_text(
            &self.transcript_source(),
            palette(self.current_settings().theme),
        ))
    }

    fn transcript_display_rows(&self) -> usize {
        rendered_rows(self.transcript_rendered(), self.body_area.width)
    }

    fn persist_on_quit(&self) -> Result<(), String> {
        self.persist_on_quit_with(settings::save)
    }

    fn persist_on_quit_with(
        &self,
        save: impl FnOnce(&Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        save(&self.settings)
    }
}

fn move_list(state: &mut ListState, length: usize, delta: isize) {
    if length == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0);
    state.select(Some(current.saturating_add_signed(delta).min(length - 1)));
}

fn palette_navigation(key: KeyEvent) -> Option<isize> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, control) {
        (KeyCode::Up | KeyCode::Char('k'), false)
        | (KeyCode::Char('p') | KeyCode::Char('k'), true) => Some(-1),
        (KeyCode::Down | KeyCode::Char('j'), false)
        | (KeyCode::Char('n') | KeyCode::Char('j'), true) => Some(1),
        _ => None,
    }
}

fn setting_choices(command: Command) -> &'static [&'static str] {
    match command {
        Command::Provider => &["ChatGPT", "OpenAI-compatible"],
        Command::Protocol => &["Responses", "Chat Completions"],
        Command::Theme => &["Latte", "Frappe", "Macchiato", "Mocha"],
        Command::LearningMode => &["Socratic", "Worked example"],
        _ => &[],
    }
}

fn selected_learning_mode(command: Command, selected: Option<usize>) -> Option<LearningMode> {
    (command == Command::LearningMode).then_some(if selected == Some(1) {
        LearningMode::WorkedExample
    } else {
        LearningMode::Socratic
    })
}

fn apply_setting_draft(
    draft: &mut SettingsDraft,
    command: Command,
    input: String,
    selected: Option<usize>,
) {
    match command {
        Command::Provider => {
            draft.values.provider = if selected == Some(1) {
                Provider::Compatible
            } else {
                Provider::Chatgpt
            }
        }
        Command::Protocol => {
            draft.values.protocol = if selected == Some(1) {
                Protocol::ChatCompletions
            } else {
                Protocol::Responses
            }
        }
        Command::Endpoint => draft.values.base_url = input,
        Command::ApiKey => draft.api_key = input,
        Command::Theme => {
            draft.values.theme =
                [Theme::Latte, Theme::Frappe, Theme::Macchiato, Theme::Mocha][selected.unwrap_or(0)]
        }
        _ => {}
    }
}

fn parse_effort(value: &str) -> Option<ReasoningEffort> {
    Some(match value.to_ascii_lowercase().as_str() {
        "none" => ReasoningEffort::None,
        "minimal" => ReasoningEffort::Minimal,
        "low" => ReasoningEffort::Low,
        "medium" => ReasoningEffort::Medium,
        "high" => ReasoningEffort::High,
        "xhigh" => ReasoningEffort::Xhigh,
        "max" => ReasoningEffort::Max,
        _ => return None,
    })
}

fn reconcile_effort(model: &ModelOption, current: ReasoningEffort) -> ReasoningEffort {
    if current != ReasoningEffort::Default
        && model
            .reasoning_efforts
            .iter()
            .any(|item| parse_effort(&item.effort) == Some(current))
    {
        return current;
    }
    model
        .default_reasoning_effort
        .as_deref()
        .and_then(parse_effort)
        .unwrap_or(ReasoningEffort::Default)
}

fn effort_index(model: &ModelOption, effort: ReasoningEffort) -> usize {
    model
        .reasoning_efforts
        .iter()
        .position(|item| parse_effort(&item.effort) == Some(effort))
        .map_or(0, |index| index + 1)
}

fn cycle_effort(value: ReasoningEffort, direction: isize) -> ReasoningEffort {
    let all = [
        ReasoningEffort::Default,
        ReasoningEffort::None,
        ReasoningEffort::Minimal,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::Xhigh,
        ReasoningEffort::Max,
    ];
    cycle_value(&all, value, direction)
}

fn cycle_theme(value: Theme, direction: isize) -> Theme {
    cycle_value(
        &[Theme::Latte, Theme::Frappe, Theme::Macchiato, Theme::Mocha],
        value,
        direction,
    )
}

fn cycle_value<T: Copy + PartialEq>(values: &[T], value: T, direction: isize) -> T {
    let index = values.iter().position(|item| *item == value).unwrap_or(0);
    values[(index as isize + direction).rem_euclid(values.len() as isize) as usize]
}

fn sanitize_paste(value: &str) -> String {
    learning::strip_controls(value)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

async fn login_result() -> Result<secrets::OAuthCredentials, String> {
    let (url, receiver) = match auth::start_login().await {
        Ok(login) => login,
        Err(error) => return Err(error),
    };
    if let Err(error) = open::that_detached(&url) {
        return Err(format!("Could not open browser: {error}. Open: {url}"));
    }
    receiver
        .await
        .map_err(|_| "Login task ended unexpectedly".to_owned())?
}

fn signed_in_message(email: Option<String>) -> String {
    format!(
        "Signed in{}",
        email
            .map(|value| format!(" as {value}"))
            .unwrap_or_default()
    )
}

fn api_key_update(input: &str) -> Option<Option<&str>> {
    match input.trim() {
        "" => None,
        "CLEAR" => Some(None),
        value => Some(Some(value)),
    }
}

fn credential_override(input: &str) -> CredentialOverride {
    match input.trim() {
        "" => CredentialOverride::Unchanged,
        "CLEAR" => CredentialOverride::Clear,
        value => CredentialOverride::Replace(value.to_owned()),
    }
}

fn save_settings_transaction<'a>(
    previous: &Settings,
    next: &Settings,
    secret: Option<Option<&'a str>>,
    mut save_config: impl FnMut(&Settings) -> Result<(), String>,
    mut save_secret: impl FnMut(Option<&'a str>) -> Result<(), String>,
) -> Result<(), String> {
    save_config(next)?;
    if let Some(secret) = secret
        && let Err(error) = save_secret(secret)
    {
        return match save_config(previous) {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}; settings rollback failed: {rollback}")),
        };
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Palette {
    base: Color,
    surface: Color,
    text: Color,
    muted: Color,
    accent: Color,
    green: Color,
    yellow: Color,
    red: Color,
}

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Latte => Palette {
            base: Color::Rgb(239, 241, 245),
            surface: Color::Rgb(204, 208, 218),
            text: Color::Rgb(76, 79, 105),
            muted: Color::Rgb(108, 111, 133),
            accent: Color::Rgb(30, 102, 245),
            green: Color::Rgb(64, 160, 43),
            yellow: Color::Rgb(223, 142, 29),
            red: Color::Rgb(210, 15, 57),
        },
        Theme::Frappe => Palette {
            base: Color::Rgb(48, 52, 70),
            surface: Color::Rgb(65, 69, 89),
            text: Color::Rgb(198, 208, 245),
            muted: Color::Rgb(165, 173, 206),
            accent: Color::Rgb(140, 170, 238),
            green: Color::Rgb(166, 209, 137),
            yellow: Color::Rgb(229, 200, 144),
            red: Color::Rgb(231, 130, 132),
        },
        Theme::Macchiato => Palette {
            base: Color::Rgb(36, 39, 58),
            surface: Color::Rgb(54, 58, 79),
            text: Color::Rgb(202, 211, 245),
            muted: Color::Rgb(165, 173, 203),
            accent: Color::Rgb(138, 173, 244),
            green: Color::Rgb(166, 218, 149),
            yellow: Color::Rgb(238, 212, 159),
            red: Color::Rgb(237, 135, 150),
        },
        Theme::Mocha => Palette {
            base: Color::Rgb(30, 30, 46),
            surface: Color::Rgb(49, 50, 68),
            text: Color::Rgb(205, 214, 244),
            muted: Color::Rgb(166, 173, 200),
            accent: Color::Rgb(137, 180, 250),
            green: Color::Rgb(166, 227, 161),
            yellow: Color::Rgb(249, 226, 175),
            red: Color::Rgb(243, 139, 168),
        },
    }
}

struct MarkdownTheme(Palette);

impl RichTextTheme for MarkdownTheme {
    fn generation(&self) -> Generation {
        Generation(0)
    }
    fn get_text_color(&self) -> Color {
        self.0.text
    }
    fn get_muted_text_color(&self) -> Color {
        self.0.muted
    }
    fn get_primary_color(&self) -> Color {
        self.0.accent
    }
    fn get_secondary_color(&self) -> Color {
        self.0.green
    }
    fn get_info_color(&self) -> Color {
        self.0.accent
    }
    fn get_background_color(&self) -> Color {
        self.0.base
    }
    fn get_border_color(&self) -> Color {
        self.0.surface
    }
    fn get_focused_border_color(&self) -> Color {
        self.0.accent
    }
    fn get_popup_selected_background(&self) -> Color {
        self.0.accent
    }
    fn get_popup_selected_text_color(&self) -> Color {
        self.0.base
    }
    fn get_json_key_color(&self) -> Color {
        self.0.accent
    }
    fn get_json_string_color(&self) -> Color {
        self.0.green
    }
    fn get_json_number_color(&self) -> Color {
        self.0.yellow
    }
    fn get_json_bool_color(&self) -> Color {
        self.0.yellow
    }
    fn get_json_null_color(&self) -> Color {
        self.0.muted
    }
    fn get_accent_yellow(&self) -> Color {
        self.0.yellow
    }
}

fn markdown_text(value: &str, colors: Palette) -> Text<'static> {
    // Ratatui wraps this unwrapped styled text to the transcript's current inner width.
    let renderer = MarkdownRenderer::new(0);
    let blocks = renderer.parse(value);
    Text::from(renderer.render(&blocks, &MarkdownTheme(colors)))
}

fn text_plain(text: &Text<'_>) -> String {
    text.lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rendered_rows(text: Text<'static>, width: u16) -> usize {
    Paragraph::new(text)
        .block(Block::bordered())
        .wrap(Wrap { trim: false })
        .line_count(width)
}

fn highlight_selection(
    text: &mut Text<'static>,
    selection: Option<(usize, usize)>,
    colors: Palette,
) {
    let Some((start, end)) = selection else {
        return;
    };
    let selected = start.min(end)..start.max(end);
    let mut position = 0;
    for line in &mut text.lines {
        let mut spans = Vec::new();
        for span in std::mem::take(&mut line.spans) {
            let style = span.style;
            let mut segment = String::new();
            let mut segment_selected = None;
            for character in span.content.chars() {
                let is_selected = selected.contains(&position);
                if segment_selected.is_some_and(|value| value != is_selected) {
                    spans.push(Span::styled(
                        std::mem::take(&mut segment),
                        if segment_selected == Some(true) {
                            style.bg(colors.accent).fg(colors.base)
                        } else {
                            style
                        },
                    ));
                }
                segment_selected = Some(is_selected);
                segment.push(character);
                position += 1;
            }
            if !segment.is_empty() {
                spans.push(Span::styled(
                    segment,
                    if segment_selected == Some(true) {
                        style.bg(colors.accent).fg(colors.base)
                    } else {
                        style
                    },
                ));
            }
        }
        line.spans = spans;
        position += 1;
    }
}

fn render(frame: &mut Frame, app: &mut App) {
    let colors = palette(app.current_settings().theme);
    frame.render_widget(
        Block::new().style(Style::new().bg(colors.base).fg(colors.text)),
        frame.area(),
    );
    match app.screen {
        Screen::Help => render_help(frame, colors),
        Screen::Settings => render_settings(frame, app, colors),
        Screen::Main => render_main(frame, app, colors),
    }
    if app.popup.is_some() {
        render_popup(frame, app, colors);
    }
}

fn render_main(frame: &mut Frame, app: &mut App, colors: Palette) {
    let [header, body, input, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    app.body_area = body;
    let mode = if app.mode == LearningMode::Socratic {
        "SOCRATIC"
    } else {
        "WORKED EXAMPLE"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " UNFOLD ",
                Style::new()
                    .fg(colors.base)
                    .bg(colors.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {mode}  search:{}  {} / {:?}",
                if app.web_search { "on" } else { "off" },
                app.settings.model,
                app.settings.reasoning_effort
            )),
        ]))
        .block(
            Block::new()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().fg(colors.surface)),
        ),
        header,
    );
    let mut text = app.transcript_rendered();
    highlight_selection(&mut text, app.selection, colors);
    app.transcript_lines = rendered_rows(text.clone(), body.width);
    app.viewport.clamp(app.transcript_lines, app.body_height());
    let title = if app.viewport.unseen > 0 {
        format!(" Session / {} new lines ", app.viewport.unseen)
    } else {
        " Session ".into()
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::bordered()
                    .title(Line::styled(
                        title,
                        Style::new().fg(colors.text).add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::new().fg(colors.surface)),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.viewport.offset.min(u16::MAX as usize) as u16, 0)),
        body,
    );
    if app.active.is_some()
        && app
            .turns
            .last()
            .is_some_and(|turn| learning::visible_content(&turn.content).trim().is_empty())
    {
        let area = centered(46, 28, body);
        let spinner = ["|", "/", "-", "\\"][(app.tick as usize) % 4];
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(format!(
                "\n{spinner}  Preparing {}...",
                if app.mode == LearningMode::Socratic {
                    "your first prompt"
                } else {
                    "worked example"
                }
            ))
            .centered()
            .style(Style::new().fg(colors.yellow).add_modifier(Modifier::BOLD))
            .block(
                Block::bordered()
                    .title(" Loading ")
                    .border_style(Style::new().fg(colors.accent)),
            ),
            area,
        );
    }
    let mut scrollbar = ScrollbarState::new(app.transcript_lines)
        .position(app.viewport.offset)
        .viewport_content_length(app.body_height());
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::new().fg(colors.accent))
            .begin_symbol(None)
            .end_symbol(None),
        body,
        &mut scrollbar,
    );
    let input_title = if app.turns.is_empty() {
        " Problem / Enter to start "
    } else {
        " Response / Enter to send "
    };
    frame.render_widget(
        Paragraph::new(app.input.as_str())
            .style(Style::new().fg(colors.text))
            .block(
                Block::bordered()
                    .title(input_title)
                    .border_style(Style::new().fg(colors.accent)),
            )
            .wrap(Wrap { trim: false }),
        input,
    );
    let spinner = if app.active.is_some() {
        ["|", "/", "-", "\\"][(app.tick as usize) % 4]
    } else {
        "."
    };
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled(
                    format!("{spinner} "),
                    Style::new().fg(if app.active.is_some() {
                        colors.yellow
                    } else {
                        colors.green
                    }),
                ),
                Span::raw(app.status.as_str()),
            ]),
            Line::styled(
                "Ctrl+P commands  F1 help  PgUp/PgDn scroll  Ctrl+C/V copy/paste  Ctrl+Q quit",
                Style::new().fg(colors.text),
            ),
        ])),
        footer,
    );
}

fn render_settings(frame: &mut Frame, app: &App, colors: Palette) {
    let draft = app
        .settings_draft
        .as_ref()
        .expect("settings screen has draft");
    let fields = [
        format!("Provider       {:?}", draft.values.provider),
        format!("Protocol       {:?}", draft.values.protocol),
        format!("Endpoint       {}", draft.values.base_url),
        format!("Model          {}  [Enter: catalog]", draft.values.model),
        format!("Reasoning      {:?}", draft.values.reasoning_effort),
        format!("Theme          {:?}", draft.values.theme),
        format!(
            "API key        {}",
            if draft.api_key.is_empty() {
                "unchanged; type replacement or CLEAR"
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
                value.clone(),
                if index == app.settings_field {
                    Style::new()
                        .fg(colors.base)
                        .bg(colors.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(colors.text)
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .title(" Settings / Tab field / arrows change / Enter/F2 save / Esc discard ")
                    .border_style(Style::new().fg(colors.accent)),
            )
            .wrap(Wrap { trim: false }),
        frame.area(),
    );
}

fn render_help(frame: &mut Frame, colors: Palette) {
    let help = "KEYBOARD\nCtrl+P or F4  Search commands and configuration\nPalette: type to filter; arrows, j/k, Ctrl+N/P, or Ctrl+J/K navigate; Enter confirms; Esc cancels\nEnter  Start/send or confirm dialog\nEsc  Cancel response or close dialog\nF2  Mode before session\nF3  Web search\nF5  Another hint\nF8 or Ctrl+M  Model and reasoning picker\nF9/F10/F12  Explain/check/reveal\nPgUp/PgDn or mouse wheel  Scroll\nCtrl+Home/End  Top/follow latest\nShift+Left/Right  Extend transcript selection\nCtrl+C/Ctrl+V  Copy selection/paste into text editors\nCtrl+N  New problem (outside palette)\nCtrl+Q  Quit\n\nMOUSE\nWheel scrolls. Drag selects transcript text. Click palette and dialog choices to activate them; click setting text editors to focus.\n\nPress any key to return.";
    frame.render_widget(
        Paragraph::new(help)
            .style(Style::new().fg(colors.text))
            .block(
                Block::bordered()
                    .title(" Help ")
                    .border_style(Style::new().fg(colors.accent)),
            )
            .wrap(Wrap { trim: false }),
        frame.area(),
    );
}

fn render_popup(frame: &mut Frame, app: &mut App, colors: Palette) {
    let area = centered(70, 70, frame.area());
    app.popup_area = area;
    frame.render_widget(Clear, area);
    frame.render_widget(Block::new().style(Style::new().bg(colors.base)), area);
    match &mut app.popup {
        Some(Popup::Palette { query, state }) => {
            render_palette_popup(frame, area, query, state, colors);
        }
        Some(Popup::Setting {
            command,
            input,
            state,
            ..
        }) => {
            render_setting_popup(frame, area, *command, input, state, colors);
        }
        Some(Popup::Models {
            state,
            models,
            loading,
            error,
            ..
        }) => {
            if *loading {
                let spinner = ["|", "/", "-", "\\"][(app.tick as usize) % 4];
                frame.render_widget(
                    Paragraph::new(format!(
                        "{spinner} Contacting provider...\n\nEsc closes this dialog"
                    ))
                    .block(
                        Block::bordered()
                            .title(" Models / loading ")
                            .border_style(Style::new().fg(colors.yellow)),
                    ),
                    area,
                );
            } else if let Some(message) = error {
                frame.render_widget(
                    Paragraph::new(format!("{message}\n\nR retry / Esc close"))
                        .style(Style::new().fg(colors.red))
                        .block(Block::bordered().title(" Models / error ")),
                    area,
                );
            } else if models.is_empty() {
                frame.render_widget(
                    Paragraph::new(
                        "The provider returned no available models.\n\nR retry / Esc close",
                    )
                    .block(Block::bordered().title(" Models / empty ")),
                    area,
                );
            } else {
                let items = models
                    .iter()
                    .map(|model| {
                        ListItem::new(format!(
                            "{}{}\n  {} reasoning option(s)",
                            model.name,
                            if model.is_default { "  default" } else { "" },
                            model.reasoning_efforts.len()
                        ))
                    })
                    .collect::<Vec<_>>();
                let list = List::new(items)
                    .block(
                        Block::bordered()
                            .title(" Choose model / arrows or j/k / Enter / Esc ")
                            .border_style(Style::new().fg(colors.accent)),
                    )
                    .highlight_style(
                        Style::new()
                            .fg(colors.base)
                            .bg(colors.accent)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("> ");
                frame.render_stateful_widget(list, area, state);
            }
        }
        Some(Popup::Effort { model, state }) => {
            let mut items = vec![ListItem::new("Provider default")];
            items.extend(model.reasoning_efforts.iter().map(|option| {
                ListItem::new(format!("{:10} {}", option.effort, option.description))
            }));
            let list = List::new(items)
                .block(
                    Block::bordered()
                        .title(format!(" {} / reasoning effort ", model.name))
                        .border_style(Style::new().fg(colors.accent)),
                )
                .highlight_style(
                    Style::new()
                        .fg(colors.base)
                        .bg(colors.accent)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, area, state);
        }
        None => {}
    }
}

fn render_palette_popup(
    frame: &mut Frame,
    area: Rect,
    query: &str,
    state: &mut ListState,
    colors: Palette,
) {
    let items = App::filtered_commands(query)
        .iter()
        .map(|command| ListItem::new(command.label()))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(format!(" Commands > {query}_ "))
                .border_style(Style::new().fg(colors.accent)),
        )
        .highlight_style(
            Style::new()
                .fg(colors.base)
                .bg(colors.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, state);
}

fn render_setting_popup(
    frame: &mut Frame,
    area: Rect,
    command: Command,
    input: &str,
    state: &mut ListState,
    colors: Palette,
) {
    let choices = setting_choices(command);
    if choices.is_empty() {
        let display = if command == Command::ApiKey && !input.is_empty() {
            "*".repeat(input.chars().count())
        } else {
            input.to_owned()
        };
        frame.render_widget(
            Paragraph::new(format!("{display}_\n\nEnter saves / Esc cancels")).block(
                Block::bordered()
                    .title(format!(" {} ", command.label()))
                    .border_style(Style::new().fg(colors.accent)),
            ),
            area,
        );
        return;
    }
    let list = List::new(choices.iter().map(|choice| ListItem::new(*choice)))
        .block(
            Block::bordered()
                .title(format!(" {} / Enter saves / Esc cancels ", command.label()))
                .border_style(Style::new().fg(colors.accent)),
        )
        .highlight_style(
            Style::new()
                .fg(colors.base)
                .bg(colors.accent)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, area, state);
}

fn centered(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [vertical] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    let [result] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(ratatui::layout::Flex::Center)
        .areas(vertical);
    result
}

fn drain_events(app: &mut App) {
    while let Ok(event) = app.events.try_recv() {
        handle_app_event(app, event);
    }
}

fn handle_app_event(app: &mut App, event: AppEvent) {
    match event {
        AppEvent::Generation(id, event) => app.handle_response(id, event),
        AppEvent::Models(id, result) => app.handle_models(id, result),
        AppEvent::Login(id, result) => complete_login_with(app, id, result, secrets::save_oauth),
    }
}

fn complete_login_with(
    app: &mut App,
    id: u64,
    result: Result<secrets::OAuthCredentials, String>,
    save: impl FnOnce(&secrets::OAuthCredentials) -> Result<(), String>,
) {
    if app.login_id != Some(id) {
        return;
    }
    app.login_id = None;
    app.status = match result {
        Ok(credentials) => match save(&credentials) {
            Ok(()) => signed_in_message(credentials.email),
            Err(error) => error,
        },
        Err(error) => error,
    };
}

async fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    loop {
        if run_frame(terminal, &mut app)? {
            app.cancel();
            if let Err(error) = app.persist_on_quit() {
                app.status = error;
            }
            break;
        }
    }
    Ok(())
}

fn run_frame(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<bool> {
    app.tick = app.tick.wrapping_add(1);
    drain_events(app);
    terminal.draw(|frame| render(frame, app))?;
    if !event::poll(Duration::from_millis(50))? {
        return Ok(false);
    }
    Ok(app.handle_event(event::read()?))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = ratatui::try_init()?;
    if let Err(error) = enable_terminal_modes() {
        let _ = ratatui::try_restore();
        return Err(error.into());
    }
    let result = run(&mut terminal).await;
    let mode_result = disable_terminal_modes();
    let restore_result = ratatui::try_restore();
    finish_terminal(result, mode_result, restore_result)
}

fn finish_terminal(
    result: io::Result<()>,
    mode_result: io::Result<()>,
    restore_result: io::Result<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    result?;
    mode_result?;
    restore_result?;
    Ok(())
}

fn enable_terminal_modes() -> io::Result<()> {
    execute!(
        io::stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        EnableFocusChange
    )
}

fn disable_terminal_modes() -> io::Result<()> {
    execute!(
        io::stdout(),
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn test_app() -> App {
        let (event_tx, events) = mpsc::unbounded_channel();
        App {
            settings: Settings::default(),
            settings_draft: None,
            screen: Screen::Main,
            popup: None,
            target: String::new(),
            input: String::new(),
            mode: LearningMode::Socratic,
            web_search: false,
            turns: Vec::new(),
            active: None,
            generation: 0,
            tick: 0,
            operation_id: 0,
            login_id: None,
            events,
            event_tx,
            status: "Ready".into(),
            viewport: Viewport::new(),
            settings_field: 0,
            body_area: Rect::new(0, 0, 80, 20),
            popup_area: Rect::default(),
            transcript_lines: 100,
            selection: None,
            selection_anchor: 0,
            quit_requested: false,
        }
    }

    fn model(id: &str, default: Option<&str>, efforts: &[&str]) -> ModelOption {
        ModelOption {
            id: id.into(),
            name: id.into(),
            is_default: false,
            default_reasoning_effort: default.map(str::to_owned),
            reasoning_efforts: efforts
                .iter()
                .map(|effort| provider::ReasoningOption {
                    effort: (*effort).into(),
                    description: String::new(),
                })
                .collect(),
        }
    }

    fn credentials(email: &str) -> secrets::OAuthCredentials {
        secrets::OAuthCredentials {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_at_ms: 1,
            account_id: None,
            email: Some(email.into()),
        }
    }

    #[test]
    fn empty_model_does_not_block_provider_discovery_validation() {
        let mut settings = Settings::default();
        settings.model.clear();
        assert!(settings.validate().is_err());
        assert!(settings.validate_provider().is_ok());
    }

    #[test]
    fn model_effort_is_preserved_when_supported_and_reconciled_to_default() {
        let option = model("new", Some("medium"), &["low", "medium"]);
        assert_eq!(
            reconcile_effort(&option, ReasoningEffort::Low),
            ReasoningEffort::Low
        );
        assert_eq!(
            reconcile_effort(&option, ReasoningEffort::Max),
            ReasoningEffort::Medium
        );
    }

    #[test]
    fn stale_model_catalog_results_are_ignored() {
        let mut app = test_app();
        app.popup = Some(Popup::Models {
            request_id: 2,
            state: ListState::default(),
            models: Vec::new(),
            loading: true,
            error: None,
        });
        app.handle_models(1, Ok(vec![model("stale", None, &[])]));
        assert!(matches!(
            app.popup,
            Some(Popup::Models { loading: true, .. })
        ));
        app.handle_models(2, Ok(vec![model("current", None, &[])]));
        assert!(matches!(
            app.popup,
            Some(Popup::Models { loading: false, .. })
        ));
    }

    #[test]
    fn confirming_model_keeps_metadata_for_effort_popup() {
        let mut app = test_app();
        let option = model("gpt", Some("high"), &["low", "high"]);
        let mut state = ListState::default();
        state.select(Some(0));
        app.popup = Some(Popup::Models {
            request_id: 1,
            state,
            models: vec![option],
            loading: false,
            error: None,
        });
        app.select_model();
        assert!(
            matches!(app.popup, Some(Popup::Effort { ref model, .. }) if model.reasoning_efforts.len() == 2)
        );
    }

    #[test]
    fn model_and_effort_confirmation_update_the_persistable_settings_draft() {
        let mut app = test_app();
        app.open_settings();
        let option = model("chosen", Some("high"), &["low", "high"]);
        app.confirm_model(&option, ReasoningEffort::High);
        let draft = app.settings_draft.as_ref().unwrap();
        assert_eq!(draft.values.model, "chosen");
        assert_eq!(draft.values.reasoning_effort, ReasoningEffort::High);
    }

    #[test]
    fn settings_escape_rolls_back_all_draft_changes() {
        let mut app = test_app();
        let original = app.settings.model.clone();
        app.open_settings();
        app.settings_field = 3;
        app.edit_setting(Some('x'));
        app.close_settings();
        assert_eq!(app.settings.model, original);
        assert!(app.settings_draft.is_none());
    }

    #[test]
    fn quit_persists_committed_settings_and_discards_open_draft() {
        let mut app = test_app();
        app.settings.model = "committed".into();
        app.open_settings();
        app.settings_draft.as_mut().unwrap().values.model = "draft".into();
        app.settings_draft.as_mut().unwrap().api_key = "secret-draft".into();
        let mut persisted = None;

        app.persist_on_quit_with(|settings| {
            persisted = Some(settings.model.clone());
            Ok(())
        })
        .unwrap();

        assert_eq!(persisted.as_deref(), Some("committed"));
    }

    #[test]
    fn failed_secret_save_rolls_config_back_to_previous_settings() {
        let previous = Settings::default();
        let mut next = previous.clone();
        next.model = "new-model".into();
        let mut writes = Vec::new();

        let error = save_settings_transaction(
            &previous,
            &next,
            Some(Some("replacement")),
            |settings| {
                writes.push(settings.model.clone());
                Ok(())
            },
            |_| Err("keyring failed".into()),
        )
        .unwrap_err();

        assert_eq!(error, "keyring failed");
        assert_eq!(writes, ["new-model", previous.model.as_str()]);
    }

    #[test]
    fn viewport_follows_tail_reports_unseen_and_clamps() {
        let mut view = Viewport::new();
        view.clamp(100, 20);
        assert_eq!(view.offset, 80);
        view.scroll(-5, 100, 20);
        view.appended(3);
        assert_eq!(view.unseen, 3);
        assert!(!view.follow_tail);
        view.scroll(100, 103, 20);
        assert!(view.follow_tail);
        assert_eq!(view.unseen, 0);
        view.offset = 99;
        view.follow_tail = false;
        view.clamp(10, 20);
        assert_eq!(view.offset, 0);
    }

    #[test]
    fn narrow_transcript_counts_wrapped_display_rows() {
        let mut app = test_app();
        app.body_area = Rect::new(0, 0, 12, 10);
        app.target = "a long Markdown line that wraps several times".into();

        assert!(app.transcript_display_rows() > app.transcript_rendered().height());
    }

    #[test]
    fn streaming_unseen_count_changes_only_when_wrapping_adds_a_row() {
        let mut app = test_app();
        app.body_area = Rect::new(0, 0, 20, 10);
        app.turns.push(Turn {
            label: "Guide".into(),
            content: "x".into(),
            detail: None,
        });
        app.active = Some((1, CancellationToken::new()));
        app.viewport.follow_tail = false;

        app.handle_response(1, ResponseEvent::TextDelta { delta: "y".into() });
        assert_eq!(app.viewport.unseen, 0);
        app.handle_response(
            1,
            ResponseEvent::TextDelta {
                delta: " z z z z z z z z z z z z z z z z z z z z".into(),
            },
        );
        assert!(app.viewport.unseen > 0);
    }

    #[test]
    fn markdown_renderer_styles_headings_lists_quotes_and_code() {
        let text = markdown_text(
            "## Head\n- item\n> quote\n```\ncode\n```",
            palette(Theme::Mocha),
        );
        assert!(
            text.lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("Head")
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(text_plain(&text).contains("item"));
        assert!(
            text.lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("quote")
                    && span.style.add_modifier.contains(Modifier::ITALIC))
        );
        assert!(
            text.lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("code")
                    && span.style.fg != Some(palette(Theme::Mocha).text))
        );
    }

    #[test]
    fn inline_code_is_distinct_and_math_delimiters_remain_readable() {
        let colors = palette(Theme::Mocha);
        let text = markdown_text("`get`: average $O(1)$", colors);
        let get = text
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == "get")
            .unwrap();
        assert_ne!(get.style, Style::new().fg(colors.text));
        assert!(text_plain(&text).contains("average $O(1)$"));
    }

    #[test]
    fn selection_uses_the_same_markdown_transformed_unicode_text_as_highlighting() {
        let mut app = test_app();
        app.turns.push(Turn {
            label: "Guide".into(),
            content: "# Heading\n- café 🦀\n```rust\nlet λ = 1;\n```".into(),
            detail: None,
        });
        let plain = app.transcript_rendered_plain();
        assert!(plain.contains("Heading"));
        assert!(plain.contains("café 🦀"));
        assert!(plain.contains("let λ = 1;"));
        let start = plain
            .chars()
            .position(|character| character == 'c')
            .unwrap();
        let end = start + "café 🦀".chars().count();
        app.selection = Some((start, end));
        assert_eq!(app.selected_text().as_deref(), Some("café 🦀"));

        let colors = palette(Theme::Mocha);
        let mut rendered = app.transcript_rendered();
        highlight_selection(&mut rendered, app.selection, colors);
        let highlighted = rendered
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .filter(|span| span.style.bg == Some(colors.accent))
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(highlighted, "café 🦀");
    }

    #[test]
    fn paste_event_is_sanitized_and_inserted_into_active_editor() {
        let mut app = test_app();
        app.handle_event(Event::Paste("a\x1b[2J\r\nb".into()));
        assert_eq!(app.input, "a\nb");
        app.open_settings();
        app.settings_field = 3;
        app.handle_event(Event::Paste("x\x00y".into()));
        assert!(
            app.settings_draft
                .as_ref()
                .unwrap()
                .values
                .model
                .ends_with("xy")
        );
    }

    #[test]
    fn setting_text_editors_accept_sanitized_event_and_ctrl_v_paste() {
        let mut app = test_app();
        app.open_setting(Command::Endpoint);
        app.handle_event(Event::Paste("\x1b[2Jhttps://paste.test\r\n/v1".into()));
        assert!(matches!(
            &app.popup,
            Some(Popup::Setting { input, .. }) if input.ends_with("https://paste.test\n/v1")
        ));

        app.open_setting(Command::ApiKey);
        app.paste_clipboard_with(|| Ok("sec\x00ret".into()));
        assert!(matches!(
            &app.popup,
            Some(Popup::Setting { input, .. }) if input == "secret"
        ));
    }

    #[test]
    fn paste_does_not_mutate_popup_choice_lists() {
        let mut app = test_app();
        app.open_setting(Command::Theme);
        app.insert_paste("Mocha");
        assert!(matches!(
            &app.popup,
            Some(Popup::Setting { input, state, .. })
                if input.is_empty() && state.selected() == Some(3)
        ));
    }

    #[test]
    fn wide_transcript_does_not_wrap_or_copy_an_artificial_newline_at_column_120() {
        let mut app = test_app();
        app.body_area = Rect::new(0, 0, 180, 20);
        let long = format!("{}tail", "word ".repeat(30));
        app.target = long.clone();

        let rendered = app.transcript_rendered();
        assert!(rendered.lines.iter().any(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .contains(&long)
        }));
        assert!(app.transcript_rendered_plain().contains(&long));
    }

    #[test]
    fn paste_does_not_edit_settings_behind_popup() {
        let mut app = test_app();
        app.open_settings();
        app.settings_field = 3;
        let original = app.current_settings().model.clone();
        app.popup = Some(Popup::Models {
            request_id: 1,
            state: ListState::default(),
            models: Vec::new(),
            loading: true,
            error: None,
        });

        app.insert_paste("hidden edit");

        assert_eq!(app.current_settings().model, original);
    }

    #[test]
    fn mouse_wheel_changes_viewport_and_disables_following() {
        let mut app = test_app();
        app.viewport.offset = 80;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.viewport.offset, 77);
        assert!(!app.viewport.follow_tail);
    }

    #[test]
    fn keyboard_selection_uses_sanitized_visible_transcript() {
        let mut app = test_app();
        app.target = "safe\x1b[2J".into();
        app.extend_selection(1);
        assert_eq!(app.selection, Some((0, 1)));
        assert!(!app.transcript_rendered_plain().contains('\x1b'));
    }

    #[test]
    fn popups_render_loading_error_empty_and_choices() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut app = test_app();
        for popup in [
            Popup::Models {
                request_id: 1,
                state: ListState::default(),
                models: Vec::new(),
                loading: true,
                error: None,
            },
            Popup::Models {
                request_id: 1,
                state: ListState::default(),
                models: Vec::new(),
                loading: false,
                error: Some("bad".into()),
            },
            Popup::Models {
                request_id: 1,
                state: ListState::default(),
                models: Vec::new(),
                loading: false,
                error: None,
            },
        ] {
            app.popup = Some(popup);
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
        }
    }

    #[test]
    fn theme_round_trips_with_settings_json() {
        let settings = Settings {
            theme: Theme::Latte,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.theme, Theme::Latte);
    }

    #[test]
    fn response_events_update_only_the_active_generation() {
        let mut app = test_app();
        app.turns.push(Turn {
            label: "Hint".into(),
            content: String::new(),
            detail: None,
        });
        app.active = Some((7, CancellationToken::new()));

        app.handle_response(6, ResponseEvent::Started);
        assert_eq!(app.status, "Ready");
        app.handle_response(7, ResponseEvent::Started);
        assert_eq!(app.status, "Streaming response...");
        app.handle_response(
            7,
            ResponseEvent::TextDelta {
                delta: "safe\x1b[2J\nnext".into(),
            },
        );
        app.handle_response(
            7,
            ResponseEvent::Source {
                title: "Docs".into(),
                url: "https://example.com".into(),
            },
        );
        assert!(app.turns[0].content.contains("safe\nnext"));
        assert!(app.turns[0].content.contains("Source: Docs"));
        app.handle_response(7, ResponseEvent::Completed);
        assert!(app.active.is_none());
        assert_eq!(app.status, "Complete");

        for event in [
            ResponseEvent::Cancelled,
            ResponseEvent::Failed {
                message: "failed".into(),
            },
        ] {
            app.turns.push(Turn {
                label: "Pending".into(),
                content: String::new(),
                detail: None,
            });
            app.active = Some((8, CancellationToken::new()));
            app.handle_response(8, event);
            assert!(app.active.is_none());
            assert_ne!(
                app.turns.last().map(|turn| turn.label.as_str()),
                Some("Pending")
            );
        }
        assert_eq!(app.status, "failed");
    }

    #[test]
    fn pending_initial_turn_is_hidden_until_visible_text_and_removed_on_failure() {
        for mode in [LearningMode::Socratic, LearningMode::WorkedExample] {
            let mut app = test_app();
            app.mode = mode;
            app.target = "2 + 2".into();
            app.turns.push(Turn {
                label: learning::label(LearningAction::Initial, mode).into(),
                content: String::new(),
                detail: None,
            });
            app.active = Some((1, CancellationToken::new()));
            assert!(
                !app.transcript_source()
                    .contains(app.turns[0].label.as_str())
            );

            app.handle_response(
                1,
                ResponseEvent::TextDelta {
                    delta: "First token".into(),
                },
            );
            assert!(
                app.transcript_source()
                    .contains(app.turns[0].label.as_str())
            );
            assert!(app.transcript_source().contains("First token"));

            app.turns[0].content.clear();
            app.handle_response(
                1,
                ResponseEvent::Failed {
                    message: "network failed".into(),
                },
            );
            assert!(app.turns.is_empty());
            assert_eq!(app.status, "network failed");
        }
    }

    #[test]
    fn hidden_only_delta_keeps_pending_turn_hidden_and_loading_visible() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut app = test_app();
        app.target = "problem".into();
        app.turns.push(Turn {
            label: "Question".into(),
            content: "<analysis>still thinking</analysis>".into(),
            detail: None,
        });
        app.active = Some((1, CancellationToken::new()));

        assert!(!app.transcript_source().contains("Question"));
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Preparing your first prompt"));

        app.handle_response(
            1,
            ResponseEvent::Failed {
                message: "failed".into(),
            },
        );
        assert!(app.turns.is_empty());
    }

    #[test]
    fn pending_follow_up_preserves_learner_detail_without_empty_assistant_heading() {
        let mut app = test_app();
        app.target = "problem".into();
        app.turns.push(Turn {
            label: "Follow-up".into(),
            content: "<think>working</think>".into(),
            detail: Some("my attempt".into()),
        });
        app.active = Some((1, CancellationToken::new()));

        let source = app.transcript_source();
        assert!(source.contains("You\nmy attempt"));
        assert!(!source.contains("Follow-up"));
    }

    #[test]
    fn cancel_removes_the_empty_pending_initial_turn() {
        let mut app = test_app();
        app.target = "problem".into();
        app.turns.push(Turn {
            label: "Question".into(),
            content: String::new(),
            detail: None,
        });
        app.active = Some((3, CancellationToken::new()));
        app.handle_response(3, ResponseEvent::Cancelled);
        assert!(app.turns.is_empty());
        assert_eq!(app.status, "Cancelled");
    }

    #[test]
    fn palette_filters_navigates_without_recursive_ctrl_p_and_escape_cancels_editor() {
        let mut app = test_app();
        app.handle_key(modified_key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.handle_popup_key(key(KeyCode::Char('t')));
        assert!(matches!(&app.popup, Some(Popup::Palette { query, .. }) if query == "t"));
        app.handle_popup_key(modified_key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(matches!(app.popup, Some(Popup::Palette { .. })));

        let original = app.settings.base_url.clone();
        app.open_setting(Command::Endpoint);
        app.handle_popup_key(key(KeyCode::Char('x')));
        app.handle_popup_key(key(KeyCode::Esc));
        assert_eq!(app.settings.base_url, original);
        assert!(matches!(app.popup, Some(Popup::Palette { .. })));
    }

    #[test]
    fn palette_keyboard_navigation_query_editing_and_selection_preserve_semantics() {
        let mut app = test_app();
        app.open_palette();

        for input in [
            key(KeyCode::Down),
            modified_key(KeyCode::Char('n'), KeyModifiers::CONTROL),
            key(KeyCode::Char('j')),
            key(KeyCode::Char('k')),
            modified_key(KeyCode::Char('p'), KeyModifiers::CONTROL),
            modified_key(KeyCode::Char('k'), KeyModifiers::CONTROL),
            key(KeyCode::Up),
        ] {
            app.handle_palette_key(input);
        }
        assert!(matches!(
            &app.popup,
            Some(Popup::Palette { state, .. }) if state.selected() == Some(0)
        ));

        app.handle_palette_key(key(KeyCode::Char('x')));
        app.handle_palette_key(key(KeyCode::Backspace));
        app.handle_palette_key(modified_key(KeyCode::Char('x'), KeyModifiers::CONTROL));
        app.handle_palette_key(key(KeyCode::Char('t')));
        app.handle_palette_key(key(KeyCode::Char('h')));
        app.handle_palette_key(key(KeyCode::Char('e')));
        app.handle_palette_key(key(KeyCode::Char('m')));
        app.handle_palette_key(key(KeyCode::Char('e')));
        app.handle_palette_key(key(KeyCode::Enter));
        assert!(matches!(
            app.popup,
            Some(Popup::Setting {
                command: Command::Theme,
                ..
            })
        ));

        app.open_palette();
        app.handle_palette_key(key(KeyCode::Esc));
        assert!(app.popup.is_none());
    }

    #[test]
    fn palette_commands_apply_local_actions_and_open_setting_editors() {
        let mut app = test_app();
        for command in [
            Command::Provider,
            Command::Protocol,
            Command::Endpoint,
            Command::ApiKey,
            Command::Theme,
            Command::LearningMode,
        ] {
            app.execute_command(command);
            assert!(matches!(
                app.popup,
                Some(Popup::Setting { command: actual, .. }) if actual == command
            ));
        }

        app.web_search = false;
        app.execute_command(Command::WebSearch);
        assert!(app.web_search);
        assert!(app.popup.is_none());

        app.execute_command(Command::Help);
        assert_eq!(app.screen, Screen::Help);
        app.turns.push(Turn {
            label: "old".into(),
            content: "answer".into(),
            detail: None,
        });
        app.execute_command(Command::NewSession);
        assert!(app.turns.is_empty());
        app.execute_command(Command::Quit);
        assert!(app.quit_requested);
    }

    #[test]
    fn setting_drafts_map_each_palette_choice_without_persisting() {
        let mut draft = SettingsDraft {
            values: Settings::default(),
            api_key: String::new(),
        };
        apply_setting_draft(&mut draft, Command::Provider, String::new(), Some(1));
        apply_setting_draft(&mut draft, Command::Protocol, String::new(), Some(1));
        apply_setting_draft(
            &mut draft,
            Command::Endpoint,
            "https://example.test/v1".into(),
            None,
        );
        apply_setting_draft(&mut draft, Command::ApiKey, "secret".into(), None);
        apply_setting_draft(&mut draft, Command::Theme, String::new(), Some(2));
        apply_setting_draft(&mut draft, Command::Help, "ignored".into(), None);

        assert_eq!(draft.values.provider, Provider::Compatible);
        assert_eq!(draft.values.protocol, Protocol::ChatCompletions);
        assert_eq!(draft.values.base_url, "https://example.test/v1");
        assert_eq!(draft.api_key, "secret");
        assert_eq!(draft.values.theme, Theme::Macchiato);
        assert_eq!(
            selected_learning_mode(Command::LearningMode, Some(1)),
            Some(LearningMode::WorkedExample)
        );
        assert_eq!(
            selected_learning_mode(Command::LearningMode, Some(0)),
            Some(LearningMode::Socratic)
        );
        assert_eq!(selected_learning_mode(Command::Theme, Some(1)), None);
    }

    #[test]
    fn palette_and_setting_popups_render_choices_and_mask_api_keys() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut app = test_app();
        for popup in [
            Popup::Palette {
                query: "theme".into(),
                state: ListState::default(),
            },
            Popup::Setting {
                command: Command::Theme,
                draft: SettingsDraft {
                    values: app.settings.clone(),
                    api_key: String::new(),
                },
                input: String::new(),
                state: ListState::default(),
            },
            Popup::Setting {
                command: Command::ApiKey,
                draft: SettingsDraft {
                    values: app.settings.clone(),
                    api_key: String::new(),
                },
                input: "hunter2".into(),
                state: ListState::default(),
            },
        ] {
            app.popup = Some(popup);
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(!rendered.contains("hunter2"));
        }
    }

    #[test]
    fn session_title_is_high_contrast_in_every_theme_and_loading_panel_is_visible() {
        for theme in [Theme::Latte, Theme::Frappe, Theme::Macchiato, Theme::Mocha] {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            let mut app = test_app();
            app.settings.theme = theme;
            app.mode = LearningMode::WorkedExample;
            app.target = "problem".into();
            app.turns.push(Turn {
                label: "Worked example".into(),
                content: String::new(),
                detail: None,
            });
            app.active = Some((1, CancellationToken::new()));
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let rendered = buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Preparing worked example"));
            let session = buffer
                .content()
                .iter()
                .find(|cell| cell.symbol() == "S")
                .expect("Session title");
            assert_eq!(session.fg, palette(theme).text);
            assert!(session.modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn main_keys_drive_navigation_editing_and_session_controls() {
        let mut app = test_app();
        assert!(!app.handle_key(KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )));
        assert!(app.handle_key(modified_key(KeyCode::Char('q'), KeyModifiers::CONTROL,)));
        assert!(!app.handle_key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL,)));
        assert!(app.status.contains("Nothing selected"));

        app.handle_main_key(key(KeyCode::F(1)));
        assert_eq!(app.screen, Screen::Help);
        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(app.screen, Screen::Main);
        app.handle_main_key(key(KeyCode::F(2)));
        assert_eq!(app.mode, LearningMode::WorkedExample);
        app.handle_main_key(key(KeyCode::F(3)));
        assert!(app.web_search);
        app.handle_main_key(key(KeyCode::F(4)));
        assert!(matches!(app.popup, Some(Popup::Palette { .. })));
        app.popup = None;

        app.input = "ab".into();
        app.handle_main_key(key(KeyCode::Backspace));
        app.handle_main_key(key(KeyCode::Char('c')));
        assert_eq!(app.input, "ac");
        app.viewport.offset = 20;
        app.viewport.follow_tail = false;
        app.handle_main_key(key(KeyCode::PageUp));
        app.handle_main_key(key(KeyCode::PageDown));
        app.handle_main_key(modified_key(KeyCode::Home, KeyModifiers::CONTROL));
        assert_eq!(app.viewport.offset, 0);
        app.handle_main_key(modified_key(KeyCode::End, KeyModifiers::CONTROL));
        assert!(app.viewport.follow_tail);

        app.target = "problem".into();
        app.turns.push(Turn {
            label: "Answer".into(),
            content: "text".into(),
            detail: None,
        });
        app.handle_main_key(modified_key(KeyCode::Right, KeyModifiers::SHIFT));
        assert_eq!(app.selection, Some((0, 1)));
        app.handle_main_key(key(KeyCode::Esc));
        assert!(app.selection.is_none());
        app.handle_main_key(modified_key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert!(app.turns.is_empty());
        assert_eq!(app.status, "New problem");

        let mut actions = test_app();
        actions.turns.push(Turn {
            label: "Question".into(),
            content: "content".into(),
            detail: None,
        });
        for number in [5, 9, 10, 12, 11] {
            actions.handle_main_key(key(KeyCode::F(number)));
        }
        assert_eq!(actions.status, "Enter a problem first");
    }

    #[test]
    fn settings_keys_cycle_and_edit_every_supported_field() {
        let mut app = test_app();
        app.open_settings();
        app.handle_settings_key(key(KeyCode::Tab));
        app.handle_settings_key(key(KeyCode::BackTab));
        assert_eq!(app.settings_field, 0);

        app.cycle_setting(1);
        assert_eq!(app.current_settings().provider, Provider::Compatible);
        app.settings_field = 1;
        app.web_search = true;
        app.cycle_setting(1);
        assert_eq!(app.current_settings().protocol, Protocol::ChatCompletions);
        assert!(!app.web_search);
        app.settings_field = 4;
        app.handle_settings_key(key(KeyCode::Right));
        assert_eq!(
            app.current_settings().reasoning_effort,
            ReasoningEffort::None
        );
        app.settings_field = 5;
        app.handle_settings_key(key(KeyCode::Left));
        assert_eq!(app.current_settings().theme, Theme::Macchiato);

        for field in [2, 3, 6] {
            app.settings_field = field;
            app.handle_settings_key(key(KeyCode::Char('x')));
            app.handle_settings_key(key(KeyCode::Backspace));
        }
        app.settings_field = 0;
        app.edit_setting(Some('x'));
        app.handle_settings_key(key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Main);
    }

    #[test]
    fn provider_and_protocol_changes_immediately_disable_invalid_search() {
        let mut app = test_app();
        app.open_settings();
        app.settings_draft.as_mut().unwrap().values.provider = Provider::Compatible;
        app.web_search = true;
        app.settings_field = 1;
        app.cycle_setting(1);
        assert!(!app.web_search);

        app.settings_draft.as_mut().unwrap().values.protocol = Protocol::ChatCompletions;
        app.settings_draft.as_mut().unwrap().values.provider = Provider::Chatgpt;
        app.web_search = true;
        app.settings_field = 0;
        app.cycle_setting(1);
        assert!(!app.web_search);
    }

    #[test]
    fn api_key_draft_maps_to_all_credential_override_states() {
        assert_eq!(credential_override(""), CredentialOverride::Unchanged);
        assert_eq!(credential_override(" CLEAR "), CredentialOverride::Clear);
        assert_eq!(
            credential_override(" replacement "),
            CredentialOverride::Replace("replacement".into())
        );
    }

    #[test]
    fn popup_keyboard_and_mouse_choose_valid_items_only() {
        let mut app = test_app();
        app.open_settings();
        let mut state = ListState::default();
        state.select(Some(0));
        app.popup = Some(Popup::Models {
            request_id: 1,
            state,
            models: vec![model("first", None, &[]), model("second", None, &[])],
            loading: false,
            error: None,
        });
        app.handle_popup_key(key(KeyCode::Down));
        app.handle_popup_key(key(KeyCode::Up));
        app.handle_popup_key(key(KeyCode::Char('j')));
        app.handle_popup_key(key(KeyCode::Char('k')));
        app.handle_popup_key(key(KeyCode::Enter));
        assert_eq!(app.current_settings().model, "first");

        let effort_model = model("reasoning", Some("high"), &["low", "high"]);
        let mut state = ListState::default();
        state.select(Some(0));
        app.popup = Some(Popup::Effort {
            model: effort_model,
            state,
        });
        app.popup_area = Rect::new(10, 10, 40, 10);
        app.handle_popup_click(12, 12);
        assert_eq!(
            app.current_settings().reasoning_effort,
            ReasoningEffort::Low
        );
        app.handle_popup_click(0, 0);
        app.handle_popup_key(key(KeyCode::Esc));
        assert!(app.popup.is_none());
    }

    #[test]
    fn popup_click_accounts_for_scrolled_list_offset() {
        let mut app = test_app();
        app.open_settings();
        app.popup = Some(Popup::Models {
            request_id: 1,
            state: ListState::default().with_offset(2),
            models: vec![
                model("zero", None, &[]),
                model("one", None, &[]),
                model("two", None, &[]),
                model("three", None, &[]),
            ],
            loading: false,
            error: None,
        });
        app.popup_area = Rect::new(10, 10, 40, 10);

        app.handle_popup_click(12, 11);

        assert_eq!(app.current_settings().model, "two");
    }

    #[test]
    fn mouse_click_activates_palette_rows_with_list_offset() {
        let mut app = test_app();
        app.popup = Some(Popup::Palette {
            query: String::new(),
            state: ListState::default().with_offset(4),
        });
        app.popup_area = Rect::new(10, 10, 40, 10);

        app.handle_popup_click(12, 11);

        assert_eq!(app.screen, Screen::Main);
        assert!(matches!(
            app.popup,
            Some(Popup::Setting {
                command: Command::ApiKey,
                ..
            })
        ));
    }

    #[test]
    fn mouse_click_selects_setting_choice_with_list_offset() {
        let mut app = test_app();
        app.open_settings();
        app.popup = Some(Popup::Setting {
            command: Command::Theme,
            draft: SettingsDraft {
                values: app.settings.clone(),
                api_key: String::new(),
            },
            input: String::new(),
            state: ListState::default().with_offset(2),
        });
        app.popup_area = Rect::new(10, 10, 40, 10);

        app.handle_popup_click(12, 11);

        assert!(app.popup.is_none());
        assert_eq!(app.current_settings().theme, Theme::Macchiato);
    }

    #[test]
    fn selection_highlighting_splits_spans_at_both_boundaries() {
        let colors = palette(Theme::Mocha);
        let mut text = Text::from(Line::from(vec![Span::raw("abcd")]));
        highlight_selection(&mut text, Some((1, 3)), colors);
        assert_eq!(text.lines[0].spans.len(), 3);
        assert_eq!(text.lines[0].spans[1].content, "bc");
        assert_eq!(text.lines[0].spans[1].style.bg, Some(colors.accent));
        highlight_selection(&mut text, None, colors);
    }

    #[test]
    fn queued_events_apply_current_results_and_ignore_stale_logins() {
        let mut app = test_app();
        app.login_id = Some(2);
        let mut saved = Vec::new();
        complete_login_with(&mut app, 1, Ok(credentials("stale@example.com")), |value| {
            saved.push(value.email.clone().unwrap());
            Ok(())
        });
        complete_login_with(
            &mut app,
            2,
            Ok(credentials("current@example.com")),
            |value| {
                saved.push(value.email.clone().unwrap());
                Ok(())
            },
        );
        assert_eq!(saved, ["current@example.com"]);
        assert_eq!(app.status, "Signed in as current@example.com");
        assert!(app.login_id.is_none());
    }

    #[test]
    fn logout_prevents_pending_login_from_persisting_credentials() {
        let mut app = test_app();
        app.login_id = Some(7);
        app.login_id = None;
        let mut saved = false;
        complete_login_with(&mut app, 7, Ok(credentials("late@example.com")), |_| {
            saved = true;
            Ok(())
        });
        assert!(!saved);
        assert_eq!(signed_in_message(None), "Signed in");
    }

    #[test]
    fn terminal_cleanup_reports_the_first_failure() {
        assert!(finish_terminal(Ok(()), Ok(()), Ok(())).is_ok());
        let error = finish_terminal(
            Err(io::Error::other("run")),
            Err(io::Error::other("mode")),
            Err(io::Error::other("restore")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "run");
    }
}
