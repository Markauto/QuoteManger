use std::io::{self, Stdout};

use anyhow::{Context, Result, anyhow};
use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::db::{Database, InsertOutcome};
use crate::model::{AttributionUpdate, Quote, QuoteFilter, normalize_required};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Text,
    Attribution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Minimum,
    Maximum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Search {
        input: String,
    },
    Filter {
        minimum: String,
        maximum: String,
        field: FilterField,
    },
    Add {
        text: String,
        attribution: String,
        field: FormField,
    },
    Edit {
        id: i64,
        text: String,
        attribution: String,
        field: FormField,
    },
    ConfirmDelete {
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Reload,
    Random,
    Add {
        text: String,
        attribution: Option<String>,
    },
    Edit {
        id: i64,
        text: String,
        attribution: Option<String>,
    },
    Delete {
        id: i64,
    },
}

#[derive(Debug)]
pub struct AppState {
    pub quotes: Vec<Quote>,
    pub selected: Option<usize>,
    pub filter: QuoteFilter,
    pub mode: Mode,
    pub status: String,
}

impl AppState {
    #[must_use]
    pub fn new(quotes: Vec<Quote>) -> Self {
        let selected = (!quotes.is_empty()).then_some(0);
        Self {
            quotes,
            selected,
            filter: QuoteFilter::default(),
            mode: Mode::Browse,
            status: String::from("Ready"),
        }
    }

    #[must_use]
    pub fn selected_quote(&self) -> Option<&Quote> {
        self.selected.and_then(|index| self.quotes.get(index))
    }

    pub fn replace_quotes(&mut self, quotes: Vec<Quote>, preferred_id: Option<i64>) {
        let previous_index = self.selected.unwrap_or_default();
        let previous_id = self.selected_quote().map(|quote| quote.id);
        self.quotes = quotes;
        self.selected = preferred_id
            .or(previous_id)
            .and_then(|id| self.quotes.iter().position(|quote| quote.id == id))
            .or_else(|| {
                (!self.quotes.is_empty()).then_some(previous_index.min(self.quotes.len() - 1))
            });
    }

