//! Interactive notebook browser backed by the same desktop or SSH profile as the CLI.
mod editor;
mod view;

use crate::{cli::PageArgs, config::Profile, desktop, error::Error, model, write};
use editor::Editor;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        cursor::Show,
        event::{
            self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent,
            KeyEventKind, KeyModifiers,
        },
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, IsTerminal},
    time::Duration,
};
use tokio::task::JoinHandle;

const PAGE_SIZE: u16 = 50;

struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self, Error> {
        enable_raw_mode()?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    name: String,
    path: String,
    locked: bool,
    modified: Option<String>,
}
impl Entry {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value["id"].as_str()?.to_owned(),
            name: value["name"].as_str().unwrap_or("Untitled").to_owned(),
            path: value["path"].as_str().unwrap_or("").to_owned(),
            locked: value["locked"].as_bool().unwrap_or(false),
            modified: value["last_modified"].as_str().map(str::to_owned),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Location {
    Notebooks,
    Sections(Entry),
    Pages { notebook: Entry, section: Entry },
    Search { query: String, scan: bool },
}
impl Location {
    fn key(&self) -> String {
        match self {
            Self::Notebooks => "notebooks".into(),
            Self::Sections(book) => format!("sections:{}", book.id),
            Self::Pages { section, .. } => format!("pages:{}", section.id),
            Self::Search { query, scan } => format!("search:{scan}:{query}"),
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Notebooks => "Notebooks".into(),
            Self::Sections(book) => format!("{}  /  Sections", book.name),
            Self::Pages { notebook, section } => format!("{}  /  {}", notebook.name, section.name),
            Self::Search { query, scan } => format!(
                "{} search  /  {query}",
                if *scan { "Literal" } else { "Indexed" }
            ),
        }
    }
    fn heading(&self) -> &'static str {
        match self {
            Self::Notebooks => "NOTEBOOKS",
            Self::Sections(_) => "SECTIONS",
            Self::Pages { .. } => "PAGES",
            Self::Search { .. } => "SEARCH RESULTS",
        }
    }
}
#[derive(Clone)]
enum Request {
    List {
        location: Location,
        offset: u32,
    },
    Read {
        page: Entry,
    },
    Write {
        kind: WriteKind,
        target: String,
        title: Option<String>,
        text: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteKind {
    Create,
    Append,
}
impl WriteKind {
    fn operation(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Append => "append",
        }
    }
}
enum Response {
    List {
        rows: Vec<Entry>,
        total: usize,
        next: Option<u32>,
        warning: Option<String>,
    },
    Read(Value),
    Written(Value),
}
struct Job {
    request: Request,
    task: JoinHandle<Result<Response, Error>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Indexed,
    Literal,
    Filter,
}
struct Compose {
    kind: WriteKind,
    target: String,
    target_name: String,
    title: Editor,
    body: Editor,
    title_focus: bool,
    review_scroll: u16,
}
impl Compose {
    fn new(
        kind: WriteKind,
        target: String,
        target_name: String,
        old: Option<(String, String)>,
    ) -> Self {
        let (title, body) = old.unwrap_or_default();
        Self {
            kind,
            target,
            target_name,
            title: Editor::new(title, false),
            body: Editor::new(body, true),
            title_focus: kind == WriteKind::Create,
            review_scroll: 0,
        }
    }
    fn active(&mut self) -> &mut Editor {
        if self.title_focus {
            &mut self.title
        } else {
            &mut self.body
        }
    }
    fn draft(&self) -> (String, String) {
        (self.title.text().to_owned(), self.body.text().to_owned())
    }
}
enum Overlay {
    None,
    Help,
    Input { kind: InputKind, editor: Editor },
    Compose(Compose),
    Review(Compose),
}
struct App {
    profile_name: String,
    profile: Profile,
    color: bool,
    location: Location,
    return_to: Option<Location>,
    rows: Vec<Entry>,
    selected: usize,
    filter: String,
    total: usize,
    next: Option<u32>,
    page: Option<Value>,
    page_scroll: u16,
    page_max_scroll: u16,
    full_page: bool,
    loading: bool,
    writing: bool,
    notice: String,
    error: Option<String>,
    uncertain: bool,
    drafts: HashMap<String, (String, String)>,
    selections: HashMap<String, String>,
    overlay: Overlay,
}
impl App {
    fn new(profile: Profile, name: Option<&str>, color: bool) -> Self {
        Self {
            profile_name: name.unwrap_or("default").into(),
            profile,
            color,
            location: Location::Notebooks,
            return_to: None,
            rows: vec![],
            selected: 0,
            filter: String::new(),
            total: 0,
            next: None,
            page: None,
            page_scroll: 0,
            page_max_scroll: 0,
            full_page: false,
            loading: false,
            writing: false,
            notice: "Choose a notebook · press ? for keys".into(),
            error: None,
            uncertain: false,
            drafts: HashMap::new(),
            selections: HashMap::new(),
            overlay: Overlay::None,
        }
    }
    fn selected_entry(&self) -> Option<&Entry> {
        self.rows.get(self.selected)
    }
    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                needle.is_empty()
                    || row.name.to_lowercase().contains(&needle)
                    || row.path.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }
    fn move_selection(&mut self, delta: isize) -> Action {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return Action::None;
        }
        let current = visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
        if self.selected == visible[next] {
            return Action::None;
        }
        self.selected = visible[next];
        self.page = None;
        self.full_page = false;
        self.page_scroll = 0;
        self.read_request().map_or(Action::None, Action::Fetch)
    }
    fn read_request(&self) -> Option<Request> {
        if matches!(
            self.location,
            Location::Pages { .. } | Location::Search { .. }
        ) {
            self.selected_entry()
                .cloned()
                .map(|page| Request::Read { page })
        } else {
            None
        }
    }
    fn navigate(&mut self, location: Location) -> Action {
        if let Some(row) = self.selected_entry() {
            self.selections.insert(self.location.key(), row.id.clone());
        }
        self.location = location.clone();
        self.rows.clear();
        self.selected = 0;
        self.total = 0;
        self.next = None;
        self.page = None;
        self.page_scroll = 0;
        self.page_max_scroll = 0;
        self.full_page = false;
        self.filter.clear();
        self.error = None;
        Action::Fetch(Request::List {
            location,
            offset: 0,
        })
    }
    fn back(&mut self) -> Action {
        if self.full_page {
            self.full_page = false;
            return Action::None;
        }
        match &self.location {
            Location::Notebooks => Action::None,
            Location::Sections(_) => self.navigate(Location::Notebooks),
            Location::Pages { notebook, .. } => self.navigate(Location::Sections(notebook.clone())),
            Location::Search { .. } => {
                let location = self.return_to.take().unwrap_or(Location::Notebooks);
                self.navigate(location)
            }
        }
    }
    fn enter(&mut self) -> Action {
        let Some(row) = self.selected_entry().cloned() else {
            return Action::None;
        };
        match &self.location {
            Location::Notebooks => self.navigate(Location::Sections(row)),
            Location::Sections(book) => {
                if row.locked {
                    self.notice = "Section is locked · unlock it in OneNote desktop".into();
                    Action::None
                } else {
                    self.navigate(Location::Pages {
                        notebook: book.clone(),
                        section: row,
                    })
                }
            }
            Location::Pages { .. } | Location::Search { .. } => {
                self.full_page = true;
                if self.page.as_ref().and_then(|v| v["id"].as_str()) == Some(row.id.as_str()) {
                    Action::None
                } else {
                    Action::Fetch(Request::Read { page: row })
                }
            }
        }
    }
    fn complete(&mut self, request: &Request, result: Result<Response, Error>) -> Option<Request> {
        self.loading = false;
        self.writing = false;
        let response = match result {
            Ok(response) => {
                self.error = None;
                response
            }
            Err(error) => {
                self.error = Some(error.message);
                if matches!(request, Request::Write { .. }) {
                    self.uncertain = true;
                    self.notice =
                        "Write result unknown · inspect OneNote before another write".into();
                } else {
                    self.notice = "Request failed · r retry".into();
                }
                return None;
            }
        };
        match (request, response) {
            (
                Request::List { location, offset },
                Response::List {
                    rows,
                    total,
                    next,
                    warning,
                },
            ) if self.location == *location => {
                if *offset == 0 {
                    self.rows.clear();
                    self.selected = 0;
                    self.page = None;
                }
                for row in rows {
                    if !self.rows.iter().any(|old| old.id == row.id) {
                        self.rows.push(row);
                    }
                }
                self.total = total;
                self.next = next;
                if *offset == 0
                    && let Some(saved) = self.selections.get(&location.key())
                    && let Some(index) = self.rows.iter().position(|row| &row.id == saved)
                {
                    self.selected = index;
                }
                self.notice = warning.unwrap_or_else(|| {
                    format!(
                        "{} of {} {} loaded",
                        self.rows.len(),
                        total,
                        location.heading().to_lowercase()
                    )
                });
                if *offset == 0 {
                    return self.read_request();
                }
            }
            (Request::Read { page }, Response::Read(value))
                if self.selected_entry().is_some_and(|row| row.id == page.id) =>
            {
                self.page = Some(value);
                self.page_scroll = 0;
                self.page_max_scroll = 0;
                self.notice = format!("{} · ready", page.name);
            }
            (
                Request::Write {
                    kind,
                    target,
                    title,
                    ..
                },
                Response::Written(value),
            ) => {
                self.drafts
                    .remove(&format!("{}:{target}", kind.operation()));
                if *kind == WriteKind::Create
                    && let Some(row) = Entry::from_value(&value)
                {
                    self.rows.retain(|old| old.id != row.id);
                    self.rows.insert(0, row);
                    self.selected = 0;
                    self.total += 1;
                    self.filter.clear();
                }
                if *kind == WriteKind::Append
                    && let Some(index) = self.rows.iter().position(|row| row.id == *target)
                {
                    self.selected = index;
                }
                self.page = Some(value);
                self.page_scroll = 0;
                self.page_max_scroll = 0;
                self.notice = match kind {
                    WriteKind::Create => format!(
                        "Created {} · verified in OneNote",
                        title.as_deref().unwrap_or("page")
                    ),
                    WriteKind::Append => "Append verified in OneNote".into(),
                };
            }
            _ => {}
        }
        None
    }
    fn key(&mut self, key: KeyEvent) -> Action {
        if self.writing {
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::Help => return Action::None,
            Overlay::Input { kind, mut editor } => {
                if key.code == KeyCode::Esc {
                    return Action::None;
                }
                if key.code == KeyCode::Enter {
                    let text = editor.text().trim().to_owned();
                    match kind {
                        InputKind::Filter => {
                            self.filter = text;
                            if let Some(first) = self.visible_indices().first() {
                                self.selected = *first;
                                self.page = None;
                                return self.read_request().map_or(Action::None, Action::Fetch);
                            }
                            return Action::None;
                        }
                        InputKind::Indexed | InputKind::Literal if !text.is_empty() => {
                            if !matches!(self.location, Location::Search { .. }) {
                                self.return_to = Some(self.location.clone());
                            }
                            return self.navigate(Location::Search {
                                query: text,
                                scan: kind == InputKind::Literal,
                            });
                        }
                        _ => {}
                    }
                }
                editor.key(key);
                self.overlay = Overlay::Input { kind, editor };
                return Action::None;
            }
            Overlay::Compose(mut draft) => {
                if key.code == KeyCode::Esc {
                    self.drafts.insert(
                        format!("{}:{}", draft.kind.operation(), draft.target),
                        draft.draft(),
                    );
                    self.notice = "Draft kept for this session".into();
                    return Action::None;
                }
                if key.code == KeyCode::Tab {
                    draft.title_focus = !draft.title_focus && draft.kind == WriteKind::Create;
                    self.overlay = Overlay::Compose(draft);
                    return Action::None;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('s') | KeyCode::Enter)
                {
                    if draft.kind == WriteKind::Create
                        && let Err(e) = write::validate(draft.title.text(), true)
                    {
                        self.error = Some(e.message);
                        self.overlay = Overlay::Compose(draft);
                        return Action::None;
                    }
                    if let Err(e) = write::validate(draft.body.text(), false) {
                        self.error = Some(e.message);
                        self.overlay = Overlay::Compose(draft);
                        return Action::None;
                    }
                    self.error = None;
                    self.overlay = Overlay::Review(draft);
                    return Action::None;
                }
                draft.active().key(key);
                self.error = None;
                self.overlay = Overlay::Compose(draft);
                return Action::None;
            }
            Overlay::Review(mut draft) => {
                match key.code {
                    KeyCode::Esc => {
                        self.overlay = Overlay::Compose(draft);
                        return Action::None;
                    }
                    KeyCode::Enter => {
                        let title = if draft.kind == WriteKind::Create {
                            Some(draft.title.text().to_owned())
                        } else {
                            None
                        };
                        let text = draft.body.text().to_owned();
                        self.drafts.insert(
                            format!("{}:{}", draft.kind.operation(), draft.target),
                            draft.draft(),
                        );
                        return Action::Fetch(Request::Write {
                            kind: draft.kind,
                            target: draft.target,
                            title,
                            text,
                        });
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        draft.review_scroll = draft.review_scroll.saturating_add(1)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        draft.review_scroll = draft.review_scroll.saturating_sub(1)
                    }
                    KeyCode::PageDown => {
                        draft.review_scroll = draft.review_scroll.saturating_add(8)
                    }
                    KeyCode::PageUp => draft.review_scroll = draft.review_scroll.saturating_sub(8),
                    KeyCode::Home => draft.review_scroll = 0,
                    KeyCode::End => draft.review_scroll = u16::MAX,
                    _ => {}
                }
                self.overlay = Overlay::Review(draft);
                return Action::None;
            }
            Overlay::None => {}
        }
        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') => {
                self.overlay = Overlay::Help;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Enter | KeyCode::Right => self.enter(),
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                if self.full_page {
                    self.back()
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    Action::None
                } else {
                    self.back()
                }
            }
            KeyCode::PageDown => {
                self.page_scroll = self.page_scroll.saturating_add(8).min(self.page_max_scroll);
                Action::None
            }
            KeyCode::PageUp => {
                self.page_scroll = self.page_scroll.saturating_sub(8);
                Action::None
            }
            KeyCode::Home => {
                self.page_scroll = 0;
                Action::None
            }
            KeyCode::End => {
                self.page_scroll = self.page_max_scroll;
                Action::None
            }
            KeyCode::Char('r') => Action::Fetch(Request::List {
                location: self.location.clone(),
                offset: 0,
            }),
            KeyCode::Char('n') if self.next.is_some() && !self.loading => {
                Action::Fetch(Request::List {
                    location: self.location.clone(),
                    offset: self.next.unwrap(),
                })
            }
            KeyCode::Char('/') => {
                self.overlay = Overlay::Input {
                    kind: InputKind::Indexed,
                    editor: Editor::new(String::new(), false),
                };
                Action::None
            }
            KeyCode::Char('s') => {
                self.overlay = Overlay::Input {
                    kind: InputKind::Literal,
                    editor: Editor::new(String::new(), false),
                };
                Action::None
            }
            KeyCode::Char('f') => {
                self.overlay = Overlay::Input {
                    kind: InputKind::Filter,
                    editor: Editor::new(self.filter.clone(), false),
                };
                Action::None
            }
            KeyCode::Char('b') => self.navigate(Location::Notebooks),
            KeyCode::Char('c') if !self.profile.read_only && !self.uncertain => {
                if let Location::Pages { section, .. } = &self.location {
                    let target = section.id.clone();
                    let name = section.name.clone();
                    let old = self.drafts.remove(&format!("create:{target}"));
                    self.overlay =
                        Overlay::Compose(Compose::new(WriteKind::Create, target, name, old));
                } else {
                    self.notice = "Open a section to create a page".into();
                }
                Action::None
            }
            KeyCode::Char('a') if !self.profile.read_only && !self.uncertain => {
                if let Some(page) = self
                    .selected_entry()
                    .filter(|_| {
                        matches!(
                            self.location,
                            Location::Pages { .. } | Location::Search { .. }
                        )
                    })
                    .cloned()
                {
                    let target = page.id.clone();
                    let old = self.drafts.remove(&format!("append:{target}"));
                    self.overlay =
                        Overlay::Compose(Compose::new(WriteKind::Append, target, page.name, old));
                } else {
                    self.notice = "Select a page to append text".into();
                }
                Action::None
            }
            KeyCode::Char('c' | 'a') if self.profile.read_only => {
                self.notice = "This profile is read-only".into();
                Action::None
            }
            KeyCode::Char('c' | 'a') if self.uncertain => {
                self.notice = "Write outcome unknown · inspect OneNote before another write".into();
                Action::None
            }
            _ => Action::None,
        }
    }
    fn paste(&mut self, text: &str) {
        let truncated = match &mut self.overlay {
            Overlay::Input { editor, .. } => editor.paste(text),
            Overlay::Compose(draft) => draft.active().paste(text),
            _ => false,
        };
        if truncated {
            self.error =
                Some("Paste reached the editor limit; review the text before continuing".into());
        }
    }
}
#[allow(clippy::large_enum_variant)] // A navigation request carries two small hierarchy records.
enum Action {
    None,
    Fetch(Request),
    Quit,
}

