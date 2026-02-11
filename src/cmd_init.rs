use std::io::stdout;

use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};

use crate::config;

// ── Data Model ──

#[derive(Clone)]
enum FieldKind {
    Text { value: String, default: Option<String> },
    Password { value: String },
    Select { options: Vec<String>, selected: usize },
}

#[derive(Clone)]
struct Field {
    label: String,
    kind: FieldKind,
    done: bool,
}

struct Section {
    title: String,
    fields: Vec<Field>,
    skippable: bool,
    skipped: bool,
    active: bool,
    completed: bool,
}

struct InitApp {
    sections: Vec<Section>,
    sec_idx: usize,
    field_idx: usize,
    finished: bool,
    cancelled: bool,
}

impl InitApp {
    fn new() -> Self {
        let workspace_default = config::config_dir()
            .join("workspace")
            .to_string_lossy()
            .to_string();

        let sections = vec![
            Section {
                title: "Workspace".into(),
                skippable: false,
                skipped: false,
                active: true,
                completed: false,
                fields: vec![Field {
                    label: "Working directory".into(),
                    kind: FieldKind::Text {
                        value: String::new(),
                        default: Some(workspace_default),
                    },
                    done: false,
                }],
            },
            Section {
                title: "MyAgent Agent".into(),
                skippable: false,
                skipped: false,
                active: false,
                completed: false,
                fields: vec![
                    // 0: API Provider
                    Field {
                        label: "API Provider".into(),
                        kind: FieldKind::Select {
                            options: vec![
                                "OpenRouter (Recommended)".into(),
                                "Custom API (Claude Messages format only)".into(),
                            ],
                            selected: 0,
                        },
                        done: false,
                    },
                    // 1: API Key
                    Field {
                        label: "API Key".into(),
                        kind: FieldKind::Password {
                            value: String::new(),
                        },
                        done: false,
                    },
                    // 2: Base URL (Custom API only)
                    Field {
                        label: "Base URL (docs: platform.claude.com/docs/en/api/overview)".into(),
                        kind: FieldKind::Text {
                            value: String::new(),
                            default: None,
                        },
                        done: false,
                    },
                    // 3: Model select (OpenRouter only)
                    Field {
                        label: "Model".into(),
                        kind: FieldKind::Select {
                            options: vec![
                                "moonshotai/kimi-k2.5".into(),
                                "openrouter/pony-alpha".into(),
                                "anthropic/claude-opus-4.6".into(),
                                "Custom".into(),
                            ],
                            selected: 0,
                        },
                        done: false,
                    },
                    // 4: Custom model name
                    Field {
                        label: "Model name".into(),
                        kind: FieldKind::Text {
                            value: String::new(),
                            default: None,
                        },
                        done: false,
                    },
                ],
            },
            Section {
                title: "Claude Agent".into(),
                skippable: true,
                skipped: false,
                active: false,
                completed: false,
                fields: vec![
                    // First field: Configure/Skip select
                    Field {
                        label: "".into(),
                        kind: FieldKind::Select {
                            options: vec!["Configure".into(), "Skip".into()],
                            selected: 0,
                        },
                        done: false,
                    },
                    // Auth method select
                    Field {
                        label: "Auth method".into(),
                        kind: FieldKind::Select {
                            options: vec![
                                "ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN".into(),
                                "ANTHROPIC_BASE_URL + ANTHROPIC_API_KEY".into(),
                            ],
                            selected: 0,
                        },
                        done: false,
                    },
                    Field {
                        label: "ANTHROPIC_BASE_URL".into(),
                        kind: FieldKind::Text {
                            value: String::new(),
                            default: None,
                        },
                        done: false,
                    },
                    // Placeholder for AUTH_TOKEN or API_KEY (label set dynamically)
                    Field {
                        label: "ANTHROPIC_AUTH_TOKEN".into(),
                        kind: FieldKind::Password {
                            value: String::new(),
                        },
                        done: false,
                    },
                ],
            },
            Section {
                title: "Feishu Channel".into(),
                skippable: true,
                skipped: false,
                active: false,
                completed: false,
                fields: vec![
                    Field {
                        label: "".into(),
                        kind: FieldKind::Select {
                            options: vec!["Configure".into(), "Skip".into()],
                            selected: 0,
                        },
                        done: false,
                    },
                    Field {
                        label: "App ID".into(),
                        kind: FieldKind::Text {
                            value: String::new(),
                            default: None,
                        },
                        done: false,
                    },
                    Field {
                        label: "App Secret".into(),
                        kind: FieldKind::Password {
                            value: String::new(),
                        },
                        done: false,
                    },
                ],
            },
        ];

        Self {
            sections,
            sec_idx: 0,
            field_idx: 0,
            finished: false,
            cancelled: false,
        }
    }