    pub fn select_id(&mut self, id: i64) {
        if let Some(index) = self.quotes.iter().position(|quote| quote.id == id) {
            self.selected = Some(index);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Action::None;
        }
        match self.mode.clone() {
            Mode::Browse => self.handle_browse_key(key),
            Mode::Search { input } => self.handle_search_key(key, input),
            Mode::Filter {
                minimum,
                maximum,
                field,
            } => self.handle_filter_key(key, minimum, maximum, field),
            Mode::Add {
                text,
                attribution,
                field,
            } => self.handle_quote_form_key(key, None, text, attribution, field),
            Mode::Edit {
                id,
                text,
                attribution,
                field,
            } => self.handle_quote_form_key(key, Some(id), text, attribution, field),
            Mode::ConfirmDelete { id } => self.handle_delete_key(key, id),
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_selection(1);
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.move_selection(-1);
                Action::None
            }
            KeyCode::Char('q') if key.modifiers.is_empty() => Action::Quit,
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.mode = Mode::Search {
                    input: self.filter.search.clone().unwrap_or_default(),
                };
                Action::None
            }
            KeyCode::Char('f') if key.modifiers.is_empty() => {
                self.mode = Mode::Filter {
                    minimum: self
                        .filter
                        .min_width
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    maximum: self
                        .filter
                        .max_width
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    field: FilterField::Minimum,
                };
                Action::None
            }
            KeyCode::Char('r') if key.modifiers.is_empty() => Action::Random,
            KeyCode::Char('a') if key.modifiers.is_empty() => {
                self.mode = Mode::Add {
                    text: String::new(),
                    attribution: String::new(),
                    field: FormField::Text,
                };
                Action::None
            }
            KeyCode::Char('e') if key.modifiers.is_empty() => {
                if let Some(quote) = self.selected_quote() {
                    self.mode = Mode::Edit {
                        id: quote.id,
                        text: quote.text.clone(),
                        attribution: quote.attribution.clone().unwrap_or_default(),
                        field: FormField::Text,
                    };
                } else {
                    self.status = String::from("There is no selected quote to edit");
                }
                Action::None
            }
            KeyCode::Char('d') if key.modifiers.is_empty() => {
                if let Some(id) = self.selected_quote().map(|quote| quote.id) {
                    self.mode = Mode::ConfirmDelete { id };
                } else {
                    self.status = String::from("There is no selected quote to delete");
                }
                Action::None
            }
            KeyCode::Esc => {
                if self.filter != QuoteFilter::default() {
                    self.filter = QuoteFilter::default();
                    self.status = String::from("Cleared search and width filters");
                    Action::Reload
                } else {
                    self.status = String::from("Ready");
                    Action::None
                }
            }
            _ => Action::None,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent, mut input: String) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Enter => {
                self.filter.search = (!input.trim().is_empty()).then(|| input.trim().to_owned());
                self.mode = Mode::Browse;
                self.status = String::from("Search updated");
                Action::Reload
            }
            KeyCode::Backspace => {
                input.pop();
                self.mode = Mode::Search { input };
                Action::None
            }
            KeyCode::Char(character) if accepts_text(key.modifiers) => {
                input.push(character);
                self.mode = Mode::Search { input };
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_filter_key(
        &mut self,
        key: KeyEvent,
        mut minimum: String,
        mut maximum: String,
        mut field: FilterField,
    ) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Tab | KeyCode::BackTab => {
                field = match field {
                    FilterField::Minimum => FilterField::Maximum,
                    FilterField::Maximum => FilterField::Minimum,
                };
                self.mode = Mode::Filter {
                    minimum,
                    maximum,
                    field,
                };
                Action::None
            }
            KeyCode::Backspace => {
                active_filter_value(&mut minimum, &mut maximum, field).pop();
                self.mode = Mode::Filter {
                    minimum,
                    maximum,
                    field,
                };
                Action::None
            }
            KeyCode::Enter => {
                let parsed_minimum = parse_width(&minimum, "minimum width");
                let parsed_maximum = parse_width(&maximum, "maximum width");
                match (parsed_minimum, parsed_maximum) {
                    (Ok(min_width), Ok(max_width)) => {
                        let filter = QuoteFilter {
                            search: self.filter.search.clone(),
                            min_width,
                            max_width,
                        };
                        if let Err(error) = filter.validate() {
                            self.status = error.to_string();
                            self.mode = Mode::Filter {
                                minimum,
                                maximum,
                                field,
                            };
                            return Action::None;
                        }
                        self.filter = filter;
                        self.mode = Mode::Browse;
                        self.status = String::from("Width filter updated");
                        Action::Reload
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        self.status = error.to_string();
                        self.mode = Mode::Filter {
                            minimum,
                            maximum,
                            field,
                        };
                        Action::None
                    }
                }
            }
            KeyCode::Char(character) if accepts_text(key.modifiers) => {
                active_filter_value(&mut minimum, &mut maximum, field).push(character);
                self.mode = Mode::Filter {
                    minimum,
                    maximum,
                    field,
                };
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_quote_form_key(
        &mut self,
        key: KeyEvent,
        id: Option<i64>,
        mut text: String,
        mut attribution: String,
        mut field: FormField,
    ) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Tab | KeyCode::BackTab => {
                field = match field {
                    FormField::Text => FormField::Attribution,
                    FormField::Attribution => FormField::Text,
                };
                self.set_quote_mode(id, text, attribution, field);
                Action::None
            }
            KeyCode::Backspace => {
                active_quote_value(&mut text, &mut attribution, field).pop();
                self.set_quote_mode(id, text, attribution, field);
                Action::None
            }
            KeyCode::Enter => match normalize_required(&text, "quote text") {
                Ok(text) => {
                    let attribution =
                        (!attribution.trim().is_empty()).then(|| attribution.trim().to_owned());
                    self.mode = Mode::Browse;
                    match id {
                        Some(id) => Action::Edit {
                            id,
                            text,
                            attribution,
                        },
                        None => Action::Add { text, attribution },
                    }
                }
                Err(error) => {
                    self.status = error.to_string();
                    self.set_quote_mode(id, text, attribution, field);
                    Action::None
                }
            },
            KeyCode::Char(character) if accepts_text(key.modifiers) => {
                active_quote_value(&mut text, &mut attribution, field).push(character);
                self.set_quote_mode(id, text, attribution, field);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_delete_key(&mut self, key: KeyEvent, id: i64) -> Action {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if key.modifiers.is_empty() => {
                self.mode = Mode::Browse;
                Action::Delete { id }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc if key.modifiers.is_empty() => {
                self.mode = Mode::Browse;
                self.status = String::from("Deletion cancelled");
                Action::None
            }
            _ => Action::None,
        }
    }

    fn set_quote_mode(
        &mut self,
        id: Option<i64>,
        text: String,
        attribution: String,
        field: FormField,
    ) {
        self.mode = match id {
            Some(id) => Mode::Edit {
                id,
                text,
                attribution,
                field,
            },
            None => Mode::Add {
                text,
                attribution,
                field,
            },
        };
    }

    fn move_selection(&mut self, change: isize) {
        if self.quotes.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected.unwrap_or_default();
        self.selected = Some(if change < 0 {
            current.checked_sub(1).unwrap_or(self.quotes.len() - 1)
        } else {
            (current + 1) % self.quotes.len()
        });
    }
}

pub fn run(database: &mut Database) -> Result<()> {
    let quotes = database.list(&QuoteFilter::default())?;
    let mut state = AppState::new(quotes);
    let mut guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("could not initialize terminal renderer")?;
    let run_result = run_loop(&mut terminal, database, &mut state);
    finish_session(run_result, || guard.restore())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    database: &mut Database,
    state: &mut AppState,
) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, state))?;
        if let Event::Key(key) = event::read()? {
            let action = state.handle_key(key);
            if action == Action::Quit {
                return Ok(());
            }
            perform_action(database, state, action);
        }
    }
}