fn request_start(profile: Profile, request: Request, app: &mut App) -> Job {
    app.loading = true;
    app.writing = matches!(request, Request::Write { .. });
    app.error = None;
    app.notice = match &request {
        Request::List { .. } => "Loading from OneNote…",
        Request::Read { .. } => "Reading page…",
        Request::Write { .. } => "Writing to OneNote · please wait…",
    }
    .into();
    let task_request = request.clone();
    let task = tokio::spawn(async move {
        match task_request {
            Request::List { location, offset } => {
                let page = PageArgs {
                    limit: PAGE_SIZE,
                    offset,
                    fields: vec![],
                };
                let value = match &location {
                    Location::Notebooks => model::collection(
                        &desktop::run(&profile, json!({"operation":"notebooks"})).await?,
                        "Notebook",
                        &page,
                    )?,
                    Location::Sections(book) => model::collection(
                        &desktop::run(&profile, json!({"operation":"sections","scope":book.id}))
                            .await?,
                        "Section",
                        &page,
                    )?,
                    Location::Pages { section, .. } => model::collection(
                        &desktop::run(&profile, json!({"operation":"pages","scope":section.id}))
                            .await?,
                        "Page",
                        &page,
                    )?,
                    Location::Search { query, scan: false } => model::search(
                        &desktop::run(&profile, json!({"operation":"search","query":query}))
                            .await?,
                        &page,
                    )?,
                    Location::Search { query, scan: true } => {
                        let batch = desktop::scan(&profile, json!({"operation":"scan","scope":null,"scan_limit":100,"scan_offset":0})).await?;
                        model::scan(&batch, query, &page)?
                    }
                };
                let rows = value["items"]
                    .as_array()
                    .map(|v| v.iter().filter_map(Entry::from_value).collect())
                    .unwrap_or_default();
                let warning = if value["indexing_pending"] == true {
                    Some(format!(
                        "Search index is still updating · {} pages not indexed",
                        value["unindexed_count"]
                    ))
                } else if value["incomplete"] == true {
                    Some(
                        "Literal scan incomplete · refine your query or use the CLI for more"
                            .into(),
                    )
                } else {
                    None
                };
                Ok(Response::List {
                    rows,
                    total: value["total"].as_u64().unwrap_or(0) as usize,
                    next: value["next_offset"].as_u64().map(|n| n as u32),
                    warning,
                })
            }
            Request::Read { page } => {
                let xml = desktop::run(&profile, json!({"operation":"read","id":page.id})).await?;
                Ok(Response::Read(model::read(&xml, &page.id, false)?))
            }
            Request::Write {
                kind,
                target,
                title,
                text,
            } => {
                let args = crate::cli::ContentArgs {
                    text: Some(text),
                    file: None,
                    dry_run: false,
                };
                let payload = write::prepare(kind.operation(), &target, title.as_deref(), &args)?;
                let value = desktop::write(&profile, payload).await?;
                let id = value["id"].as_str().ok_or_else(|| {
                    Error::new("write_uncertain", "Write response has no page ID")
                })?;
                let xml = value["xml"].as_str().ok_or_else(|| {
                    Error::new("write_uncertain", "Write response has no page content")
                })?;
                if value["verified"] != true {
                    return Err(Error::new(
                        "write_uncertain",
                        "OneNote did not verify the write",
                    ));
                }
                Ok(Response::Written(
                    model::read(xml, id, false)
                        .map_err(|e| Error::new("write_uncertain", e.message))?,
                ))
            }
        }
    });
    Job { request, task }
}