    fn prefill(&mut self, cfg: &config::AppConfig) {
        let me = cfg.myagent_env();
        let cl = cfg.claude_env();

        // Workspace
        if let Some(w) = &cfg.workspace {
            self.set_field_value(0, 0, w);
        }
        // MyAgent - detect provider from base_url
        let is_openrouter = me.base_url.contains("openrouter.ai");
        if !is_openrouter {
            // Custom API: select provider=1
            if let Some(FieldKind::Select { selected, .. }) =
                self.sections.get_mut(1).and_then(|s| s.fields.get_mut(0)).map(|f| &mut f.kind)
            {
                *selected = 1;
            }
            self.set_field_value(1, 2, &me.base_url); // base_url field
        }
        self.set_field_value(1, 1, &me.api_key); // API key field
        // Try to match model to preset options
        let model_presets = ["moonshotai/kimi-k2.5", "openrouter/pony-alpha", "anthropic/claude-opus-4.6"];
        if let Some(idx) = model_presets.iter().position(|m| *m == me.model) {
            if let Some(FieldKind::Select { selected, .. }) =
                self.sections.get_mut(1).and_then(|s| s.fields.get_mut(3)).map(|f| &mut f.kind)
            {
                *selected = idx;
            }
        } else {
            // Custom model
            if is_openrouter {
                if let Some(FieldKind::Select { selected, .. }) =
                    self.sections.get_mut(1).and_then(|s| s.fields.get_mut(3)).map(|f| &mut f.kind)
                {
                    *selected = 3; // Custom
                }
            }
            self.set_field_value(1, 4, &me.model); // custom model name field
        }
        // Claude
        if cl.base_url.is_some() || cl.auth_token.is_some() || cl.api_key.is_some() {
            // Pre-select "Configure"
            if let Some(FieldKind::Select { selected, .. }) =
                self.sections.get_mut(2).and_then(|s| s.fields.get_mut(0)).map(|f| &mut f.kind)
            {
                *selected = 0;
            }
            // Determine auth method
            if cl.api_key.is_some() && cl.auth_token.is_none() {
                if let Some(FieldKind::Select { selected, .. }) =
                    self.sections.get_mut(2).and_then(|s| s.fields.get_mut(1)).map(|f| &mut f.kind)
                {
                    *selected = 1; // BASE_URL + API_KEY
                }
                self.sections[2].fields[3].label = "ANTHROPIC_API_KEY".to_string();
                if let Some(k) = &cl.api_key {
                    self.set_field_value(2, 3, k);
                }
            } else {
                if let Some(t) = &cl.auth_token {
                    self.set_field_value(2, 3, t);
                }
            }
            if let Some(u) = &cl.base_url {
                self.set_field_value(2, 2, u);
            }
        }
        // Feishu
        if let Some(f) = cfg.feishu_config() {
            if let Some(FieldKind::Select { selected, .. }) =
                self.sections.get_mut(3).and_then(|s| s.fields.get_mut(0)).map(|f| &mut f.kind)
            {
                *selected = 0;
            }
            self.set_field_value(3, 1, &f.app_id);
            self.set_field_value(3, 2, &f.app_secret);
        }
    }

    fn set_field_value(&mut self, sec: usize, field: usize, val: &str) {
        if val.is_empty() { return; }
        if let Some(f) = self.sections.get_mut(sec).and_then(|s| s.fields.get_mut(field)) {
            match &mut f.kind {
                FieldKind::Text { value, .. } => *value = val.to_string(),
                FieldKind::Password { value } => *value = val.to_string(),
                _ => {}
            }
        }
    }

    fn current_field(&self) -> Option<&Field> {
        self.sections
            .get(self.sec_idx)
            .and_then(|s| s.fields.get(self.field_idx))
    }

    fn current_field_mut(&mut self) -> Option<&mut Field> {
        self.sections
            .get_mut(self.sec_idx)
            .and_then(|s| s.fields.get_mut(self.field_idx))
    }