fn perform_action(database: &Database, state: &mut AppState, action: Action) {
    match action {
        Action::None | Action::Quit => {}
        Action::Reload => {
            refresh(database, state, None);
        }
        Action::Random => match database.random(&state.filter) {
            Ok(Some(quote)) => {
                state.select_id(quote.id);
                state.status = format!("Selected random quote {}", quote.id);
            }
            Ok(None) => state.status = String::from("No quotes match the current filters"),
            Err(error) => state.status = format!("Could not select a quote: {error:#}"),
        },
        Action::Add { text, attribution } => match database.add(&text, attribution.as_deref()) {
            Ok(InsertOutcome::Added(quote)) => {
                let id = quote.id;
                if refresh(database, state, Some(id)) {
                    state.status = format!("Added quote {id}");
                }
            }
            Ok(InsertOutcome::Duplicate(id)) => {
                if refresh(database, state, Some(id)) {
                    state.status = format!("Skipped duplicate quote {id}");
                }
            }
            Err(error) => state.status = format!("Could not add quote: {error:#}"),
        },
        Action::Edit {
            id,
            text,
            attribution,
        } => {
            let attribution = match attribution {
                Some(attribution) => AttributionUpdate::Set(attribution),
                None => AttributionUpdate::Clear,
            };
            match database.edit(id, Some(&text), attribution) {
                Ok(_) => {
                    if refresh(database, state, Some(id)) {
                        state.status = format!("Updated quote {id}");
                    }
                }
                Err(error) => state.status = format!("Could not edit quote: {error:#}"),
            }
        }
        Action::Delete { id } => match database.remove(id) {
            Ok(true) => {
                if refresh(database, state, None) {
                    state.status = format!("Deleted quote {id}");
                }
            }
            Ok(false) => state.status = format!("Quote {id} no longer exists"),
            Err(error) => state.status = format!("Could not delete quote: {error:#}"),
        },
    }
}

