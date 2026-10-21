//! The `/settings` panel: a Status tab (what the app runs with) and a
//! Config tab (what can be changed), drawn in place of the input box.
//!
//! The panel draws and handles keys; what the settings are and how they're
//! saved is a [`SettingsModel`]. For the usual switches, choices and text
//! values, [`SimpleSettings`] builds one from closures:
//!
//! ```
//! use std::sync::{Arc, Mutex};
//! use shellkit::{Choice, SimpleSettings};
//! let on = Arc::new(Mutex::new(true));
//! let (get, set) = (on.clone(), on.clone());
//! let settings = SimpleSettings::new()
//!     .status("Version", || "v1.0".into())
//!     .toggle("Automatic updates", move || *get.lock().unwrap(), move |v| {
//!         *set.lock().unwrap() = v;
//!         Ok(())
//!     })
//!     .choice(
//!         "Colour",
//!         || vec![Choice::new("red", "Red", ""), Choice::new("blue", "Blue", "")],
//!         || Some("red".into()),
//!         |_v| Ok(()),
//!     );
//! ```
//!
//! For settings that lead to one another (pick a provider, then type its
//! key), implement [`SettingsModel`]: each step returns the next
//! [`Outcome`], a [`Picker`] or a [`Field`], with an id saying what it's for.

use std::sync::mpsc::{self, Receiver};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::out::{Kind, Line};
use crate::style::{Row, Style, pad};
use crate::theme::theme;
use crate::widgets::{accent_rule, text_field, title_with_tabs};

/// Picker rows shown at once.
pub const PICK_ROWS: usize = 8;
/// Width of the Status tab's labels.
const STATUS_LABEL_W: usize = 14;

/// The Status tab: groups of labelled values.
pub type StatusGroups = Vec<Vec<(String, Line)>>;

/// A setting's value on the Config tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// On or Off; Enter flips it.
    Switch(bool),
    /// Text with a `›`: Enter opens what changes it.
    Text(Style, String),
    /// Text in the warning colour, for a value that still needs setting.
    Unset(String),
}

/// One row of the Config tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    pub id: String,
    pub label: String,
    pub value: Value,
}

impl SettingRow {
    pub fn new(id: impl Into<String>, label: impl Into<String>, value: Value) -> Self {
        SettingRow {
            id: id.into(),
            label: label.into(),
            value,
        }
    }
}

/// One value to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub name: String,
    pub about: String,
    /// The name's style; the default text colour when unset.
    pub style: Option<Style>,
}

impl Choice {
    pub fn new(value: impl Into<String>, name: impl Into<String>, about: impl Into<String>) -> Self {
        Choice {
            value: value.into(),
            name: name.into(),
            about: about.into(),
            style: None,
        }
    }

    pub fn styled(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
}

/// A list fetched in the background, and why it's a fallback if it is.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub options: Vec<Choice>,
    pub note: Option<String>,
}

/// A list of values to pick one from, opened under a setting.
pub struct Picker {
    /// What the pick is for; handed back to [`SettingsModel::picked`].
    pub id: String,
    /// The row it opens under.
    pub under: String,
    pub options: Vec<Choice>,
    pub selected: usize,
    /// Shown when there's nothing to pick.
    pub empty: String,
    /// While the options are fetched: what's said meanwhile, the answer, and the value to select.
    loading: Option<(String, Receiver<Loaded>, Option<String>)>,
    note: Option<String>,
}

impl Picker {
    /// `options` under the row `under`, with `current` selected.
    pub fn new(under: impl Into<String>, options: Vec<Choice>, current: Option<&str>) -> Self {
        let selected = options
            .iter()
            .position(|o| Some(o.value.as_str()) == current)
            .unwrap_or(0);
        let under = under.into();
        Picker {
            id: under.clone(),
            under,
            options,
            selected,
            empty: "Nothing to pick from".into(),
            loading: None,
            note: None,
        }
    }

    /// Options `load` fetches on a thread; `waiting` is shown meanwhile.
    pub fn loading(
        under: impl Into<String>,
        waiting: impl Into<String>,
        current: Option<String>,
        load: impl FnOnce() -> Loaded + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(load());
        });
        let mut p = Picker::new(under, vec![], None);
        p.loading = Some((waiting.into(), rx, current));
        p
    }

    /// Hand the pick to [`SettingsModel::picked`] as `id` (default: the row's id).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn empty(mut self, text: impl Into<String>) -> Self {
        self.empty = text.into();
        self
    }
}