    fn advance(&mut self) {
        let sec = &mut self.sections[self.sec_idx];
        if let Some(f) = sec.fields.get_mut(self.field_idx) {
            // For text fields with empty value, use default
            if let FieldKind::Text { value, default } = &mut f.kind {
                if value.is_empty() {
                    if let Some(d) = default.clone() {
                        *value = d;
                    }
                }
            }
            f.done = true;
        }

        // Handle skip selection for skippable sections
        if sec.skippable && self.field_idx == 0 {
            if let FieldKind::Select { selected, .. } = &sec.fields[0].kind {
                if *selected == 1 {
                    // Skip
                    sec.skipped = true;
                    sec.completed = true;
                    sec.active = false;
                    self.next_section();
                    return;
                }
            }
        }

        // Handle Claude auth method selection
        if self.sec_idx == 2 && self.field_idx == 1 {
            if let FieldKind::Select { selected, .. } = &sec.fields[1].kind {
                let label = if *selected == 0 {
                    "ANTHROPIC_AUTH_TOKEN"
                } else {
                    "ANTHROPIC_API_KEY"
                };
                sec.fields[3].label = label.to_string();
            }
        }

        // Handle MyAgent provider/model conditional flow
        if self.sec_idx == 1 {
            let provider = if let FieldKind::Select { selected, .. } = &sec.fields[0].kind {
                *selected
            } else {
                0
            };

            match self.field_idx {
                1 => {
                    // After API key: OpenRouter → skip base_url (field 2), go to model select (field 3)
                    //                 Custom → go to base_url (field 2)
                    if provider == 0 {
                        self.field_idx = 3; // skip to model select
                        return;
                    }
                    // Custom: fall through to field 2
                }
                2 => {
                    // After base_url (Custom API): skip model select (field 3), go to model name (field 4)
                    self.field_idx = 4;
                    return;
                }
                3 => {
                    // After model select (OpenRouter): if preset model → done, if Custom → field 4
                    if let FieldKind::Select { selected, .. } = &sec.fields[3].kind {
                        if *selected < 3 {
                            // Preset model selected, section done
                            sec.completed = true;
                            sec.active = false;
                            self.next_section();
                            return;
                        }
                    }
                    // Custom model: fall through to field 4
                }
                4 => {
                    // After custom model name: section done
                    sec.completed = true;
                    sec.active = false;
                    self.next_section();
                    return;
                }
                _ => {}
            }
        }

        self.field_idx += 1;
        if self.field_idx >= sec.fields.len() {
            sec.completed = true;
            sec.active = false;
            self.next_section();
        }
    }

