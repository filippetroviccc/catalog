use crate::analyze::{BrowseEntry, BrowseIndex, human_size};
use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::Frame;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

pub fn run_browse_tui(index: &BrowseIndex, start_path: Option<PathBuf>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = BrowserApp::new(index, start_path);
    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

struct BrowserApp<'a> {
    index: &'a BrowseIndex,
    base_path: Option<PathBuf>,
    current_path: Option<PathBuf>,
    history: Vec<Option<PathBuf>>,
    entries: Vec<BrowseEntry>,
    state: ListState,
    list_area: Rect,
}

impl<'a> BrowserApp<'a> {
    fn new(index: &'a BrowseIndex, start_path: Option<PathBuf>) -> Self {
        let base_path = start_path;
        let current_path = base_path.clone();
        let entries = index.children_for(current_path.as_deref());
        let mut state = ListState::default();
        if !entries.is_empty() {
            state.select(Some(0));
        }
        Self {
            index,
            base_path,
            current_path,
            history: Vec::new(),
            entries,
            state,
            list_area: Rect::default(),
        }
    }

    fn refresh(&mut self) {
        self.entries = self.index.children_for(self.current_path.as_deref());
        let selected = self.state.selected().unwrap_or(0);
        if self.entries.is_empty() {
            self.state.select(None);
        } else if selected >= self.entries.len() {
            self.state.select(Some(0));
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let max = (self.entries.len() - 1) as isize;
        let current = self.state.selected().unwrap_or(0) as isize;
        let mut next = current + delta;
        if next < 0 {
            next = 0;
        } else if next > max {
            next = max;
        }
        self.state.select(Some(next as usize));
    }

    fn move_to(&mut self, idx: usize) {
        if self.entries.is_empty() {
            return;
        }
        let clamped = idx.min(self.entries.len() - 1);
        self.state.select(Some(clamped));
    }

    fn open_selected(&mut self) {
        let Some(idx) = self.state.selected() else { return; };
        let entry = match self.entries.get(idx) {
            Some(e) => e,
            None => return,
        };
        if !entry.is_dir {
            return;
        }
        self.history.push(self.current_path.clone());
        self.current_path = Some(entry.path.clone());
        self.refresh();
        self.state.select(if self.entries.is_empty() { None } else { Some(0) });
    }

    fn go_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.current_path = prev;
            self.refresh();
            self.state.select(if self.entries.is_empty() { None } else { Some(0) });
        }
    }

    fn current_label(&self) -> String {
        match &self.current_path {
            Some(path) => path.to_string_lossy().to_string(),
            None => "(roots)".to_string(),
        }
    }

    fn total_label(&self) -> String {
        let total = self.index.total_for(self.current_path.as_deref());
        human_size(total)
    }

    fn display_name(&self, entry: &BrowseEntry) -> String {
        if self.current_path.is_none() {
            entry.path.to_string_lossy().to_string()
        } else {
            entry
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.to_string_lossy().to_string())
        }
    }

    fn can_go_back(&self) -> bool {
        !self.history.is_empty() || (self.base_path.is_none() && self.current_path.is_some())
    }

    fn set_selection_from_mouse(&mut self, row: u16) {
        if row < self.list_area.y || row >= self.list_area.y + self.list_area.height {
            return;
        }
        let idx = (row - self.list_area.y) as usize;
        if idx < self.entries.len() {
            self.state.select(Some(idx));
        }
    }

    fn open_at_mouse(&mut self, row: u16) {
        if row < self.list_area.y || row >= self.list_area.y + self.list_area.height {
            return;
        }
        let idx = (row - self.list_area.y) as usize;
        if idx >= self.entries.len() {
            return;
        }
        self.state.select(Some(idx));
        if self.entries[idx].is_dir {
            self.open_selected();
        }
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut BrowserApp) -> Result<()> {
    loop {
        terminal.draw(|f| draw_ui(f, app))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(app, key) {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        app.open_at_mouse(mouse.row);
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        app.set_selection_from_mouse(mouse.row);
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {}
                Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
            }
        }
    }
}

fn handle_key(app: &mut BrowserApp, key: KeyEvent) -> bool {
    match key {
        KeyEvent {
            code: KeyCode::Char('q'),
            ..
        } => return true,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => return true,
        KeyEvent {
            code: KeyCode::Up, ..
        }
        | KeyEvent {
            code: KeyCode::Char('k'),
            ..
        } => app.move_selection(-1),
        KeyEvent {
            code: KeyCode::Down,
            ..
        }
        | KeyEvent {
            code: KeyCode::Char('j'),
            ..
        } => app.move_selection(1),
        KeyEvent {
            code: KeyCode::PageUp,
            ..
        } => app.move_selection(-10),
        KeyEvent {
            code: KeyCode::PageDown,
            ..
        } => app.move_selection(10),
        KeyEvent {
            code: KeyCode::Home,
            ..
        } => app.move_to(0),
        KeyEvent {
            code: KeyCode::End, ..
        } => {
            if !app.entries.is_empty() {
                app.move_to(app.entries.len() - 1)
            }
        }
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => app.open_selected(),
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        }
        | KeyEvent {
            code: KeyCode::Left,
            ..
        }
        | KeyEvent {
            code: KeyCode::Char('b'),
            ..
        } => {
            if app.can_go_back() {
                app.go_back();
            }
        }
        _ => {}
    }
    false
}

fn draw_ui(frame: &mut Frame, app: &mut BrowserApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.size());

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Path: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(app.current_label()),
        ]),
        Line::from(vec![
            Span::styled("Total: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(app.total_label()),
            Span::raw(format!("  Items: {}", app.entries.len())),
        ]),
    ]);
    frame.render_widget(header, chunks[0]);

    let items = if app.entries.is_empty() {
        vec![ListItem::new("(empty)")]
    } else {
        let max_size_len = app
            .entries
            .iter()
            .map(|e| human_size(e.size).len())
            .max()
            .unwrap_or(1);
        app.entries
            .iter()
            .map(|entry| {
                let size = human_size(entry.size);
                let name = app.display_name(entry);
                let label = if entry.is_dir { format!("{}/", name) } else { name };
                let line = format!("{:>width$}  {}", size, label, width = max_size_len);
                ListItem::new(line)
            })
            .collect()
    };

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White));
    app.list_area = chunks[1];
    frame.render_stateful_widget(list, chunks[1], &mut app.state);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": open  "),
        Span::styled("Backspace", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": up  "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": quit  "),
        Span::styled("Mouse", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": click to open"),
    ]));
    frame.render_widget(footer, chunks[2]);
}
