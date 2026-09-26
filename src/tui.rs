use crate::event::{compact, fmt_ms, fmt_time, truncate, Event};
use crate::store::{self, Store};
use anyhow::Result;
use ratatui::{
    crossterm::event::{self, Event as CEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct Info {
    pub port: u16,
    pub cmd: Option<String>,
}

const LIVE: usize = 0;
const VARS: usize = 1;
const API: usize = 2;
const MAP: usize = 3;
const MEM: usize = 4;
const TABS: [&str; 5] = ["Live", "Vars", "API", "Map", "Mem"];

struct App {
    info: Info,
    tab: usize,
    /// selected row per tab; None = follow newest
    sel: [Option<usize>; 5],
    off: [usize; 5],
    len: [usize; 5],
    detail: bool,
    scroll: u16,
    filter: String,
    editing: bool,
    /// Vars tab: which snapshot of the selected variable (None = latest)
    step: Option<usize>,
    hist_len: usize,
    /// Mem tab: global memory step (None = latest)
    mem_step: Option<usize>,
    mem_len: usize,
    /// API tab: endpoints (grouped) instead of individual calls
    endpoints_mode: bool,
}

pub fn run(store: Arc<Store>, info: Info) -> Result<()> {
    let mut term = ratatui::init();
    let mut app = App { info, tab: LIVE, sel: [None; 5], off: [0; 5], len: [0; 5], detail: false, scroll: 0, filter: String::new(), editing: false, step: None, hist_len: 0, mem_step: None, mem_len: 0, endpoints_mode: false };
    let res = (|| -> Result<()> {
        loop {
            term.draw(|f| app.draw(f, &store))?;
            if event::poll(Duration::from_millis(120))? {
                if let CEvent::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press && app.key(k, &store) {
                        return Ok(());
                    }
                }
            }
        }
    })();
    ratatui::restore();
    res
}

impl App {
    fn key(&mut self, k: KeyEvent, store: &Store) -> bool {
        if self.editing {
            match k.code {
                KeyCode::Enter => self.editing = false,
                KeyCode::Esc => {
                    self.editing = false;
                    self.filter.clear();
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.sel[self.tab] = None;
            return false;
        }
        match k.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char(c @ '1'..='5') => self.set_tab(c as usize - '1' as usize),
            KeyCode::Tab => self.set_tab((self.tab + 1) % 5),
            KeyCode::BackTab => self.set_tab((self.tab + 4) % 5),
            KeyCode::Char('h') | KeyCode::Left if self.tab == MEM && self.mem_len > 0 => {
                let cur = self.mem_step.unwrap_or(self.mem_len - 1);
                self.mem_step = Some(cur.saturating_sub(1));
            }
            KeyCode::Char('l') | KeyCode::Right if self.tab == MEM && self.mem_len > 0 => {
                let cur = self.mem_step.unwrap_or(self.mem_len - 1);
                self.mem_step = if cur + 1 >= self.mem_len - 1 { None } else { Some(cur + 1) };
            }
            KeyCode::Char('j') | KeyCode::Down => self.mv(1),
            KeyCode::Char('k') | KeyCode::Up => self.mv(-1),
            KeyCode::Char('g') | KeyCode::Home => self.sel[self.tab] = Some(0),
            KeyCode::Char('G') | KeyCode::End => self.sel[self.tab] = None,
            KeyCode::PageDown | KeyCode::Char('J') => self.scroll = self.scroll.saturating_add(8),
            KeyCode::PageUp | KeyCode::Char('K') => self.scroll = self.scroll.saturating_sub(8),
            KeyCode::Enter => {
                self.detail = !self.detail;
                self.scroll = 0;
            }
            KeyCode::Char('h') | KeyCode::Left if self.tab == VARS && self.hist_len > 0 => {
                let cur = self.step.unwrap_or(self.hist_len - 1);
                self.step = Some(cur.saturating_sub(1));
            }
            KeyCode::Char('l') | KeyCode::Right if self.tab == VARS && self.hist_len > 0 => {
                let cur = self.step.unwrap_or(self.hist_len - 1);
                self.step = if cur + 1 >= self.hist_len - 1 { None } else { Some(cur + 1) };
            }
            KeyCode::Char('e') if self.tab == API => {
                self.endpoints_mode = !self.endpoints_mode;
                self.sel[API] = None;
                self.scroll = 0;
            }
            KeyCode::Char('/') => self.editing = true,
            KeyCode::Char('c') => {
                store.clear();
                self.sel = [None; 5];
                self.mem_step = None;
            }
            KeyCode::Esc => {
                if self.detail {
                    self.detail = false
                } else {
                    self.filter.clear()
                }
            }
            _ => {}
        }
        false
    }

    fn set_tab(&mut self, t: usize) {
        self.tab = t;
        self.scroll = 0;
    }

    fn mv(&mut self, d: i64) {
        let t = self.tab;
        let len = self.len[t];
        if len == 0 {
            return;
        }
        let cur = self.cur(t) as i64;
        let n = (cur + d).clamp(0, len as i64 - 1) as usize;
        self.sel[t] = if n == len - 1 && d > 0 && t != VARS && t != MAP { None } else { Some(n) };
        self.scroll = 0;
        if t == VARS {
            self.step = None;
        }
    }

    fn cur(&self, t: usize) -> usize {
        let len = self.len[t];
        match self.sel[t] {
            Some(i) => i.min(len.saturating_sub(1)),
            None if t == VARS || t == MAP => 0,
            None => len.saturating_sub(1),
        }
    }

    /// Scroll offset keeping `cur` visible in a window of height `h`.
    fn window(&mut self, t: usize, h: usize) -> usize {
        let cur = self.cur(t);
        let off = &mut self.off[t];
        if cur < *off {
            *off = cur;
        } else if h > 0 && cur >= *off + h {
            *off = cur + 1 - h;
        }
        if self.len[t] <= h {
            *off = 0;
        }
        *off
    }

    fn draw(&mut self, f: &mut Frame, store: &Store) {
        let [head, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
        self.header(f, head, store);
        match self.tab {
            LIVE => self.live(f, body, store),
            VARS => self.vars(f, body, store),
            API => self.api(f, body, store),
            MAP => self.map(f, body, store),
            _ => self.mem(f, body, store),
        }
        self.footer(f, foot);
    }

    fn header(&self, f: &mut Frame, area: Rect, store: &Store) {
        let (n, total, app_status) = store.with(|i| (i.events.len(), i.total, i.app_status.clone()));
        let mut spans = vec![Span::styled(" cdev ", Style::new().bold().fg(Color::Black).bg(Color::Green)), Span::raw(" ")];
        for (i, t) in TABS.iter().enumerate() {
            let s = format!(" {} {} ", i + 1, t);
            spans.push(if i == self.tab { Span::styled(s, Style::new().reversed().bold()) } else { Span::styled(s, Style::new().dim()) });
        }
        spans.push(Span::styled(format!("   http://localhost:{}  {n}/{total} events", self.info.port), Style::new().dim()));
        if let Some(c) = app_status.or_else(|| self.info.cmd.clone()) {
            let col = if c.starts_with("running") { Color::Green } else { Color::Yellow };
            spans.push(Span::styled(format!("  {}", truncate(&c, 60)), Style::new().fg(col)));
        }
        f.render_widget(Line::from(spans), area);
    }

    fn footer(&self, f: &mut Frame, area: Rect) {
        let line = if self.editing {
            Line::from(vec![Span::styled(" filter: ", Style::new().bold().fg(Color::Yellow)), Span::raw(&self.filter), Span::styled("█", Style::new().fg(Color::Yellow))])
        } else {
            let mut s = vec![Span::styled(" 1-5 tabs · j/k move · ←/→ step (Vars/Mem) · G follow · ⏎ detail · J/K scroll · / filter · c clear · q quit", Style::new().dim())];
            if !self.filter.is_empty() {
                s.push(Span::styled(format!("   filter: {}", self.filter), Style::new().fg(Color::Yellow)));
            }
            Line::from(s)
        };
        f.render_widget(line, area);
    }

    // ---------- Live ----------
    fn live(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        let filter = self.filter.clone();
        store.with(|inner| {
            let items: Vec<&Arc<Event>> = inner.events.iter().filter(|e| store::matches(e, None, &filter)).collect();
            self.len[LIVE] = items.len();
            let (list_area, detail_area) = self.split_detail(area);
            let h = list_area.height as usize;
            let off = self.window(LIVE, h);
            let cur = self.cur(LIVE);
            let w = list_area.width as usize;
            let lines: Vec<Line> = items.iter().enumerate().skip(off).take(h)
                .map(|(i, e)| row(e, w, i == cur && (self.sel[LIVE].is_some() || self.detail))).collect();
            if lines.is_empty() {
                f.render_widget(empty_hint(self.info.port), list_area);
            } else {
                f.render_widget(Paragraph::new(lines), list_area);
            }
            if let (Some(a), Some(e)) = (detail_area, items.get(cur)) {
                f.render_widget(Paragraph::new(detail_lines(e)).scroll((self.scroll, 0)).block(pane(" detail ")), a);
            }
        });
    }

    fn split_detail(&self, area: Rect) -> (Rect, Option<Rect>) {
        if self.detail {
            let [a, b] = Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
            (a, Some(b))
        } else {
            (area, None)
        }
    }

    // ---------- Vars ----------
    fn vars(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        let filter = self.filter.to_lowercase();
        store.with(|inner| {
            let vars: Vec<store::VarInfo> = store::vars(inner).into_iter()
                .filter(|v| filter.is_empty() || v.name.to_lowercase().contains(&filter) || v.svc.to_lowercase().contains(&filter))
                .collect();
            self.len[VARS] = vars.len();
            let [left, right] = Layout::horizontal([Constraint::Percentage(34), Constraint::Percentage(66)]).areas(area);
            let h = left.height.saturating_sub(2) as usize;
            let off = self.window(VARS, h);
            let cur = self.cur(VARS);
            let lines: Vec<Line> = vars.iter().enumerate().skip(off).take(h).map(|(i, v)| {
                let mut l = Line::from(vec![
                    Span::styled(format!("{} ", truncate(&v.svc, 10)), Style::new().fg(svc_color(&v.svc))),
                    Span::styled(v.name.clone(), Style::new().bold()),
                    Span::styled(format!(" ×{}  ", v.count), Style::new().dim()),
                    Span::styled(v.last.str("shape").to_string(), Style::new().fg(Color::Cyan).dim()),
                ]);
                if i == cur { l = l.style(Style::new().reversed()); }
                l
            }).collect();
            f.render_widget(Paragraph::new(lines).block(pane(" watched ")), left);

            let Some(v) = vars.get(cur) else {
                f.render_widget(Paragraph::new(vec![
                    Line::from(""),
                    Line::from("  No watched values yet.").bold(),
                    Line::from(""),
                    Line::from("  JS/TS:   cdev.w(value)            // returns value"),
                    Line::from("  Python:  cdev.w(value)"),
                    Line::from("  C++:     CDEV_W(expr)"),
                ]).block(pane(" history ")), right);
                return;
            };
            let hist = store::var_history(inner, &v.svc, &v.name);
            self.hist_len = hist.len();
            let step = self.step.unwrap_or(hist.len().saturating_sub(1)).min(hist.len().saturating_sub(1));
            let [top, bot] = Layout::vertical([Constraint::Percentage(30), Constraint::Percentage(70)]).areas(right);

            // history list, newest first, selected step highlighted
            let mut prev_shape = "";
            let mut rows: Vec<Line> = Vec::new();
            for (i, e) in hist.iter().enumerate() {
                let shape = e.str("shape");
                let norm = |s: &str| s.replace(" | null", "").replace(": null", ": T");
                let changed = !prev_shape.is_empty() && norm(prev_shape) != norm(shape);
                prev_shape = shape;
                let mut spans = vec![
                    Span::styled(format!("{:>4} ", i + 1), Style::new().fg(Color::DarkGray)),
                    Span::styled(format!("{} ", fmt_time(e.ts)), Style::new().dim()),
                    Span::raw(compact(e.get("v").unwrap_or(&Value::Null), top.width.saturating_sub(22) as usize)),
                ];
                if changed {
                    spans.push(Span::styled(format!("  ⚠ type → {shape}"), Style::new().fg(Color::Yellow)));
                }
                let mut l = Line::from(spans);
                if i == step && self.step.is_some() {
                    l = l.style(Style::new().reversed());
                }
                rows.push(l);
            }
            rows.reverse();
            let hh = top.height.saturating_sub(2) as usize;
            let from_top = hist.len().saturating_sub(1) - step;
            let hscroll = from_top.saturating_sub(hh.saturating_sub(1)) as u16;
            f.render_widget(Paragraph::new(rows).scroll((hscroll, 0)).block(pane(&format!(" {} · {} snapshots ", v.name, hist.len()))), top);

            let Some(e) = hist.get(step) else { return };
            let prev = if step > 0 { hist.get(step - 1).and_then(|p| p.get("v")) } else { None };
            let val = e.get("v").unwrap_or(&Value::Null);
            let mut d = vec![
                Line::from(vec![Span::styled("type  ", Style::new().dim()), Span::styled(e.str("shape").to_string(), Style::new().fg(Color::Cyan))]),
                Line::from(vec![Span::styled("at    ", Style::new().dim()), Span::raw(format!("{}  {}", e.str("loc"), fmt_time(e.ts)))]),
            ];
            let title;
            match crate::structure::render(val, prev, &v.name, bot.width.saturating_sub(2) as usize) {
                Some(view) => {
                    title = format!(" {} · step {}/{} ", view.kind.name(), step + 1, hist.len());
                    if !view.note.is_empty() {
                        d.push(Line::from(vec![Span::styled("step  ", Style::new().dim()), Span::styled(view.note, Style::new().fg(Color::Yellow).bold())]));
                    }
                    d.push(Line::from(""));
                    d.extend(view.lines);
                }
                None => {
                    title = format!(" value · step {}/{} ", step + 1, hist.len());
                    d.push(Line::from(""));
                    d.extend(pretty(Some(val)));
                }
            }
            f.render_widget(Paragraph::new(d).scroll((self.scroll, 0)).block(pane(&title)), bot);
        });
    }

    // ---------- API ----------
    fn api(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        if self.endpoints_mode {
            return self.api_endpoints(f, area, store);
        }
        let filter = self.filter.to_lowercase();
        let calls: Vec<crate::api::Call> = store.with(crate::api::calls).into_iter()
            .filter(|c| filter.is_empty() || format!("{} {} {} {}", c.method, c.path, c.from, c.endpoint).to_lowercase().contains(&filter))
            .collect();
        self.len[API] = calls.len();
        let [list, detail] = Layout::vertical([Constraint::Percentage(36), Constraint::Percentage(64)]).areas(area);
        let h = list.height.saturating_sub(2) as usize;
        let off = self.window(API, h);
        let cur = self.cur(API);
        let lines: Vec<Line> = calls.iter().enumerate().skip(off).take(h).map(|(i, c)| {
            let sc = status_color(c.status, c.err);
            let both = match c.paired { Some("cid") => "⇄ ", Some(_) => "≈ ", None => "  " };
            let mut l = Line::from(vec![
                Span::styled(format!("{} ", fmt_time(c.ts)), Style::new().dim()),
                Span::styled(both, Style::new().fg(Color::Cyan).bold()),
                Span::styled(format!("{:<6}", c.method), Style::new().bold()),
                Span::styled(format!("{:>4} ", c.status.map(|s| format!("{s}")).unwrap_or_else(|| "ERR".into())), Style::new().fg(sc).bold()),
                Span::styled(format!("{:>8}  ", fmt_ms(c.ms)), Style::new().dim()),
                Span::raw(c.path.clone()),
                Span::styled(format!("   {} → {}", c.from, c.endpoint), Style::new().fg(Color::DarkGray)),
            ]);
            if i == cur && self.sel[API].is_some() { l = l.style(Style::new().reversed()); }
            l
        }).collect();
        f.render_widget(Paragraph::new(lines).block(pane(" calls  (⇄ both sides · ≈ paired by timing · e: endpoints) ")), list);

        let Some(c) = calls.get(cur) else {
            f.render_widget(Paragraph::new("  No HTTP traffic yet. Run your app with `cdev run -- …` or load /cdev.js in the page.").block(pane(" call ")), detail);
            return;
        };
        let rows: Vec<(&str, &Arc<Event>)> = [("client", c.client.as_ref()), ("server", c.server.as_ref())]
            .into_iter().filter_map(|(k, e)| e.map(|e| (k, e))).collect();
        let areas = Layout::vertical(vec![Constraint::Ratio(1, rows.len() as u32); rows.len()]).split(detail);
        for ((side, e), a) in rows.iter().zip(areas.iter()) {
            let [req, res] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(*a);
            let who = if *side == "client" {
                format!(" client · {}{} sent ", e.svc, match e.str("caller") { "" => String::new(), f => format!(" in {f}()") })
            } else {
                format!(" server · {}{} received ", e.svc, match e.str("handler") { "" => String::new(), h => format!(" ({h})") })
            };
            let mut rq = vec![Line::from(vec![Span::styled(format!("{} ", e.str("method")), Style::new().bold()), Span::raw(e.str("url").to_string())]), Line::from("")];
            rq.extend(http_side(e.get("req")));
            let status = e.num("status");
            let mut rs = vec![Line::from(vec![
                Span::styled(status.map(|s| format!("{s}")).unwrap_or_else(|| format!("ERR {}", e.str("err"))), Style::new().bold().fg(status_color(status, e.get("err").is_some()))),
                Span::styled(format!("   {}", fmt_ms(e.num("ms"))), Style::new().dim()),
            ]), Line::from("")];
            rs.extend(http_side(e.get("res")));
            let back = if *side == "client" { " response received " } else { " response sent " };
            f.render_widget(Paragraph::new(rq).scroll((self.scroll, 0)).block(pane(&who)), req);
            f.render_widget(Paragraph::new(rs).scroll((self.scroll, 0)).block(pane(back)), res);
        }
    }

    fn api_endpoints(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        let filter = self.filter.to_lowercase();
        let (eps, calls) = store.with(|i| (crate::api::endpoints(i), crate::api::calls(i)));
        let eps: Vec<crate::api::Endpoint> = eps.into_iter()
            .filter(|e| filter.is_empty() || format!("{} {} {}", e.method, e.route, e.endpoint).to_lowercase().contains(&filter)).collect();
        self.len[API] = eps.len();
        let [list, detail] = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);
        let h = list.height.saturating_sub(2) as usize;
        let off = self.window(API, h);
        let cur = self.cur(API).min(eps.len().saturating_sub(1));
        let lines: Vec<Line> = eps.iter().enumerate().skip(off).take(h).map(|(i, e)| {
            let mut spans = vec![
                Span::styled(format!("{:<6}", e.method), Style::new().bold()),
                Span::styled(format!("{:<30}", truncate(&e.route, 30)), Style::new()),
                Span::styled(format!(" {:<14}", truncate(&e.endpoint, 14)), Style::new().fg(svc_color(&e.endpoint))),
                Span::raw(format!(" ×{:<4}", e.count)),
                Span::styled(format!(" avg {:>8}", fmt_ms(Some(e.avg_ms))), Style::new().dim()),
                Span::raw("  "),
            ];
            for (st, n) in &e.statuses {
                let code = st.parse::<f64>().ok();
                spans.push(Span::styled(format!("{st}×{n} "), Style::new().fg(status_color(code, code.is_none()))));
            }
            let mut l = Line::from(spans);
            if i == cur && self.sel[API].is_some() { l = l.style(Style::new().reversed()); }
            l
        }).collect();
        f.render_widget(Paragraph::new(lines).block(pane(" endpoints  (e: calls) ")), list);
        let Some(e) = eps.get(cur) else {
            f.render_widget(Paragraph::new("  No endpoints yet.").block(pane(" endpoint ")), detail);
            return;
        };
        let dim = Style::new().dim();
        let mut d = vec![
            Line::from(vec![Span::styled(format!("{} {}", e.method, e.route), Style::new().bold()), Span::styled(format!("  on {}", e.endpoint), Style::new().fg(svc_color(&e.endpoint)))]),
            Line::from(Span::styled(format!("{} calls · {} errors · avg {} · max {}", e.count, e.errors, fmt_ms(Some(e.avg_ms)), fmt_ms(Some(e.max_ms))), dim)),
            Line::from(""),
            Line::from(vec![Span::styled("called by   ", dim), Span::raw(e.callers.join(", "))]),
            Line::from(vec![Span::styled("handled by  ", dim), Span::raw(if e.handlers.is_empty() { "–".to_string() } else { e.handlers.join(", ") })]),
            Line::from(""),
            Line::from(Span::styled("request body", dim)),
        ];
        for s in if e.req_shapes.is_empty() { vec!["(none seen)".to_string()] } else { e.req_shapes.clone() } {
            d.push(Line::from(Span::styled(format!("  {s}"), Style::new().fg(Color::Cyan))));
        }
        d.push(Line::from(Span::styled("response body", dim)));
        for s in if e.res_shapes.is_empty() { vec!["(none seen)".to_string()] } else { e.res_shapes.clone() } {
            d.push(Line::from(Span::styled(format!("  {s}"), Style::new().fg(Color::Cyan))));
        }
        d.push(Line::from(""));
        d.push(Line::from(Span::styled("recent calls", dim)));
        let by_id: std::collections::HashMap<u64, &crate::api::Call> = calls.iter().map(|c| (c.id, c)).collect();
        for id in e.recent.iter().rev().take(12) {
            if let Some(c) = by_id.get(id) {
                let body = c.server.as_ref().or(c.client.as_ref())
                    .and_then(|s| s.get("res")).and_then(|r| r.get("body")).and_then(Value::as_str).unwrap_or("");
                d.push(Line::from(vec![
                    Span::styled(format!("  {} ", fmt_time(c.ts)), dim),
                    Span::styled(format!("{:>4} ", c.status.map(|s| format!("{s}")).unwrap_or_else(|| "ERR".into())), Style::new().fg(status_color(c.status, c.err))),
                    Span::styled(format!("{:>8}  ", fmt_ms(c.ms)), dim),
                    Span::raw(format!("{}  ", c.path)),
                    Span::styled(truncate(body, 60), dim),
                ]));
            }
        }
        f.render_widget(Paragraph::new(d).scroll((self.scroll, 0)).block(pane(" endpoint ")), detail);
    }

    // ---------- Memory ----------
    fn mem(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        let mv = store.with(|i| crate::memory::view(i, self.mem_step));
        self.mem_len = mv.steps;
        let [top, body] = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);
        let head = if mv.steps == 0 {
            vec![Line::from("  No memory info yet. C++ CDEV_W / Rust cdev_w!(&x) / Python cdev.w / JS cdev.w attach addresses.").dim()]
        } else {
            vec![
                Line::from(vec![
                    Span::styled(format!(" step {}/{}  ", mv.step + 1, mv.steps), Style::new().bold()),
                    Span::styled("←/→ to step", Style::new().dim()),
                ]),
                Line::from(Span::styled(format!(" {}", mv.note), Style::new().fg(Color::Yellow).bold())),
            ]
        };
        f.render_widget(Paragraph::new(head).block(Block::default().borders(Borders::BOTTOM).border_style(Style::new().fg(Color::DarkGray))), top);
        let [left, right] = Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)]).areas(body);

        // short handles for heap blocks so names can point at them: #1, #2…
        let handle: std::collections::HashMap<&str, usize> = mv.blocks.iter().enumerate().map(|(i, b)| (b.addr.as_str(), i + 1)).collect();
        let mut l: Vec<Line> = Vec::new();
        for nm in &mv.names {
            let st = if nm.changed { Style::new().fg(Color::Black).bg(Color::Yellow).bold() } else { Style::new().bold() };
            l.push(Line::from(vec![
                Span::styled(nm.name.clone(), st),
                Span::styled(format!("  {}", truncate(&nm.ty, left.width.saturating_sub(nm.name.len() as u16 + 6) as usize)), Style::new().fg(Color::Cyan)),
            ]));
            let mut spans = vec![Span::raw("   ")];
            match (&nm.addr, nm.region.as_str()) {
                (Some(a), "stack" | "static" | "value") => spans.push(Span::styled(format!("{a} · {}", nm.region), Style::new().fg(Color::DarkGray))),
                _ => spans.push(Span::styled("name", Style::new().fg(Color::DarkGray))),
            }
            if let Some(sz) = nm.size {
                spans.push(Span::styled(format!(" · {sz} B"), Style::new().dim()));
            }
            if let Some((a, off)) = &nm.inside {
                spans.push(Span::styled(format!(" · inside #{} +{off}", handle.get(a.as_str()).copied().unwrap_or(0)), Style::new().fg(Color::Cyan)));
            }
            if let Some(t) = &nm.to_name {
                spans.push(Span::styled(format!("  ──▶ {t}"), Style::new().fg(Color::Magenta).bold()));
            } else if let Some(h) = nm.to.as_deref().and_then(|a| handle.get(a)) {
                spans.push(Span::styled(format!("  ──▶ #{h}"), Style::new().fg(Color::Magenta).bold()));
            }
            if let Some(r) = nm.refs {
                spans.push(Span::styled(format!("  refs {r}"), Style::new().fg(Color::Yellow)));
            }
            l.push(Line::from(spans));
            l.push(Line::from(Span::styled(format!("   {}", truncate(&nm.value, left.width.saturating_sub(6) as usize)), Style::new().dim())));
        }
        f.render_widget(Paragraph::new(l).block(pane(" stack · names ")), left);

        let mut r: Vec<Line> = Vec::new();
        for (i, b) in mv.blocks.iter().enumerate() {
            let st = if b.changed { Style::new().fg(Color::Black).bg(Color::Yellow).bold() } else { Style::new().bold().fg(Color::Magenta) };
            let mut spans = vec![Span::styled(format!("#{:<2}", i + 1), st), Span::styled(format!(" {}", b.addr), Style::new().fg(Color::DarkGray))];
            if let Some(sz) = b.size {
                spans.push(Span::styled(format!("  {sz} B"), Style::new().dim()));
            }
            if b.region != "heap" {
                spans.push(Span::styled(format!("  [{}]", b.region), Style::new().fg(Color::Cyan)));
            }
            spans.push(Span::raw(format!("  {}", b.owners.join(", "))));
            if b.shared > 1 {
                spans.push(Span::styled(format!("  shared ×{}", b.shared), Style::new().fg(Color::Red).bold()));
            }
            if let Some((a, off)) = &b.inside {
                spans.push(Span::styled(format!("  inside #{} +{off}", handle.get(a.as_str()).copied().unwrap_or(0)), Style::new().fg(Color::Cyan)));
            }
            r.push(Line::from(spans));
            if b.kind == "buffer" {
                let cap = b.cap.unwrap_or(b.cells.len() as u64) as usize;
                let shown = cap.min(24);
                let mut cells = String::from("    ");
                for k in 0..shown {
                    match b.cells.get(k) {
                        Some(c) => cells.push_str(&format!("[{}]", truncate(c, 6))),
                        None => cells.push_str("[ ]"),
                    }
                }
                if cap > shown {
                    cells.push('…');
                }
                r.push(Line::from(cells));
                r.push(Line::from(Span::styled(
                    format!("    len {} / cap {} × {} B", b.len.unwrap_or(0), b.cap.unwrap_or(0), b.elem.unwrap_or(0)),
                    Style::new().dim(),
                )));
            } else if !b.cells.is_empty() {
                r.push(Line::from(Span::styled(format!("    {}", truncate(&b.cells.join(" "), right.width.saturating_sub(8) as usize)), Style::new().dim())));
            }
            if !b.links.is_empty() {
                let ls: Vec<String> = b.links.iter().map(|(lbl, t)| format!("{lbl}→#{}", handle.get(t.as_str()).copied().unwrap_or(0))).collect();
                r.push(Line::from(Span::styled(format!("    {}", ls.join("  ")), Style::new().fg(Color::Magenta))));
            }
        }
        if r.is_empty() && mv.steps > 0 {
            r.push(Line::from("  nothing on the heap in these snapshots").dim());
        }
        f.render_widget(Paragraph::new(r).scroll((self.scroll, 0)).block(pane(" heap (by address) ")), right);
    }

    // ---------- Map ----------
    fn map(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        let (edges, mut ports) = store.with(|i| (store::edges(i), i.ports.iter().map(|(p, n)| (*p, n.clone())).collect::<Vec<_>>()));
        ports.sort();
        let mut lines: Vec<Line> = Vec::new();
        if !ports.is_empty() {
            let mut s = vec![Span::styled("services  ", Style::new().dim())];
            for (p, n) in &ports {
                s.push(Span::styled(format!("{n}", ), Style::new().fg(svc_color(n)).bold()));
                s.push(Span::styled(format!(":{p}   "), Style::new().dim()));
            }
            lines.push(Line::from(s));
            lines.push(Line::from(""));
        }
        if edges.is_empty() {
            lines.push(Line::from("  Nothing talking to anything yet. HTTP traffic and traced calls show up here.").dim());
        }
        let mut i = 0;
        while i < edges.len() {
            let from = edges[i].from.clone();
            lines.push(Line::from(Span::styled(from.clone(), Style::new().bold().fg(svc_color(from.split('·').next().unwrap_or(""))))));
            let group: Vec<&store::Edge> = edges[i..].iter().take_while(|e| e.from == from).collect();
            let n = group.len();
            for (j, e) in group.iter().enumerate() {
                let last = j == n - 1;
                let (branch, cont) = if last { ("  └─", "     ") } else { ("  ├─", "  │  ") };
                let arrow = if e.via == "call" { "ƒ▶ " } else { "─▶ " };
                let mut s = vec![
                    Span::styled(format!("{branch}{arrow}"), Style::new().dim()),
                    Span::styled(format!("{:<28}", e.to), Style::new().fg(svc_color(e.to.split('·').next().unwrap_or(""))).bold()),
                    Span::raw(format!(" ×{:<5}", e.count)),
                    Span::styled(format!(" avg {:>8}", fmt_ms(Some(e.avg_ms))), Style::new().dim()),
                ];
                if e.errors > 0 {
                    s.push(Span::styled(format!("  {} err", e.errors), Style::new().fg(Color::Red).bold()));
                }
                lines.push(Line::from(s));
                if e.via == "http" {
                    let labels = e.labels.iter().map(|(l, c)| format!("{l} ×{c}")).collect::<Vec<_>>().join(" · ");
                    lines.push(Line::from(vec![Span::styled(cont, Style::new().dim()), Span::styled(format!("   {labels}"), Style::new().fg(Color::DarkGray))]));
                }
            }
            lines.push(Line::from(""));
            i += n;
        }
        self.len[MAP] = lines.len();
        let scroll = self.cur(MAP) as u16;
        f.render_widget(Paragraph::new(lines).scroll((scroll, 0)).block(pane(" what talks to what ")), area);
    }
}