/// Text typed in place of a setting's value.
pub struct Field {
    /// What the text is for; handed back to [`SettingsModel::entered`].
    pub id: String,
    /// The row it's typed in.
    pub under: String,
    /// Dots instead of the text (a key or password).
    pub secret: bool,
    /// Muted text while it's empty.
    pub placeholder: Option<String>,
    /// The panel's hint while it's open.
    pub hint: String,
    buf: Vec<char>,
    cur: usize,
}

impl Field {
    pub fn new(under: impl Into<String>, text: &str) -> Self {
        let under = under.into();
        Field {
            id: under.clone(),
            under,
            secret: false,
            placeholder: None,
            hint: "Type the value · Enter saves · Esc cancels".into(),
            buf: text.chars().collect(),
            cur: text.chars().count(),
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn hint(mut self, text: impl Into<String>) -> Self {
        self.hint = text.into();
        self
    }
}

/// What a step in the panel leads to.
pub enum Outcome {
    None,
    /// Saved: the panel reloads, and the shell refreshes what depends on the settings.
    Changed,
    /// Not saved, and why; shown under the settings.
    Error(String),
    Pick(Picker),
    Edit(Field),
}

/// What the settings are and how they're saved.
pub trait SettingsModel {
    fn title(&self) -> String {
        "Settings".into()
    }

    /// The Status tab, read when the panel opens and after each change.
    fn status(&mut self) -> StatusGroups;

    /// The Config tab's rows, group by group, read as the panel draws.
    fn groups(&mut self) -> Vec<Vec<SettingRow>>;

    /// Enter or Space on the row `id`.
    fn activate(&mut self, id: &str) -> Outcome;

    /// A value picked from the picker `id`.
    fn picked(&mut self, id: &str, value: &str) -> Outcome {
        let _ = (id, value);
        Outcome::None
    }

    /// Text entered in the field `id` (never empty).
    fn entered(&mut self, id: &str, text: &str) -> Outcome {
        let _ = (id, text);
        Outcome::None
    }

    /// Width of the Config tab's labels.
    fn label_width(&self) -> usize {
        26
    }

    /// The Config tab's rows at their most (every setting and the gaps
    /// between groups), so the panel keeps its height as rows come and go.
    fn max_rows(&mut self) -> usize {
        let g = self.groups();
        g.iter().map(Vec::len).sum::<usize>() + g.len().saturating_sub(1)
    }
}

pub enum SettingsAction {
    None,
    Close,
    /// A setting was saved: refresh what depends on it.
    Changed,
}

pub struct SettingsPanel {
    model: Box<dyn SettingsModel>,
    status: StatusGroups,
    tab: usize,
    sel: usize,
    picker: Option<Picker>,
    field: Option<Field>,
    /// Why the last change was not saved.
    error: Option<String>,
}

impl SettingsPanel {
    pub fn new(mut model: Box<dyn SettingsModel>) -> Self {
        let status = model.status();
        SettingsPanel {
            model,
            status,
            tab: 0,
            sel: 0,
            picker: None,
            field: None,
            error: None,
        }
    }

    /// Open on the Config tab.
    pub fn on_config(mut self) -> Self {
        self.tab = 1;
        self
    }

    pub fn model(&mut self) -> &mut dyn SettingsModel {
        self.model.as_mut()
    }

    fn outcome(&mut self, o: Outcome) -> SettingsAction {
        match o {
            Outcome::None => SettingsAction::None,
            Outcome::Changed => {
                self.error = None;
                self.status = self.model.status();
                SettingsAction::Changed
            }
            Outcome::Error(e) => {
                self.error = Some(e);
                SettingsAction::None
            }
            Outcome::Pick(p) => {
                self.picker = Some(p);
                SettingsAction::None
            }
            Outcome::Edit(f) => {
                self.field = Some(f);
                SettingsAction::None
            }
        }
    }

    /// True while a list is being fetched, so the shell keeps drawing.
    pub fn busy(&mut self) -> bool {
        let Some(p) = &mut self.picker else {
            return false;
        };
        let Some((_, rx, current)) = &p.loading else {
            return false;
        };
        let got = match rx.try_recv() {
            Ok(l) => Some(l),
            Err(mpsc::TryRecvError::Empty) => return true,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        let current = current.clone();
        p.loading = None;
        if let Some(l) = got {
            p.note = l.note;
            p.selected = l
                .options
                .iter()
                .position(|o| Some(&o.value) == current.as_ref())
                .unwrap_or(0);
            p.options = l.options;
        }
        true
    }

    /// Text pasted while a field is open goes into it.
    pub fn paste(&mut self, text: &str) {
        if let Some(f) = &mut self.field {
            for c in text.chars().filter(|c| !c.is_control()) {
                f.buf.insert(f.cur, c);
                f.cur += 1;
            }
        }
    }

    fn ids(&mut self) -> Vec<String> {
        self.model.groups().concat().into_iter().map(|r| r.id).collect()
    }

    pub fn key(&mut self, key: KeyEvent) -> SettingsAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let close = key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('c'));
        if let Some(f) = &mut self.field {
            match key.code {
                KeyCode::Enter => {
                    let text: String = f.buf.iter().collect::<String>().trim().to_owned();
                    let f = self.field.take().expect("checked");
                    if text.is_empty() {
                        return SettingsAction::None;
                    }
                    let o = self.model.entered(&f.id, &text);
                    return self.outcome(o);
                }
                _ if close => self.field = None,
                KeyCode::Left => f.cur = f.cur.saturating_sub(1),
                KeyCode::Right => f.cur = (f.cur + 1).min(f.buf.len()),
                KeyCode::Home => f.cur = 0,
                KeyCode::End => f.cur = f.buf.len(),
                KeyCode::Backspace if f.cur > 0 => {
                    f.cur -= 1;
                    f.buf.remove(f.cur);
                }
                KeyCode::Delete if f.cur < f.buf.len() => {
                    f.buf.remove(f.cur);
                }
                KeyCode::Char('v') if ctrl => {}
                KeyCode::Char(c) if !ctrl => {
                    f.buf.insert(f.cur, c);
                    f.cur += 1;
                }
                _ => {}
            }
            return SettingsAction::None;
        }
        if let Some(p) = &mut self.picker {
            if p.loading.is_some() {
                if close || key.code == KeyCode::Left {
                    self.picker = None;
                }
                return SettingsAction::None;
            }
            match key.code {
                KeyCode::Up => p.selected = p.selected.saturating_sub(1),
                KeyCode::Down => p.selected = (p.selected + 1).min(p.options.len().saturating_sub(1)),
                KeyCode::PageUp => p.selected = p.selected.saturating_sub(PICK_ROWS),
                KeyCode::PageDown => p.selected = (p.selected + PICK_ROWS).min(p.options.len().saturating_sub(1)),
                KeyCode::Left => self.picker = None,
                _ if close => self.picker = None,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let p = self.picker.take().expect("checked");
                    let Some(choice) = p.options.get(p.selected) else {
                        return SettingsAction::None;
                    };
                    let o = self.model.picked(&p.id, &choice.value);
                    return self.outcome(o);
                }
                _ => {}
            }
            return SettingsAction::None;
        }
        match key.code {
            _ if close => return SettingsAction::Close,
            KeyCode::Char('q') => return SettingsAction::Close,
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.tab = 1 - self.tab;
                self.error = None;
            }
            _ if self.tab != 1 => {}
            KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Down => self.sel += 1,
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.sel = usize::MAX,
            KeyCode::Enter | KeyCode::Char(' ') => {
                let ids = self.ids();
                if ids.is_empty() {
                    return SettingsAction::None;
                }
                self.sel = self.sel.min(ids.len() - 1);
                let o = self.model.activate(&ids[self.sel]);
                return self.outcome(o);
            }
            _ => {}
        }
        self.sel = self.sel.min(self.ids().len().saturating_sub(1));
        SettingsAction::None
    }

    /// The panel for a `w`-column screen: as tall as its content, at most `h` rows.
    pub fn rows(&mut self, w: usize, h: usize) -> Vec<Row> {
        let t = theme();
        let title = self.model.title();
        let top: Vec<Row> = vec![
            accent_rule(w),
            vec![],
            title_with_tabs(&title, &["Status", "Config"], self.tab),
            vec![],
        ];

        let mut status: Vec<Row> = Vec::new();
        for (i, group) in self.status.iter().enumerate() {
            if i > 0 {
                status.push(vec![]);
            }
            for (label, value) in group {
                let mut row: Row = vec![(
                    Style::default(),
                    format!("  {}", pad(&format!("{label}:"), STATUS_LABEL_W)),
                )];
                row.extend(value.iter().map(|(k, t)| (k.style(), t.clone())));
                status.push(row);
            }
        }
        let config = self.config_rows();
        let hint = if self.tab == 0 {
            "←→ tabs · Esc closes".to_owned()
        } else if let Some(f) = &self.field {
            f.hint.clone()
        } else if self.picker.is_some() {
            "↑↓ choose · Enter picks · Esc goes back".to_owned()
        } else {
            "Enter or Space changes it · ←→ tabs · Esc closes".to_owned()
        };
        let bottom: Vec<Row> = vec![vec![], vec![(Style::fg(t.muted), format!("  {hint}"))]];
        // As tall as the Config tab can get with a picker open, so switching tabs keeps the height.
        let needed = status.len().max(self.model.max_rows() + PICK_ROWS + 4);
        let body_h = needed.min(h.saturating_sub(top.len() + bottom.len())).max(1);
        let mut body = if self.tab == 0 { status } else { config };
        // Keep the open picker in view on a short screen.
        let skip = body.len().saturating_sub(body_h).min(self.sel);
        body.drain(..skip);
        body.resize(body_h, vec![]);
        let mut rows = top;
        rows.extend(body);
        rows.extend(bottom);
        rows.truncate(h);
        rows
    }

    fn config_rows(&mut self) -> Vec<Row> {
        let groups = self.model.groups();
        let label_w = self.model.label_width();
        let mut rows: Vec<Row> = Vec::new();
        let mut i = 0;
        for (g, group) in groups.into_iter().enumerate() {
            if g > 0 {
                rows.push(vec![]);
            }
            for setting in group {
                self.setting_rows(&mut rows, &setting, i == self.sel, label_w);
                i += 1;
            }
        }
        if let Some(e) = &self.error {
            rows.push(vec![]);
            rows.push(vec![(Style::fg(theme().danger), format!("    {e}"))]);
        }
        rows
    }

    /// One setting's row, and its picker when open.
    fn setting_rows(&self, rows: &mut Vec<Row>, setting: &SettingRow, on: bool, label_w: usize) {
        let t = theme();
        let base = if on { Style::fg(t.accent) } else { Style::default() };
        let mut row: Row = vec![
            (base, if on { "  ❯ " } else { "    " }.into()),
            (base, pad(&setting.label, label_w)),
        ];
        match (&self.field, &setting.value) {
            (Some(f), _) if on && f.under == setting.id => {
                let shown: Vec<char> = if f.secret {
                    vec!['•'; f.buf.len()]
                } else {
                    f.buf.clone()
                };
                row.extend(text_field(&shown, f.cur, Style::default()));
                if let Some(p) = f.placeholder.as_ref().filter(|_| f.buf.is_empty()) {
                    row.push((Style::fg(t.muted), p.clone()));
                }
            }
            (_, Value::Switch(v)) => {
                let style = Style::fg(if *v { t.good } else { t.danger });
                row.push((
                    if on { style.bold() } else { style },
                    if *v { "On" } else { "Off" }.into(),
                ));
            }
            (_, Value::Text(style, value)) => {
                row.push((
                    if on && *style == Style::default() { base } else { *style },
                    value.clone(),
                ));
                row.push((Style::fg(t.muted), " ›".into()));
            }
            (_, Value::Unset(value)) => {
                row.push((Style::fg(t.warn), value.clone()));
                row.push((Style::fg(t.muted), " ›".into()));
            }
        }
        rows.push(row);
        let Some(p) = self.picker.as_ref().filter(|p| p.under == setting.id) else {
            return;
        };
        let indent = " ".repeat(label_w + 4);
        if let Some((waiting, _, _)) = &p.loading {
            rows.push(vec![(Style::fg(t.muted), format!("{indent}{waiting}"))]);
            return;
        }
        if let Some(n) = &p.note {
            let why: String = if n.chars().count() > 76 {
                n.chars().take(75).chain(std::iter::once('…')).collect()
            } else {
                n.clone()
            };
            rows.push(vec![(
                Style::fg(t.warn),
                format!("{indent}Couldn't get the list, so these are the usual ones:"),
            )]);
            rows.push(vec![(Style::fg(t.muted), format!("{indent}{why}"))]);
        }
        if p.options.is_empty() {
            rows.push(vec![(Style::fg(t.muted), format!("{indent}{}", p.empty))]);
        }
        let name_w = p
            .options
            .iter()
            .map(|o| o.name.chars().count() + 2)
            .max()
            .unwrap_or(0)
            .max(label_w);
        let first = p
            .selected
            .saturating_sub(4)
            .min(p.options.len().saturating_sub(PICK_ROWS));
        // A row says how many are above, so one fewer option shows.
        let shown = if first > 0 { PICK_ROWS - 1 } else { PICK_ROWS };
        let first = if first > 0 { (first + 1).min(p.selected) } else { 0 };
        if first > 0 {
            rows.push(vec![(Style::fg(t.muted), format!("{indent}↑ {first} more"))]);
        }
        for (j, choice) in p.options.iter().enumerate().skip(first).take(shown) {
            let pon = j == p.selected;
            let name_style = match choice.style {
                Some(s) if pon => s.bold(),
                Some(s) => s,
                None if pon => Style::fg(t.accent),
                None => Style::default(),
            };
            rows.push(vec![
                (Style::default(), indent.clone()),
                (
                    if pon { Style::fg(t.accent) } else { Style::default() },
                    if pon { "❯ " } else { "  " }.into(),
                ),
                (name_style, pad(&choice.name, name_w)),
                (Style::fg(t.muted), choice.about.clone()),
            ]);
        }
        let below = p.options.len().saturating_sub(first + shown);
        if below > 0 {
            rows.push(vec![(Style::fg(t.muted), format!("{indent}↓ {below} more"))]);
        }
    }
}