fn refresh(database: &Database, state: &mut AppState, preferred_id: Option<i64>) -> bool {
    match database.list(&state.filter) {
        Ok(quotes) => {
            state.replace_quotes(quotes, preferred_id);
            true
        }
        Err(error) => {
            state.status = format!("Could not refresh quotes: {error:#}");
            false
        }
    }
}

fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .split(area);
    let panes = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(sections[1]);

    frame.render_widget(render_header(state), sections[0]);
    render_list(frame, state, panes[0]);
    render_details(frame, state, panes[1]);
    frame.render_widget(render_footer(state), sections[2]);
    render_modal(frame, state, area);
}

fn render_header(state: &AppState) -> Paragraph<'_> {
    let search = state.filter.normalized_search().unwrap_or("all");
    let minimum = state
        .filter
        .min_width
        .map_or_else(|| "0".to_owned(), |value| value.to_string());
    let maximum = state
        .filter
        .max_width
        .map_or_else(|| "∞".to_owned(), |value| value.to_string());
    Paragraph::new(format!(
        " quotes  search: {search}  width: {minimum}..{maximum}  {} match(es)",
        state.quotes.len()
    ))
    .style(Style::default().fg(Color::Black).bg(Color::Cyan))
}

fn render_list(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let items = state
        .quotes
        .iter()
        .map(|quote| {
            ListItem::new(format!(
                "#{:<4} [{:>3}] {}",
                quote.id,
                quote.display_width,
                quote.rendered()
            ))
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(Block::default().title(" Quotes ").borders(Borders::ALL))
        .highlight_symbol("▶ ")
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut list_state = ListState::default().with_selected(state.selected);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_details(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let text = match state.selected_quote() {
        Some(quote) => Text::from(vec![
            Line::from(vec![
                Span::styled("ID: ", Style::default().fg(Color::Cyan)),
                Span::raw(quote.id.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Displayed width: ", Style::default().fg(Color::Cyan)),
                Span::raw(quote.display_width.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Created: ", Style::default().fg(Color::Cyan)),
                Span::raw(quote.created_at.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Updated: ", Style::default().fg(Color::Cyan)),
                Span::raw(quote.updated_at.to_string()),
            ]),
            Line::from(""),
            Line::styled(&quote.text, Style::default().add_modifier(Modifier::BOLD)),
            Line::from(""),
            Line::from(
                quote
                    .attribution
                    .as_deref()
                    .map_or_else(|| "No attribution".to_owned(), |value| format!("— {value}")),
            ),
        ]),
        None => Text::from("No quote selected. Press a to add one."),
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().title(" Details ").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(state: &AppState) -> Paragraph<'_> {
    Paragraph::new(vec![
        Line::from(
            " ↑/k ↓/j move  / search  f width  r random  a add  e edit  d delete  Esc clear  q quit",
        ),
        Line::styled(
            format!(" {}", state.status),
            Style::default().fg(Color::Yellow),
        ),
    ])
}

fn render_modal(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    if state.mode == Mode::Browse {
        return;
    }
    let popup = centered_rect(72, 45, area);
    frame.render_widget(Clear, popup);
    match &state.mode {
        Mode::Browse => {}
        Mode::Search { input } => render_form(
            frame,
            popup,
            " Search ",
            vec![
                active_line("Query", input, true),
                help_line("Enter apply • Esc cancel"),
            ],
        ),
        Mode::Filter {
            minimum,
            maximum,
            field,
        } => render_form(
            frame,
            popup,
            " Width filter ",
            vec![
                active_line("Minimum", minimum, *field == FilterField::Minimum),
                active_line("Maximum", maximum, *field == FilterField::Maximum),
                help_line("Blank means unbounded • Tab fields • Enter apply • Esc cancel"),
            ],
        ),
        Mode::Add {
            text,
            attribution,
            field,
        } => render_form(
            frame,
            popup,
            " Add quote ",
            vec![
                active_line("Text", text, *field == FormField::Text),
                active_line("Attribution", attribution, *field == FormField::Attribution),
                help_line("Attribution is optional • Tab fields • Enter save • Esc cancel"),
            ],
        ),
        Mode::Edit {
            id,
            text,
            attribution,
            field,
        } => render_form(
            frame,
            popup,
            &format!(" Edit quote {id} "),
            vec![
                active_line("Text", text, *field == FormField::Text),
                active_line("Attribution", attribution, *field == FormField::Attribution),
                help_line(
                    "Leave attribution blank to clear • Tab fields • Enter save • Esc cancel",
                ),
            ],
        ),
        Mode::ConfirmDelete { id } => render_form(
            frame,
            popup,
            " Confirm deletion ",
            vec![
                Line::from(format!("Delete quote {id}? This cannot be undone.")),
                help_line("Press y to delete, n or Esc to cancel"),
            ],
        ),
    }
}

fn render_form(frame: &mut Frame<'_>, area: Rect, title: &str, lines: Vec<Line<'_>>) {
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn active_line<'a>(label: &'a str, value: &'a str, active: bool) -> Line<'a> {
    let marker = if active { "▶" } else { " " };
    Line::from(vec![
        Span::styled(
            format!("{marker} {label}: "),
            Style::default().fg(if active { Color::Yellow } else { Color::Gray }),
        ),
        Span::raw(value),
    ])
}

fn help_line(text: &str) -> Line<'_> {
    Line::styled(text, Style::default().fg(Color::DarkGray))
}

fn centered_rect(horizontal_percent: u16, vertical_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - vertical_percent) / 2),
        Constraint::Percentage(vertical_percent),
        Constraint::Percentage((100 - vertical_percent) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - horizontal_percent) / 2),
        Constraint::Percentage(horizontal_percent),
        Constraint::Percentage((100 - horizontal_percent) / 2),
    ])
    .split(vertical[1])[1]
}

fn parse_width(input: &str, label: &str) -> Result<Option<u32>> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let value: i64 = input
        .parse()
        .with_context(|| format!("{label} must be a non-negative integer"))?;
    if value < 0 {
        return Err(anyhow!("{label} cannot be negative"));
    }
    Ok(Some(
        u32::try_from(value).with_context(|| format!("{label} is too large"))?,
    ))
}

fn active_filter_value<'a>(
    minimum: &'a mut String,
    maximum: &'a mut String,
    field: FilterField,
) -> &'a mut String {
    match field {
        FilterField::Minimum => minimum,
        FilterField::Maximum => maximum,
    }
}