// ---------- helpers ----------

fn pane(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).border_style(Style::new().fg(Color::DarkGray)).title(Span::styled(title, Style::new().bold()))
}

fn empty_hint(port: u16) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(""),
        Line::from("  Waiting for events…").bold(),
        Line::from(""),
        Line::from("  cdev run -- npm run dev            wrap a Node/Python dev server (auto-instrumented)"),
        Line::from(format!("  <script src=\"http://localhost:{port}/cdev.js\"></script>   in any page")),
        Line::from(format!("  web panel:  http://localhost:{port}")),
    ])
}

fn row(e: &Event, width: usize, selected: bool) -> Line<'static> {
    let (badge, col) = kind_badge(e);
    let loc = e.str("loc");
    let head = 13 + 11 + 6;
    let room = width.saturating_sub(head);
    let loc_txt = if loc.is_empty() { String::new() } else { format!("  {loc}") };
    let sum_room = room.saturating_sub(loc_txt.chars().count().min(room / 3));
    let summary = truncate(&e.summary().replace('\n', " ⏎ "), sum_room.max(10));
    let mut spans = vec![
        Span::styled(format!("{} ", fmt_time(e.ts)), Style::new().dim()),
        Span::styled(format!("{:<10} ", truncate(&e.svc, 10)), Style::new().fg(svc_color(&e.svc))),
        Span::styled(format!("{badge:<5} "), Style::new().fg(col).bold()),
        Span::styled(summary, Style::new().fg(if e.level() == "error" { Color::Red } else if e.level() == "warn" { Color::Yellow } else { Color::Reset })),
    ];
    if !loc_txt.is_empty() {
        spans.push(Span::styled(truncate(&loc_txt, room / 3), Style::new().fg(Color::DarkGray)));
    }
    let l = Line::from(spans);
    if selected { l.style(Style::new().add_modifier(Modifier::REVERSED)) } else { l }
}