    fn next_section(&mut self) {
        self.sec_idx += 1;
        self.field_idx = 0;
        if self.sec_idx >= self.sections.len() {
            self.finished = true;
        } else {
            self.sections[self.sec_idx].active = true;
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.cancelled = true;
            }
            KeyCode::Enter => {
                self.advance();
            }
            KeyCode::Up | KeyCode::Down => {
                if let Some(f) = self.current_field_mut() {
                    if let FieldKind::Select {
                        options, selected, ..
                    } = &mut f.kind
                    {
                        if code == KeyCode::Up && *selected > 0 {
                            *selected -= 1;
                        } else if code == KeyCode::Down
                            && *selected < options.len() - 1
                        {
                            *selected += 1;
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(f) = self.current_field_mut() {
                    match &mut f.kind {
                        FieldKind::Text { value, .. } => value.push(c),
                        FieldKind::Password { value } => value.push(c),
                        _ => {}
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(f) = self.current_field_mut() {
                    match &mut f.kind {
                        FieldKind::Text { value, .. } => {
                            value.pop();
                        }
                        FieldKind::Password { value } => {
                            value.pop();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_paste(&mut self, text: String) {
        if let Some(f) = self.current_field_mut() {
            // Strip newlines/carriage returns from pasted text
            let clean = text.replace(['\n', '\r'], "");
            match &mut f.kind {
                FieldKind::Text { value, .. } => value.push_str(&clean),
                FieldKind::Password { value } => value.push_str(&clean),
                _ => {}
            }
        }
    }

    fn build_config(&self) -> serde_json::Value {
        let workspace = self.get_text(0, 0);
        let api_key = self.get_text(1, 1); // field 1: API key

        // Determine provider, base_url, model
        let provider = self.get_select(1, 0); // field 0: provider select
        let (base_url, model) = if provider == 0 {
            // OpenRouter
            let model_sel = self.get_select(1, 3); // field 3: model select
            let model = if model_sel == 3 {
                // Custom model
                self.get_text(1, 4)
            } else {
                let options = ["moonshotai/kimi-k2.5", "openrouter/pony-alpha", "anthropic/claude-opus-4.6"];
                options[model_sel].to_string()
            };
            ("https://openrouter.ai/api".to_string(), model)
        } else {
            // Custom API
            (self.get_text(1, 2), self.get_text(1, 4))
        };

        let mut agents = serde_json::json!({
            "myagent": { "env": {
                "MYAGENT_API_KEY": api_key,
                "MYAGENT_BASE_URL": base_url,
                "MYAGENT_MODEL": model,
            }}
        });

        // Claude
        if !self.sections[2].skipped {
            let auth_method = self.get_select(2, 1);
            let base = self.get_text(2, 2);
            let credential = self.get_text(2, 3);
            let key_name = if auth_method == 0 {
                "ANTHROPIC_AUTH_TOKEN"
            } else {
                "ANTHROPIC_API_KEY"
            };
            agents["claude"] = serde_json::json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base,
                    key_name: credential,
                }
            });
        }

        let mut config = serde_json::json!({
            "version": 1,
            "workspace": workspace,
            "default_agent": "myagent",
            "agents": agents,
        });

        // Feishu
        if !self.sections[3].skipped {
            let app_id = self.get_text(3, 1);
            let app_secret = self.get_text(3, 2);
            config["channels"] = serde_json::json!({
                "feishu": {
                    "app_id": app_id,
                    "app_secret": app_secret,
                }
            });
        }

        config
    }

    fn get_text(&self, sec: usize, field: usize) -> String {
        if let Some(f) = self.sections.get(sec).and_then(|s| s.fields.get(field)) {
            match &f.kind {
                FieldKind::Text { value, default } => {
                    if value.is_empty() {
                        default.clone().unwrap_or_default()
                    } else {
                        value.clone()
                    }
                }
                FieldKind::Password { value } => value.clone(),
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    fn get_select(&self, sec: usize, field: usize) -> usize {
        if let Some(f) = self.sections.get(sec).and_then(|s| s.fields.get(field)) {
            if let FieldKind::Select { selected, .. } = &f.kind {
                return *selected;
            }
        }
        0
    }
}

// ── Rendering ──

// "my" = first 18 columns, "agent" = rest
const LOGO_SPLIT: usize = 18;
const LOGO_LINES: &[&str] = &[
    "                                           _   ",
    "  _ __ ___  _   _  __ _  __ _  ___ _ __ | |_  ",
    r" | '_ ` _ \| | | |/ _` |/ _` |/ _ \ '_ \| __|",
    " | | | | | | |_| | (_| | (_| |  __/ | | | |_  ",
    r" |_| |_| |_|\__, |\__,_|\__, |\___|_| |_|\__| ",
    "             |___/       |___/                  ",
];

fn render(frame: &mut Frame, app: &InitApp) {
    let area = frame.area();
    let mut lines: Vec<Line> = Vec::new();

    let my_style = Style::default().fg(Color::Rgb(160, 82, 45)).add_modifier(Modifier::BOLD);
    let agent_style = Style::default().fg(Color::Rgb(255, 245, 225)).add_modifier(Modifier::BOLD);
    for logo_line in LOGO_LINES {
        let (left, right) = if logo_line.len() > LOGO_SPLIT {
            (&logo_line[..LOGO_SPLIT], &logo_line[LOGO_SPLIT..])
        } else {
            (*logo_line, "")
        };
        lines.push(Line::from(vec![
            Span::styled(left.to_string(), my_style),
            Span::styled(right.to_string(), agent_style),
        ]));
    }
    lines.push(Line::from(""));

    for (si, sec) in app.sections.iter().enumerate() {
        // Section title
        let title_style = if sec.active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if sec.completed {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let prefix = if sec.completed && !sec.skipped {
            "✓ "
        } else if sec.skipped {
            "- "
        } else if sec.active {
            "▸ "
        } else {
            "  "
        };
        lines.push(Line::from(Span::styled(
            format!("{prefix}── {} ──", sec.title),
            title_style,
        )));

        if sec.skipped {
            lines.push(Line::from(Span::styled(
                "    Skipped",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Fields
        for (fi, field) in sec.fields.iter().enumerate() {
            let is_active = sec.active && fi == app.field_idx && si == app.sec_idx;
            // Skip rendering the Configure/Skip select for completed sections
            if sec.completed && fi == 0 && sec.skippable {
                continue;
            }
            // Don't render future fields in active section
            if sec.active && fi > app.field_idx && !field.done {
                continue;
            }
            // Don't render fields for non-active, non-completed sections
            if !sec.active && !sec.completed {
                continue;
            }
            // Skip fields that were never visited (conditional flow)
            if sec.completed && !field.done {
                continue;
            }

            render_field(&mut lines, field, is_active);
        }
        lines.push(Line::from(""));
    }

    if app.finished {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "✓ Done! Try: myagent -p \"hello\"",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // Hint line at bottom
    if !app.finished {
        lines.push(Line::from(""));
        let hint = match app.current_field() {
            Some(Field {
                kind: FieldKind::Select { .. },
                ..
            }) => "↑↓ select  Enter confirm  Esc quit",
            _ => "Enter confirm  Esc quit",
        };
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_field(lines: &mut Vec<Line>, field: &Field, is_active: bool) {
    match &field.kind {
        FieldKind::Text { value, default } => {
            let label_style = if is_active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let display_val = if value.is_empty() {
                if let Some(d) = default {
                    if is_active {
                        format!("{d}")
                    } else if field.done {
                        d.clone()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                value.clone()
            };

            let mut spans = vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    format!("{}: ", field.label),
                    label_style,
                ),
            ];

            if is_active && value.is_empty() && default.is_some() {
                spans.push(Span::styled(
                    display_val.clone(),
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(
                    "█",
                    Style::default().fg(Color::White),
                ));
            } else if is_active {
                spans.push(Span::styled(
                    display_val.clone(),
                    Style::default().fg(Color::White),
                ));
                spans.push(Span::styled(
                    "█",
                    Style::default().fg(Color::White),
                ));
            } else {
                spans.push(Span::styled(
                    display_val,
                    Style::default().fg(Color::White),
                ));
            }
            lines.push(Line::from(spans));
        }
        FieldKind::Password { value } => {
            let label_style = if is_active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let masked = "*".repeat(value.len());
            let mut spans = vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    format!("{}: ", field.label),
                    label_style,
                ),
                Span::styled(masked, Style::default().fg(Color::White)),
            ];
            if is_active {
                spans.push(Span::styled(
                    "█",
                    Style::default().fg(Color::White),
                ));
            }
            lines.push(Line::from(spans));
        }
        FieldKind::Select {
            options, selected, ..
        } => {
            if !field.label.is_empty() {
                let label_style = if is_active {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(
                        format!("{}:", field.label),
                        label_style,
                    ),
                ]));
            }
            if is_active {
                for (i, opt) in options.iter().enumerate() {
                    let (marker, style) = if i == *selected {
                        (
                            "  ❯ ",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        ("    ", Style::default().fg(Color::DarkGray))
                    };
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(marker, style),
                        Span::styled(opt.clone(), style),
                    ]));
                }
            } else if field.done {
                let chosen = options[*selected].clone();
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(
                        chosen,
                        Style::default().fg(Color::Green),
                    ),
                ]));
            }
        }
    }
}

// ── Entry Point ──

pub fn run() -> Result<()> {
    let config_path = config::default_config_path();

    // Load existing config to pre-populate fields
    let existing = if config_path.exists() {
        config::AppConfig::load(&config_path).ok()
    } else {
        None
    };

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = InitApp::new();
    if let Some(cfg) = existing {
        app.prefill(&cfg);
    }

    loop {
        terminal.draw(|frame| render(frame, &app))?;

        if app.finished || app.cancelled {
            break;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                app.handle_key(key.code);
            }
            Event::Paste(text) => {
                app.handle_paste(text);
            }
            _ => {}
        }
    }

    let _ = stdout().execute(DisableBracketedPaste);
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    if app.cancelled {
        println!("Init cancelled.");
        return Ok(());
    }

    // Write config
    let config_json = app.build_config();
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&config_json)?,
    )?;
    println!("✓ Config saved to {}", config_path.display());
    println!("  Try: myagent -p \"hello\"");
    Ok(())
}