/// The Status tab as plain lines, for printing outside the shell.
pub fn status_lines(groups: &StatusGroups) -> Vec<Line> {
    let mut lines = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            lines.push(vec![(Kind::Out, String::new())]);
        }
        for (label, value) in group {
            let mut row = vec![(Kind::Out, pad(&format!("{label}:"), STATUS_LABEL_W))];
            row.extend(value.iter().cloned());
            lines.push(row);
        }
    }
    lines
}

type Get<T> = Box<dyn Fn() -> T>;
type Set<T> = Box<dyn FnMut(T) -> anyhow::Result<()>>;

enum Simple {
    Toggle(Get<bool>, Set<bool>),
    Choice(Get<Vec<Choice>>, Get<Option<String>>, Set<String>),
    Text(Get<Option<String>>, Set<String>, bool),
}

/// Settings from closures: switches, choices and text, in groups.
#[derive(Default)]
pub struct SimpleSettings {
    title: Option<String>,
    status: Vec<Vec<(String, Get<Line>)>>,
    groups: Vec<Vec<(String, Simple)>>,
}

impl SimpleSettings {
    pub fn new() -> Self {
        SimpleSettings::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A Status row whose value is plain text.
    pub fn status(self, label: impl Into<String>, value: impl Fn() -> String + 'static) -> Self {
        self.status_line(label, move || vec![(Kind::Out, value())])
    }

    /// A Status row whose value is styled pieces.
    pub fn status_line(mut self, label: impl Into<String>, value: impl Fn() -> Line + 'static) -> Self {
        if self.status.is_empty() {
            self.status.push(vec![]);
        }
        self.status
            .last_mut()
            .expect("just made")
            .push((label.into(), Box::new(value)));
        self
    }

    /// Start a new group of Status rows.
    pub fn status_gap(mut self) -> Self {
        self.status.push(vec![]);
        self
    }

    fn push(mut self, label: impl Into<String>, s: Simple) -> Self {
        if self.groups.is_empty() {
            self.groups.push(vec![]);
        }
        self.groups.last_mut().expect("just made").push((label.into(), s));
        self
    }

    /// On or Off.
    pub fn toggle(
        self,
        label: impl Into<String>,
        get: impl Fn() -> bool + 'static,
        set: impl FnMut(bool) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.push(label, Simple::Toggle(Box::new(get), Box::new(set)))
    }

    /// One of `options`; `get` is the value now (None: not set).
    pub fn choice(
        self,
        label: impl Into<String>,
        options: impl Fn() -> Vec<Choice> + 'static,
        get: impl Fn() -> Option<String> + 'static,
        set: impl FnMut(String) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.push(label, Simple::Choice(Box::new(options), Box::new(get), Box::new(set)))
    }

    /// Typed text; `get` is the value now (None: not set).
    pub fn text(
        self,
        label: impl Into<String>,
        get: impl Fn() -> Option<String> + 'static,
        set: impl FnMut(String) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.push(label, Simple::Text(Box::new(get), Box::new(set), false))
    }

    /// Typed text shown as dots, for a key or password.
    pub fn secret(
        self,
        label: impl Into<String>,
        get: impl Fn() -> Option<String> + 'static,
        set: impl FnMut(String) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.push(label, Simple::Text(Box::new(get), Box::new(set), true))
    }

    /// Start a new group of Config rows.
    pub fn gap(mut self) -> Self {
        self.groups.push(vec![]);
        self
    }

    fn find(&mut self, id: &str) -> Option<&mut (String, Simple)> {
        let (g, i) = id.split_once('.')?;
        self.groups
            .get_mut(g.parse::<usize>().ok()?)?
            .get_mut(i.parse::<usize>().ok()?)
    }
}

fn saved(r: anyhow::Result<()>) -> Outcome {
    match r {
        Ok(()) => Outcome::Changed,
        Err(e) => Outcome::Error(format!("{e:#}")),
    }
}

impl SettingsModel for SimpleSettings {
    fn title(&self) -> String {
        self.title.clone().unwrap_or_else(|| "Settings".into())
    }