fn active_quote_value<'a>(
    text: &'a mut String,
    attribution: &'a mut String,
    field: FormField,
) -> &'a mut String {
    match field {
        FormField::Text => text,
        FormField::Attribution => attribution,
    }
}

fn accepts_text(modifiers: KeyModifiers) -> bool {
    !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("could not enable terminal raw mode")?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("could not enter the alternate terminal screen");
        }
        Ok(Self { active: true })
    }

    fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        let raw_result = disable_raw_mode().context("could not disable terminal raw mode");
        let screen_result = execute!(io::stdout(), LeaveAlternateScreen, Show)
            .context("could not restore the terminal screen");
        self.active = false;
        raw_result.and(screen_result)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn finish_session<T>(run_result: Result<T>, cleanup: impl FnOnce() -> Result<()>) -> Result<T> {
    let cleanup_result = cleanup();
    match (run_result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(run_error), Err(cleanup_error)) => {
            Err(run_error.context(format!("terminal cleanup also failed: {cleanup_error:#}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn quote(id: i64, text: &str, attribution: Option<&str>) -> Quote {
        Quote {
            id,
            text: text.into(),
            attribution: attribution.map(str::to_owned),
            display_width: text.len() as u32,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn browse_navigation_wraps_and_supports_j_and_k() {
        let mut state = AppState::new(vec![quote(1, "one", None), quote(2, "two", None)]);
        state.handle_key(key(KeyCode::Char('j')));
        assert_eq!(state.selected_quote().unwrap().id, 2);
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.selected_quote().unwrap().id, 1);
        state.handle_key(key(KeyCode::Char('k')));
        assert_eq!(state.selected_quote().unwrap().id, 2);
    }

    #[test]
    fn search_mode_updates_filter_and_requests_reload() {
        let mut state = AppState::new(Vec::new());
        state.handle_key(key(KeyCode::Char('/')));
        state.handle_key(key(KeyCode::Char('c')));
        state.handle_key(key(KeyCode::Char('a')));
        state.handle_key(key(KeyCode::Char('t')));
        assert_eq!(state.handle_key(key(KeyCode::Enter)), Action::Reload);
        assert_eq!(state.filter.search.as_deref(), Some("cat"));
        assert_eq!(state.mode, Mode::Browse);
    }

    #[test]
    fn filter_form_rejects_negative_and_reversed_bounds() {
        let mut state = AppState::new(Vec::new());
        state.mode = Mode::Filter {
            minimum: "-1".into(),
            maximum: String::new(),
            field: FilterField::Minimum,
        };
        assert_eq!(state.handle_key(key(KeyCode::Enter)), Action::None);
        assert!(state.status.contains("cannot be negative"));
        assert!(matches!(state.mode, Mode::Filter { .. }));

        state.mode = Mode::Filter {
            minimum: "10".into(),
            maximum: "4".into(),
            field: FilterField::Maximum,
        };
        assert_eq!(state.handle_key(key(KeyCode::Enter)), Action::None);
        assert!(state.status.contains("cannot exceed"));
    }

    #[test]
    fn add_form_validates_text_before_submitting() {
        let mut state = AppState::new(Vec::new());
        state.mode = Mode::Add {
            text: "  ".into(),
            attribution: String::new(),
            field: FormField::Text,
        };
        assert_eq!(state.handle_key(key(KeyCode::Enter)), Action::None);
        assert!(state.status.contains("cannot be empty"));

        state.mode = Mode::Add {
            text: "  hello  ".into(),
            attribution: " author ".into(),
            field: FormField::Attribution,
        };
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            Action::Add {
                text: "hello".into(),
                attribution: Some("author".into())
            }
        );
    }

    #[test]
    fn edit_form_can_clear_attribution_and_delete_requires_confirmation() {
        let mut state = AppState::new(vec![quote(7, "hello", Some("author"))]);
        state.mode = Mode::Edit {
            id: 7,
            text: "hello".into(),
            attribution: String::new(),
            field: FormField::Attribution,
        };
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            Action::Edit {
                id: 7,
                text: "hello".into(),
                attribution: None
            }
        );

        state.mode = Mode::ConfirmDelete { id: 7 };
        assert_eq!(
            state.handle_key(key(KeyCode::Char('y'))),
            Action::Delete { id: 7 }
        );
    }

    #[test]
    fn cleanup_runs_after_normal_and_error_results() {
        let cleaned = Cell::new(false);
        let result = finish_session(Ok(42), || {
            cleaned.set(true);
            Ok(())
        });
        assert_eq!(result.unwrap(), 42);
        assert!(cleaned.get());

        cleaned.set(false);
        let result: Result<()> = finish_session(Err(anyhow!("render failed")), || {
            cleaned.set(true);
            Ok(())
        });
        assert!(result.is_err());
        assert!(cleaned.get());
    }
}