pub async fn run(profile: Profile, name: Option<&str>, color: bool) -> Result<(), Error> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::invalid("`onenote tui` needs a terminal"));
    }
    let color = color
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").as_deref() != Ok("dumb");
    let mut app = App::new(profile.clone(), name, color);
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut job = Some(request_start(
        profile.clone(),
        Request::List {
            location: Location::Notebooks,
            offset: 0,
        },
        &mut app,
    ));
    let mut dirty = true;
    loop {
        if job.as_ref().is_some_and(|j| j.task.is_finished()) {
            let mut done = job.take().unwrap();
            let result = (&mut done.task)
                .await
                .unwrap_or_else(|e| Err(Error::desktop(format!("Request stopped: {e}"))));
            if let Some(next) = app.complete(&done.request, result) {
                job = Some(request_start(profile.clone(), next, &mut app));
            }
            dirty = true;
        }
        if dirty {
            terminal.draw(|frame| view::draw(frame, &mut app))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(40))? {
            dirty = true;
            let action = match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    app.key(key)
                }
                Event::Paste(text) => {
                    app.paste(&text);
                    Action::None
                }
                _ => Action::None,
            };
            match action {
                Action::Quit => break,
                Action::None => {}
                Action::Fetch(request) => {
                    drop(job.take());
                    job = Some(request_start(profile.clone(), request, &mut app));
                }
            }
        }
        tokio::task::yield_now().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn row(id: &str, name: &str) -> Entry {
        Entry {
            id: id.into(),
            name: name.into(),
            path: name.into(),
            locked: false,
            modified: None,
        }
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn drill_down_and_back_preserve_hierarchy() {
        let mut app = App::new(Profile::default(), None, false);
        app.rows = vec![row("book", "Notebook")];
        assert!(matches!(
            app.key(key(KeyCode::Enter)),
            Action::Fetch(Request::List {
                location: Location::Sections(_),
                ..
            })
        ));
        app.rows = vec![row("first", "First"), row("section", "Ideas")];
        app.selected = 1;
        assert!(matches!(
            app.key(key(KeyCode::Enter)),
            Action::Fetch(Request::List {
                location: Location::Pages { .. },
                ..
            })
        ));
        assert!(matches!(
            app.key(key(KeyCode::Esc)),
            Action::Fetch(Request::List {
                location: Location::Sections(_),
                ..
            })
        ));
        let location = app.location.clone();
        app.complete(
            &Request::List {
                location,
                offset: 0,
            },
            Ok(Response::List {
                rows: vec![row("first", "First"), row("section", "Ideas")],
                total: 2,
                next: None,
                warning: None,
            }),
        );
        assert_eq!(
            app.selected_entry().map(|row| row.id.as_str()),
            Some("section")
        );
    }
    #[test]
    fn writes_keep_captured_target_and_require_review() {
        let mut app = App::new(Profile::default(), None, false);
        app.location = Location::Pages {
            notebook: row("book", "Notebook"),
            section: row("section", "Ideas"),
        };
        app.rows = vec![row("page-1", "First"), row("page-2", "Second")];
        app.key(key(KeyCode::Char('a')));
        app.selected = 1;
        app.paste("Safe note");
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(matches!(app.overlay, Overlay::Review(_)));
        assert!(
            matches!(app.key(key(KeyCode::Enter)), Action::Fetch(Request::Write { target, text, .. }) if target == "page-1" && text == "Safe note")
        );
    }
    #[test]
    fn read_only_blocks_compose() {
        let profile = Profile {
            read_only: true,
            ..Profile::default()
        };
        let mut app = App::new(profile, None, false);
        app.location = Location::Pages {
            notebook: row("book", "Notebook"),
            section: row("section", "Ideas"),
        };
        app.rows.push(row("page", "First"));
        app.key(key(KeyCode::Char('a')));
        assert!(matches!(app.overlay, Overlay::None));
    }
    #[test]
    fn verified_create_is_visible_and_uncertain_write_blocks_retries() {
        let mut app = App::new(Profile::default(), None, false);
        app.location = Location::Pages {
            notebook: row("book", "Notebook"),
            section: row("section", "Ideas"),
        };
        app.complete(
            &Request::Write {
                kind: WriteKind::Create,
                target: "section".into(),
                title: Some("New".into()),
                text: "body".into(),
            },
            Ok(Response::Written(
                json!({"id":"page","name":"New","path":"Notebook / Ideas / New","markdown":"body"}),
            )),
        );
        assert_eq!(
            app.selected_entry().map(|row| row.name.as_str()),
            Some("New")
        );
        assert_eq!(app.notice, "Created New · verified in OneNote");
        app.complete(
            &Request::Write {
                kind: WriteKind::Append,
                target: "page".into(),
                title: None,
                text: "again".into(),
            },
            Err(Error::new("write_uncertain", "Connection lost")),
        );
        app.key(key(KeyCode::Char('a')));
        assert!(app.uncertain);
        assert!(matches!(app.overlay, Overlay::None));
    }
    #[test]
    fn layouts_render_at_compact_medium_and_wide_sizes() {
        let mut app = App::new(Profile::default(), None, false);
        app.rows.push(row("book", "Notebook"));
        app.total = 1;
        for (width, height) in [(42, 12), (80, 24), (120, 35)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("ONENOTE"));
            assert!(screen.contains("Notebook"));
        }
    }
    #[test]
    fn long_write_review_can_reach_the_final_lines() {
        let mut app = App::new(Profile::default(), None, false);
        let mut draft = Compose::new(WriteKind::Append, "page".into(), "Page".into(), None);
        draft.body.paste(
            &(1..=35)
                .map(|n| format!("Line {n:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        app.overlay = Overlay::Review(draft);
        app.key(key(KeyCode::End));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Line 35"));
        assert!(
            matches!(app.key(key(KeyCode::Enter)), Action::Fetch(Request::Write { text, .. }) if text.ends_with("Line 35"))
        );
    }
    #[test]
    fn wrapped_editor_keeps_cursor_visible_on_compact_terminal() {
        let mut app = App::new(Profile::default(), None, false);
        let mut draft = Compose::new(WriteKind::Append, "page".into(), "Page".into(), None);
        draft.body.paste(&"long word ".repeat(45));
        app.overlay = Overlay::Compose(draft);
        let mut terminal = Terminal::new(TestBackend::new(42, 12)).unwrap();
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains('▏'));
    }
    #[test]
    fn escape_leaves_full_page_before_clearing_filter() {
        let mut app = App::new(Profile::default(), None, false);
        app.location = Location::Pages {
            notebook: row("book", "Notebook"),
            section: row("section", "Ideas"),
        };
        app.rows.push(row("page", "Page"));
        app.filter = "Page".into();
        app.full_page = true;
        assert!(matches!(app.key(key(KeyCode::Esc)), Action::None));
        assert!(!app.full_page);
        assert_eq!(app.filter, "Page");
    }
}