    fn status(&mut self) -> StatusGroups {
        self.status
            .iter()
            .map(|g| g.iter().map(|(label, value)| (label.clone(), value())).collect())
            .collect()
    }

    fn groups(&mut self) -> Vec<Vec<SettingRow>> {
        self.groups
            .iter()
            .enumerate()
            .map(|(g, group)| {
                group
                    .iter()
                    .enumerate()
                    .map(|(i, (label, s))| {
                        let value = match s {
                            Simple::Toggle(get, _) => Value::Switch(get()),
                            Simple::Choice(options, get, _) => match get() {
                                Some(v) => {
                                    let name = options().into_iter().find(|o| o.value == v).map_or(v, |o| o.name);
                                    Value::Text(Style::default(), name)
                                }
                                None => Value::Unset("Not set".into()),
                            },
                            Simple::Text(get, _, secret) => match get() {
                                Some(_) if *secret => Value::Text(Style::default(), "••••".into()),
                                Some(v) => Value::Text(Style::default(), v),
                                None => Value::Unset("Not set".into()),
                            },
                        };
                        SettingRow::new(format!("{g}.{i}"), label.clone(), value)
                    })
                    .collect()
            })
            .collect()
    }

    fn activate(&mut self, id: &str) -> Outcome {
        let Some((_, s)) = self.find(id) else {
            return Outcome::None;
        };
        match s {
            Simple::Toggle(get, set) => {
                let on = get();
                saved(set(!on))
            }
            Simple::Choice(options, get, _) => Outcome::Pick(Picker::new(id, options(), get().as_deref())),
            Simple::Text(get, _, secret) => {
                let mut f = if *secret {
                    Field::new(id, "").secret()
                } else {
                    Field::new(id, &get().unwrap_or_default())
                };
                if *secret {
                    f = f.hint("Paste it · Enter saves · Esc cancels");
                }
                Outcome::Edit(f)
            }
        }
    }