fn kind_badge(e: &Event) -> (&'static str, Color) {
    match e.kind.as_str() {
        "watch" => ("WATCH", Color::Cyan),
        "call" => ("CALL", Color::Magenta),
        "http" => ("HTTP", if e.level() == "error" { Color::Red } else { Color::Blue }),
        "error" => ("ERROR", Color::Red),
        "service" => ("SVC", Color::Green),
        _ => match e.level() {
            "error" => ("ERR", Color::Red),
            "warn" => ("WARN", Color::Yellow),
            "debug" => ("DEBUG", Color::DarkGray),
            "stderr" => ("STDER", Color::LightYellow),
            _ => ("LOG", Color::Gray),
        },
    }
}

fn detail_lines(e: &Event) -> Vec<Line<'static>> {
    let mut v = vec![
        Line::from(vec![Span::styled(format!("{} ", e.kind.to_uppercase()), Style::new().bold().fg(kind_badge(e).1)), Span::raw(e.summary())]),
        Line::from(Span::styled(format!("{}  ·  {}  ·  {}  {}", fmt_time(e.ts), e.svc, e.str("lang"), e.str("loc")), Style::new().dim())),
    ];
    match e.kind.as_str() {
        "call" => v.push(Line::from(Span::styled(e.signature(), Style::new().fg(Color::Cyan)))),
        "watch" => v.push(Line::from(Span::styled(format!("type: {}", e.str("shape")), Style::new().fg(Color::Cyan)))),
        "log" => {
            if let Some(Value::Array(s)) = e.get("argShapes") {
                let t: Vec<&str> = s.iter().filter_map(Value::as_str).collect();
                v.push(Line::from(Span::styled(format!("args: ({})", t.join(", ")), Style::new().fg(Color::Cyan))));
            }
        }
        _ => {}
    }
    v.push(Line::from(""));
    let mut data = e.data.clone();
    for k in ["argShapes", "retShape", "shape", "loc", "lang", "text"] {
        data.remove(k);
    }
    v.extend(pretty(Some(&Value::Object(data))));
    v
}

fn pretty(v: Option<&Value>) -> Vec<Line<'static>> {
    let s = match v {
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(j @ (Value::Object(_) | Value::Array(_))) => serde_json::to_string_pretty(&j).unwrap_or_default(),
            _ => s.clone(),
        },
        Some(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
        None => String::new(),
    };
    s.lines().map(|l| Line::from(l.to_string())).collect()
}

fn http_side(side: Option<&Value>) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let Some(side) = side else { return out };
    if let Some(Value::Object(h)) = side.get("headers") {
        for (k, v) in h {
            out.push(Line::from(vec![
                Span::styled(format!("{k}: "), Style::new().fg(Color::DarkGray)),
                Span::raw(v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())),
            ]));
        }
        out.push(Line::from(""));
    }
    match side.get("body") {
        None | Some(Value::Null) => out.push(Line::from(Span::styled("(no body)", Style::new().dim()))),
        Some(b) => out.extend(pretty(Some(b))),
    }
    out
}

fn status_color(s: Option<f64>, err: bool) -> Color {
    match s {
        _ if err => Color::Red,
        Some(s) if s >= 500.0 => Color::Red,
        Some(s) if s >= 400.0 => Color::Yellow,
        Some(s) if s >= 300.0 => Color::Cyan,
        Some(_) => Color::Green,
        None => Color::DarkGray,
    }
}