    fn picked(&mut self, id: &str, value: &str) -> Outcome {
        match self.find(id) {
            Some((_, Simple::Choice(_, _, set))) => saved(set(value.to_owned())),
            _ => Outcome::None,
        }
    }

    fn entered(&mut self, id: &str, text: &str) -> Outcome {
        match self.find(id) {
            Some((_, Simple::Text(_, set, _))) => saved(set(text.to_owned())),
            _ => Outcome::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::style::row_text;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn text(rows: &[Row]) -> String {
        rows.iter().map(row_text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn simple_settings_switch_pick_and_type() {
        let on = Rc::new(RefCell::new(false));
        let colour = Rc::new(RefCell::new(None::<String>));
        let name = Rc::new(RefCell::new(None::<String>));
        let (o1, o2, c1, c2, n1, n2) = (
            on.clone(),
            on.clone(),
            colour.clone(),
            colour.clone(),
            name.clone(),
            name.clone(),
        );
        let model = SimpleSettings::new()
            .status("Version", || "v1".into())
            .toggle(
                "Watch",
                move || *o1.borrow(),
                move |v| {
                    *o2.borrow_mut() = v;
                    Ok(())
                },
            )
            .choice(
                "Colour",
                || vec![Choice::new("r", "Red", "warm"), Choice::new("b", "Blue", "cool")],
                move || c1.borrow().clone(),
                move |v| {
                    *c2.borrow_mut() = Some(v);
                    Ok(())
                },
            )
            .gap()
            .text(
                "Name",
                move || n1.borrow().clone(),
                move |v| {
                    *n2.borrow_mut() = Some(v);
                    Ok(())
                },
            );
        let mut p = SettingsPanel::new(Box::new(model));
        assert!(text(&p.rows(80, 30)).contains("Version:      v1"));
        p.key(key(KeyCode::Right));
        let shown = text(&p.rows(80, 30));
        assert!(shown.contains("❯ Watch") && shown.contains("Off") && shown.contains("Not set"));

        assert!(matches!(p.key(key(KeyCode::Enter)), SettingsAction::Changed));
        assert!(*on.borrow());

        p.key(key(KeyCode::Down));
        p.key(key(KeyCode::Enter));
        assert!(text(&p.rows(80, 30)).contains("❯ Red"));
        p.key(key(KeyCode::Down));
        assert!(matches!(p.key(key(KeyCode::Enter)), SettingsAction::Changed));
        assert_eq!(colour.borrow().as_deref(), Some("b"));
        assert!(text(&p.rows(80, 30)).contains("Blue ›"));

        p.key(key(KeyCode::Down));
        p.key(key(KeyCode::Enter));
        for c in "Ada".chars() {
            p.key(key(KeyCode::Char(c)));
        }
        assert!(matches!(p.key(key(KeyCode::Enter)), SettingsAction::Changed));
        assert_eq!(name.borrow().as_deref(), Some("Ada"));
        assert!(matches!(p.key(key(KeyCode::Esc)), SettingsAction::Close));
    }

    #[test]
    fn a_failed_save_says_why() {
        let model = SimpleSettings::new().toggle("Watch", || false, |_| anyhow::bail!("disk full"));
        let mut p = SettingsPanel::new(Box::new(model)).on_config();
        assert!(matches!(p.key(key(KeyCode::Enter)), SettingsAction::None));
        assert!(text(&p.rows(80, 30)).contains("disk full"));
    }

    #[test]
    fn a_picker_can_load_in_the_background() {
        struct M;
        impl SettingsModel for M {
            fn status(&mut self) -> StatusGroups {
                vec![]
            }
            fn groups(&mut self) -> Vec<Vec<SettingRow>> {
                vec![vec![SettingRow::new(
                    "model",
                    "Model",
                    Value::Text(Style::default(), "a".into()),
                )]]
            }
            fn activate(&mut self, id: &str) -> Outcome {
                Outcome::Pick(Picker::loading(
                    id,
                    "Asking for the models…",
                    Some("b".into()),
                    || Loaded {
                        options: vec![Choice::new("a", "A", ""), Choice::new("b", "B", "")],
                        note: Some("offline".into()),
                    },
                ))
            }
        }
        let mut p = SettingsPanel::new(Box::new(M)).on_config();
        p.key(key(KeyCode::Enter));
        assert!(text(&p.rows(80, 30)).contains("Asking for the models…") || !p.busy());
        while p.busy() && p.picker.as_ref().is_some_and(|p| p.loading.is_some()) {
            std::thread::yield_now();
        }
        let shown = text(&p.rows(80, 30));
        assert!(shown.contains("❯ B") && shown.contains("offline"), "{shown}");
    }

    #[test]
    fn status_prints_as_lines() {
        let groups: StatusGroups = vec![
            vec![("Version".into(), vec![(Kind::Out, "v1".into())])],
            vec![("Logs".into(), vec![(Kind::Out, "C:\\logs".into())])],
        ];
        let lines: Vec<String> = status_lines(&groups)
            .iter()
            .map(|l| l.iter().map(|(_, t)| t.as_str()).collect())
            .collect();
        assert_eq!(lines, ["Version:      v1", "", "Logs:         C:\\logs"]);
    }
}