fn svc_color(s: &str) -> Color {
    const C: [Color; 7] = [Color::Green, Color::Blue, Color::Magenta, Color::Cyan, Color::Yellow, Color::LightBlue, Color::LightMagenta];
    let h = s.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    C[(h as usize) % C.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::json;

    fn ev(v: serde_json::Value) -> Event {
        serde_json::from_value(v).unwrap()
    }

    /// Renders every tab against sample data; `cargo test -- --nocapture` prints them.
    #[test]
    fn renders_all_tabs() {
        let store = Store::new(1000);
        store.push(ev(json!({"kind": "service", "svc": "api", "name": "api", "port": 3000})));
        store.push(ev(json!({"kind": "log", "svc": "api", "level": "warn", "text": "slow query", "loc": "db.js:12"})));
        for (i, arr) in [[5, 1, 4], [1, 5, 4], [1, 4, 5]].iter().enumerate() {
            store.push(ev(json!({"kind": "watch", "svc": "api", "name": "sort", "v": {"arr": arr, "i": 0, "j": i}})));
        }
        store.push(ev(json!({"kind": "call", "svc": "api", "name": "getUser", "params": ["id"], "args": [2], "ret": {"id": 2}, "ms": 1.5, "caller": "GET /u/2"})));
        store.push(ev(json!({"kind": "http", "svc": "api", "dir": "in", "method": "GET", "url": "http://localhost:3000/u/2", "status": 200, "ms": 3.0, "from": "browser",
            "req": {"headers": {"accept": "*/*"}}, "res": {"headers": {"content-type": "application/json"}, "body": "{\"id\":2}"}})));
        let mut app = App { info: Info { port: 4400, cmd: None }, tab: LIVE, sel: [None; 5], off: [0; 5], len: [0; 5], detail: true, scroll: 0,
            filter: String::new(), editing: false, step: None, hist_len: 0, mem_step: None, mem_len: 0, endpoints_mode: false };
        let mut term = Terminal::new(TestBackend::new(110, 24)).unwrap();
        store.push(ev(json!({"kind": "watch", "svc": "api", "name": "v", "t": "std::vector<int>", "v": [1, 2, 3],
            "mem": {"addr": "0x16b9ae9f0", "size": 24, "region": "stack", "heap": {"addr": "0x143e05db0", "len": 3, "cap": 4, "elem": 4, "region": "heap"}}})));
        store.push(ev(json!({"kind": "watch", "svc": "api", "name": "p", "t": "int*", "v": "0x16b9ae9f0",
            "mem": {"addr": "0x16b9ae998", "size": 8, "region": "stack", "ptr": "0x16b9ae9f0"}})));
        for tab in [LIVE, VARS, API, MAP, MEM] {
            app.tab = tab;
            term.draw(|f| app.draw(f, &store)).unwrap();
            let buf = term.backend().buffer().clone();
            let text: String = (0..buf.area.height).map(|y| {
                (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n"
            }).collect();
            println!("{text}");
            assert!(text.contains("cdev"));
        }
    }
}
