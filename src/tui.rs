use std::time::Instant;
use tuirealm::{
    command::{Cmd, CmdResult},
    component::{AppComponent, Component},
    event::{Event, Key, KeyModifiers, MouseEventKind, NoUserEvent},
    props::{AttrValue, Attribute, QueryResult},
    ratatui::{
        layout::{Alignment, Constraint, Direction, Layout, Rect},
        style::{Color, Style, Stylize},
        text::{Line, Span},
        widgets::{Block, Borders, Gauge, Paragraph, Wrap},
        Frame,
    },
    state::State,
};
use zhiyuan_core::{ProgressUpdate, QualityScore, ResearchPlan, ResearchReport, SubTask};

#[derive(Debug, Clone, Default)]
struct TaskStat {
    phase: String,
    pages_total: usize,
    pages_ok: usize,
    pages_fail: usize,
    tokens_out: usize,
}

const GOLD: Color = Color::Rgb(212, 167, 106);
const TEAL: Color = Color::Rgb(91, 173, 171);
const RED: Color = Color::Rgb(196, 85, 76);
const GRAY: Color = Color::Rgb(136, 143, 160);
const STEEL: Color = Color::Rgb(74, 111, 165);
const WARM: Color = Color::Rgb(232, 213, 183);

fn tasks_summary(sub_tasks: &[SubTask]) -> String {
    sub_tasks
        .iter()
        .map(|t| t.description.as_str())
        .collect::<Vec<_>>()
        .join("  ◆  ")
}

#[derive(Debug, Clone)]
pub enum TuiEvent {
    PlanReady(ResearchPlan),
    Progress(ProgressUpdate),
    LogLine(String),
    TokenUsage(usize, usize),
    PdfMessage(String),
    PdfDone,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Msg {
    Quit,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum Id {
    App,
}

struct InputBuf {
    text: String,
    cursor: usize,
}

impl InputBuf {
    fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }
    fn push(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }
    fn pop(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor].chars().next_back().unwrap();
            self.text.remove(self.cursor - prev.len_utf8());
            self.cursor -= prev.len_utf8();
        }
    }
    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
    fn value(&self) -> &str {
        &self.text
    }
    fn cursor_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor].chars().next_back().unwrap();
            self.cursor -= prev.len_utf8();
        }
    }
    fn cursor_right(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.text[self.cursor..].chars().next().unwrap();
            self.cursor += next.len_utf8();
        }
    }
    fn cursor_home(&mut self) {
        self.cursor = 0;
    }
    fn cursor_end(&mut self) {
        self.cursor = self.text.len();
    }
    fn cursor_char_idx(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }
}

enum Phase {
    Loading,
    PlanReview {
        plan: ResearchPlan,
        input: InputBuf,
        feedback_pending: bool,
        versions: Vec<String>,
        version_scroll: usize,
    },
    Researching {
        start_time: Instant,
        iteration: usize,
        max_iterations: usize,
        quality: Option<QualityScore>,
        findings_count: usize,
        log_lines: Vec<String>,
        spinner_frame: usize,
        log_scroll: usize,
        tasks: Vec<String>,
        task_stats: Vec<TaskStat>,
        current_task: usize,
        pages_total: usize,
        pages_ok: usize,
        pages_fail: usize,
        tokens_in: usize,
        tokens_out: usize,
    },
    PdfGenerating {
        report: ResearchReport,
        messages: Vec<String>,
        done: bool,
    },
    Error(String),
}

pub struct App {
    phase: Phase,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
    query_text: String,
    plan_feedback_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    needs_pdf: bool,
}

impl App {
    pub fn new(
        query_text: String,
        event_rx: tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
        plan_feedback_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Self {
        Self {
            phase: Phase::Loading,
            event_rx,
            query_text,
            plan_feedback_tx,
            needs_pdf: false,
        }
    }

    pub fn take_pdf_request(&mut self) -> bool {
        std::mem::replace(&mut self.needs_pdf, false)
    }

    pub fn report(&self) -> Option<&ResearchReport> {
        match &self.phase {
            Phase::PdfGenerating { report, .. } => Some(report),
            _ => None,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                TuiEvent::PlanReady(new_plan) => {
                    let version = tasks_summary(&new_plan.sub_tasks);
                    if let Phase::PlanReview {
                        ref mut plan,
                        ref mut versions,
                        ref mut feedback_pending,
                        ..
                    } = self.phase
                    {
                        versions.push(version);
                        *feedback_pending = false;
                        *plan = new_plan;
                        return;
                    }
                    self.phase = Phase::PlanReview {
                        plan: new_plan,
                        input: InputBuf::new(),
                        feedback_pending: false,
                        versions: vec![version],
                        version_scroll: 0,
                    };
                }
                TuiEvent::Progress(u) => self.handle_progress(u),
                TuiEvent::LogLine(l) => self.add_log(l),
                TuiEvent::TokenUsage(prompt_tok, completion_tok) => {
                    if let Phase::Researching {
                        ref mut tokens_in,
                        ref mut tokens_out,
                        ..
                    } = self.phase
                    {
                        *tokens_in += prompt_tok;
                        *tokens_out += completion_tok;
                    }
                }
                TuiEvent::PdfMessage(msg) => {
                    if let Phase::PdfGenerating {
                        ref mut messages, ..
                    } = self.phase
                    {
                        messages.push(msg);
                    }
                }
                TuiEvent::PdfDone => {
                    if let Phase::PdfGenerating {
                        ref mut done,
                        ref mut messages,
                        ..
                    } = self.phase
                    {
                        *done = true;
                        messages.push("PDF 生成完成".into());
                    }
                }
            }
        }
    }

    fn start_researching(&mut self, tasks: Vec<String>) {
        let stats = tasks
            .iter()
            .map(|_| TaskStat {
                phase: "待处理".into(),
                ..Default::default()
            })
            .collect();
        self.phase = Phase::Researching {
            start_time: Instant::now(),
            iteration: 0,
            max_iterations: 4,
            quality: None,
            findings_count: 0,
            log_lines: Vec::new(),
            spinner_frame: 0,
            log_scroll: 0,
            tasks,
            task_stats: stats,
            current_task: 0,
            pages_total: 0,
            pages_ok: 0,
            pages_fail: 0,
            tokens_in: 0,
            tokens_out: 0,
        };
    }

    fn fire_research(&mut self, plan: ResearchPlan) {
        let tasks: Vec<String> = plan
            .sub_tasks
            .iter()
            .map(|t| t.description.clone())
            .collect();
        self.plan_feedback_tx.take();
        self.start_researching(tasks);
    }

    fn handle_progress(&mut self, update: ProgressUpdate) {
        match update {
            ProgressUpdate::Started {
                max_iterations: mi,
                tasks,
            } => {
                if let Phase::Researching {
                    ref mut max_iterations,
                    ..
                } = self.phase
                {
                    *max_iterations = mi;
                } else {
                    self.start_researching(tasks);
                    if let Phase::Researching {
                        ref mut max_iterations,
                        ..
                    } = self.phase
                    {
                        *max_iterations = mi;
                    }
                }
            }
            ProgressUpdate::Phase {
                name: _,
                message: _,
            } => {
                if let Phase::Researching {
                    ref mut current_task,
                    ref tasks,
                    ..
                } = self.phase
                {
                    if *current_task < tasks.len() {
                        *current_task += 1;
                    }
                }
            }
            ProgressUpdate::Iteration {
                iteration: it,
                max_iterations: mi,
                quality: q,
                findings_count: fc,
                ..
            } => {
                if let Phase::Researching {
                    ref mut iteration,
                    ref mut max_iterations,
                    ref mut quality,
                    ref mut findings_count,
                    ..
                } = self.phase
                {
                    *iteration = it;
                    *max_iterations = mi;
                    *quality = q;
                    *findings_count = fc;
                }
            }
            ProgressUpdate::Report(report) => {
                self.needs_pdf = true;
                self.phase = Phase::PdfGenerating {
                    messages: vec!["研究完成，正在生成 PDF...".into()],
                    done: false,
                    report,
                };
            }
            ProgressUpdate::TaskPhase { task_desc, phase } => {
                if let Phase::Researching {
                    ref mut task_stats,
                    ref tasks,
                    ..
                } = self.phase
                {
                    if let Some(idx) = tasks.iter().position(|t| t == &task_desc) {
                        if idx < task_stats.len() {
                            task_stats[idx].phase = phase;
                        }
                    }
                }
            }
            ProgressUpdate::Error(e) => {
                self.phase = Phase::Error(e);
            }
        }
    }

    fn strip_log_prefix(s: &str) -> String {
        if let Some(after_ts) = s.find("Z  ") {
            let rest = &s[after_ts + 3..];
            if let Some(level_end) = rest.find(' ') {
                return rest[level_end + 1..].trim().to_string();
            }
        }
        s.to_string()
    }

    fn add_log(&mut self, line: String) {
        let line = Self::strip_log_prefix(&line);
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            return;
        }

        if let Phase::Researching {
            ref mut log_lines,
            ref mut pages_total,
            ref mut pages_ok,
            ref mut pages_fail,
            ref mut tokens_out,
            ref mut task_stats,
            ref current_task,
            ..
        } = self.phase
        {
            if trimmed.contains("提取器选定URL 总数=") {
                if let Some(n) = trimmed
                    .rsplit('=')
                    .next()
                    .and_then(|s| s.trim().parse().ok())
                {
                    *pages_total = n;
                    if *current_task < task_stats.len() {
                        task_stats[*current_task].pages_total = n;
                    }
                }
            } else if trimmed.contains("✓ 提取成功") {
                *pages_ok += 1;
                if *current_task < task_stats.len() {
                    task_stats[*current_task].pages_ok += 1;
                }
            } else if trimmed.contains("✗ 提取失败") {
                *pages_fail += 1;
                if *current_task < task_stats.len() {
                    task_stats[*current_task].pages_fail += 1;
                }
            }
            if let Some(n_pos) = trimmed.find("RESPONSE(") {
                let rest = &trimmed[n_pos + 9..];
                if let Some(end) = rest.find(" chars") {
                    if let Ok(n) = rest[..end].parse::<usize>() {
                        *tokens_out += n;
                        if *current_task < task_stats.len() {
                            task_stats[*current_task].tokens_out += n;
                        }
                    }
                }
            }

            log_lines.push(trimmed);
            if log_lines.len() > 50 {
                log_lines.remove(0);
            }
        }
    }
}

impl Component for App {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let spinner_char = if let Phase::Researching {
            ref mut spinner_frame,
            ..
        } = self.phase
        {
            *spinner_frame = (*spinner_frame + 1) % 8;
            Some(["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"][*spinner_frame % 8])
        } else {
            None
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        // header
        let hdr: Line = match &self.phase {
            Phase::Loading => Line::from(Span::styled("── 准备 ──", GRAY)),
            Phase::PlanReview { .. } => {
                Line::from(Span::styled("── 研究计划 ──", Style::new().fg(GOLD).bold()))
            }
            Phase::Researching { iteration, .. } => {
                let s = spinner_char.unwrap_or("⣾");
                Line::from(vec![
                    Span::styled(
                        format!("── 深度研究 {}  ──  第 {} 轮  ", s, iteration),
                        GOLD,
                    ),
                    Span::styled("──", GRAY),
                ])
            }
            Phase::PdfGenerating { done, .. } => {
                let status = if *done { "完成" } else { "生成中" };
                Line::from(Span::styled(
                    format!("── PDF {} ──", status),
                    Style::new().fg(TEAL).bold(),
                ))
            }
            Phase::Error(_) => Line::from(Span::styled("── 错误 ──", Style::new().fg(RED).bold())),
        };
        frame.render_widget(Paragraph::new(hdr).alignment(Alignment::Left), chunks[0]);

        // content body
        let inner = chunks[1];

        match &self.phase {
            Phase::Loading => {
                frame.render_widget(
                    Paragraph::new("正在生成研究计划...")
                        .alignment(Alignment::Center)
                        .fg(GRAY),
                    inner,
                );
            }
            Phase::PlanReview {
                plan,
                input,
                versions,
                version_scroll,
                feedback_pending,
            } => {
                let has_history = !versions.is_empty();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(4),
                        if has_history {
                            Constraint::Min(2)
                        } else {
                            Constraint::Min(3)
                        },
                        if has_history {
                            Constraint::Length(1)
                        } else {
                            Constraint::Length(0)
                        },
                        if has_history {
                            Constraint::Min(2)
                        } else {
                            Constraint::Length(0)
                        },
                        Constraint::Length(4),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                frame.render_widget(
                    Paragraph::new(format!("{}", self.query_text)).fg(GRAY),
                    chunks[0],
                );

                let mut task_content = vec![Line::from(Span::styled("── 子任务 ──", GRAY))];
                for t in &plan.sub_tasks {
                    task_content.push(Line::from(Span::raw(format!("    ◆  {}", t.description))));
                }
                frame.render_widget(Paragraph::new(task_content).fg(GRAY), chunks[1]);

                let outline_text = if let Some(ref o) = plan.outline {
                    o.to_string()
                } else {
                    String::new()
                };
                let outline_inner = if has_history { chunks[2] } else { chunks[2] };
                frame.render_widget(
                    Paragraph::new(outline_text).wrap(Wrap { trim: false }),
                    outline_inner,
                );

                if has_history {
                    frame.render_widget(Paragraph::new("── 修订历史 ──").fg(GRAY), chunks[3]);

                    let ver_chunk = chunks[4];
                    let height = ver_chunk.height as usize;
                    let total = versions.len();
                    let max_start = total.saturating_sub(height);
                    let start = max_start.saturating_sub(*version_scroll).min(max_start);
                    let mut ver_lines: Vec<Line> = versions
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take(height)
                        .map(|(i, v)| {
                            Line::from(Span::styled(format!("第 {} 版  {}", i + 1, v), GRAY))
                        })
                        .collect();
                    if *feedback_pending && start + height >= total {
                        ver_lines.push(Line::from(Span::styled("⏳ 正在更新...", GOLD)));
                    }
                    frame.render_widget(Paragraph::new(ver_lines), ver_chunk);
                }

                let input_chunk = if has_history { chunks[5] } else { chunks[3] };
                let input_row = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Min(1),
                        Constraint::Length(2),
                    ])
                    .split(input_chunk);
                let input_bg = Color::Rgb(30, 40, 60);
                let input_block = Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::new().fg(GOLD))
                    .style(Style::new().bg(input_bg));
                let input_text = if input.value().is_empty() {
                    "\n  输入评论或修改建议，然后 Enter 执行  ".to_string()
                } else {
                    let cpos = input.cursor_char_idx();
                    let before: String = input.value().chars().take(cpos).collect();
                    let after: String = input.value().chars().skip(cpos).collect();
                    format!("\n  {}\u{258c}{}  ", before, after)
                };
                frame.render_widget(
                    Paragraph::new(input_text)
                        .style(Style::new().bg(input_bg).fg(GOLD))
                        .block(input_block),
                    input_row[1],
                );

                let enter_chunk = if has_history { chunks[7] } else { chunks[5] };
                frame.render_widget(
                    Paragraph::new("── Enter 确认执行 ──")
                        .alignment(Alignment::Center)
                        .fg(GRAY),
                    enter_chunk,
                );
            }
            Phase::Researching {
                iteration,
                max_iterations,
                quality,
                findings_count: _,
                log_lines,
                spinner_frame,
                log_scroll,
                tasks,
                task_stats,
                current_task,
                pages_ok,
                pages_fail,
                tokens_in,
                tokens_out,
                start_time,
                pages_total: _,
            } => {
                let spinner = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"][*spinner_frame % 8];

                let pct = if *max_iterations > 0 {
                    *iteration as f64 / *max_iterations as f64
                } else {
                    0.0
                };

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                let elapsed_secs = start_time.elapsed().as_secs();
                let elapsed_str = if elapsed_secs >= 3600 {
                    format!("{}h{:02}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
                } else if elapsed_secs >= 60 {
                    format!("{}m{:02}s", elapsed_secs / 60, elapsed_secs % 60)
                } else {
                    format!("{}s", elapsed_secs)
                };
                frame.render_widget(
                    Paragraph::new(format!("{}  {}  {}", spinner, self.query_text, elapsed_str))
                        .fg(GOLD),
                    chunks[0],
                );

                frame.render_widget(
                    Gauge::default()
                        .ratio(pct as f64)
                        .label(format!("{} / {}", iteration, max_iterations))
                        .use_unicode(true)
                        .gauge_style(Style::new().fg(TEAL))
                        .block(Block::default().borders(Borders::ALL)),
                    chunks[1],
                );

                // 两栏：左(35%) 任务+质量 | 右(65%) 日志
                let panes = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .split(chunks[2]);

                // 左栏：任务
                let task_lines: Vec<Line> = tasks
                    .iter()
                    .enumerate()
                    .flat_map(|(i, t)| {
                        let (icon, style) = if i < *current_task {
                            ("◆", Style::new().fg(TEAL))
                        } else if i == *current_task {
                            ("●", Style::new().fg(GOLD).bold())
                        } else {
                            ("○", Style::new().fg(GRAY))
                        };
                        let label: String = t.chars().take(20).collect();
                        let mut lines = vec![Line::from(Span::styled(
                            format!(" {}  {}", icon, label),
                            style,
                        ))];
                        if i < task_stats.len() {
                            let s = &task_stats[i];
                            let ratio = if s.pages_ok + s.pages_fail > 0 {
                                s.pages_ok as f64 / (s.pages_ok + s.pages_fail) as f64
                            } else {
                                0.0
                            };
                            let bar_len = (ratio * 6.0).round() as usize;
                            lines.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(
                                    format!(
                                        "{}{}",
                                        "▊".repeat(bar_len),
                                        "·".repeat(6usize.saturating_sub(bar_len))
                                    ),
                                    TEAL,
                                ),
                                Span::raw("  "),
                                Span::styled(format!("{}", s.pages_ok), TEAL),
                                Span::styled(format!("/{}", s.pages_ok + s.pages_fail), GRAY),
                                Span::raw("  "),
                                Span::styled(
                                    format!("✗{}", s.pages_fail),
                                    if s.pages_fail > 0 { RED } else { GRAY },
                                ),
                            ]));
                            let phase_color = match s.phase.as_str() {
                                "搜索中" | "提取中" | "综合中" => GOLD,
                                "完成" => TEAL,
                                _ => GRAY,
                            };
                            let pct_str = if s.pages_ok + s.pages_fail == 0 {
                                if s.phase == "完成" {
                                    "  —".into()
                                } else {
                                    "  0%".into()
                                }
                            } else {
                                format!("{:>3.0}%", ratio * 100.0)
                            };
                            lines.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(&s.phase, phase_color),
                                Span::raw("  "),
                                Span::styled(pct_str, GRAY),
                            ]));
                        }
                        lines
                    })
                    .collect();

                let left_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Min(1),
                        Constraint::Length(1),
                        Constraint::Length(5),
                    ])
                    .split(panes[0]);

                frame.render_widget(Paragraph::new("── 任务 ──").fg(GRAY), left_split[0]);
                frame.render_widget(Paragraph::new(task_lines), left_split[1]);

                // 左栏：质量
                fn bar8(v: f64, color: Color) -> Span<'static> {
                    let filled = (v * 8.0).round() as usize;
                    Span::styled(
                        format!(
                            "{}{}",
                            "█".repeat(filled),
                            "░".repeat(8usize.saturating_sub(filled))
                        ),
                        Style::new().fg(color),
                    )
                }
                let (cq, cr, cd, cf, co) = match quality {
                    Some(q) => (q.coverage, q.reliability, q.depth, q.freshness, q.overall),
                    None => (0.0, 0.0, 0.0, 0.0, 0.0),
                };
                let gauge_lines = vec![
                    Line::from(vec![
                        Span::styled("覆盖", GRAY),
                        Span::raw(" "),
                        Span::styled(format!("{:>3.0}%", cq * 100.0), WARM),
                        Span::raw(" "),
                        bar8(cq, TEAL),
                        Span::raw("  "),
                        Span::styled("可靠", GRAY),
                        Span::raw(" "),
                        Span::styled(format!("{:>3.0}%", cr * 100.0), WARM),
                        Span::raw(" "),
                        bar8(cr, TEAL),
                    ]),
                    Line::from(vec![
                        Span::styled("深度", GRAY),
                        Span::raw(" "),
                        Span::styled(format!("{:>3.0}%", cd * 100.0), WARM),
                        Span::raw(" "),
                        bar8(cd, STEEL),
                        Span::raw("  "),
                        Span::styled("多样", GRAY),
                        Span::raw(" "),
                        Span::styled(format!("{:>3.0}%", cf * 100.0), WARM),
                        Span::raw(" "),
                        bar8(cf, STEEL),
                    ]),
                    Line::from(vec![
                        Span::styled("总评分", GRAY),
                        Span::styled(format!("  {:>.2}", co), Style::new().fg(GOLD).bold()),
                    ]),
                ];
                frame.render_widget(Paragraph::new("── 质量 ──").fg(GRAY), left_split[2]);
                frame.render_widget(Paragraph::new(gauge_lines), left_split[3]);

                // 右栏：日志
                let right_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(panes[1]);

                let log_area = right_split[1];
                let visible = log_area.height as usize;
                let max_start = log_lines.len().saturating_sub(visible);
                let start = max_start.saturating_sub(*log_scroll).min(max_start);
                let log_text: Vec<Line> = log_lines
                    .iter()
                    .skip(start)
                    .map(|l| {
                        let d: String = l.chars().take(60).collect();
                        let color = if d.starts_with('✓') {
                            TEAL
                        } else if d.starts_with('✗') {
                            RED
                        } else if d.starts_with('→') || d.starts_with('⏭') {
                            GOLD
                        } else if d.starts_with('ℹ') {
                            STEEL
                        } else {
                            GRAY
                        };
                        Line::from(Span::styled(d, color))
                    })
                    .collect();
                frame.render_widget(Paragraph::new("── 日志 ──").fg(GRAY), right_split[0]);
                frame.render_widget(Paragraph::new(log_text), log_area);

                // 状态栏
                fn micro_bar(ratio: f64, width: usize) -> String {
                    let filled = (ratio * width as f64).round() as usize;
                    format!(
                        "{}{}",
                        "▊".repeat(filled.min(width)),
                        "·".repeat(width.saturating_sub(filled))
                    )
                }
                fn fmt_tokens(n: usize) -> String {
                    if n >= 10000 {
                        format!("{:.1}万", n as f64 / 10000.0)
                    } else {
                        n.to_string()
                    }
                }
                let pages_ratio = if *pages_ok + *pages_fail > 0 {
                    *pages_ok as f64 / (*pages_ok + *pages_fail) as f64
                } else {
                    0.0
                };
                let stat_line = Line::from(vec![
                    Span::styled(format!("成功 {}", pages_ok), TEAL),
                    Span::raw("  "),
                    Span::styled(micro_bar(pages_ratio, 10), TEAL),
                    Span::raw("  │  "),
                    Span::styled(
                        format!("失败 {}", pages_fail),
                        if *pages_fail > 0 { RED } else { GRAY },
                    ),
                    Span::raw("  │  "),
                    Span::styled(
                        format!("词元 {}", fmt_tokens(*tokens_in + *tokens_out)),
                        GRAY,
                    ),
                ]);
                frame.render_widget(Paragraph::new(stat_line), chunks[3]);
            }
            Phase::PdfGenerating {
                messages,
                done,
                ref report,
            } => {
                let mut lines: Vec<Line> = Vec::new();
                let q = &report.quality_score;
                lines.push(Line::from(vec![
                    Span::styled("质量 ", GRAY),
                    Span::styled(format!("{:.2}  ", q.overall), Style::new().fg(GOLD).bold()),
                    Span::styled("覆盖", GRAY),
                    Span::raw(format!(" {:.0}%  ", q.coverage * 100.0)),
                    Span::styled("可靠", GRAY),
                    Span::raw(format!(" {:.0}%  ", q.reliability * 100.0)),
                    Span::styled("深度", GRAY),
                    Span::raw(format!(" {:.0}%  ", q.depth * 100.0)),
                ]));
                lines.push(Line::from(Span::raw("")));
                for msg in messages.iter().rev().take(10).rev() {
                    let color = if msg.starts_with('✓') {
                        TEAL
                    } else if msg.starts_with('✗') || msg.starts_with('❌') {
                        RED
                    } else if msg.starts_with('⚠') {
                        GOLD
                    } else {
                        GRAY
                    };
                    lines.push(Line::from(Span::styled(msg.as_str(), color)));
                }
                if !done {
                    lines.push(Line::from(Span::styled("\n正在生成...", GRAY)));
                } else {
                    lines.push(Line::from(Span::styled("\n按 q 退出", GRAY)));
                }
                frame.render_widget(Paragraph::new(lines), inner);
            }
            Phase::Error(e) => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(e.as_str(), RED)))
                        .alignment(Alignment::Center),
                    inner,
                );
            }
        }
    }

    fn query<'a>(&'a self, _attr: Attribute) -> Option<QueryResult<'a>> {
        None
    }

    fn attr(&mut self, _attr: Attribute, _value: AttrValue) {}

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        CmdResult::NoChange
    }
}

impl App {
    fn handle_quit(&self) -> Option<Msg> {
        match &self.phase {
            Phase::PdfGenerating { done, .. } if *done => Some(Msg::Quit),
            Phase::PdfGenerating { .. } => None,
            _ => Some(Msg::Quit),
        }
    }
}

impl AppComponent<Msg, NoUserEvent> for App {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(k) => match k.code {
                Key::Char('q') | Key::Esc => self.handle_quit(),
                Key::Enter => {
                    if let Phase::PlanReview {
                        ref mut input,
                        ref plan,
                        ref mut feedback_pending,
                        ..
                    } = self.phase
                    {
                        if *feedback_pending {
                            return None;
                        }
                        let trimmed = input.value().trim().to_string();
                        let plan = plan.clone();
                        if trimmed.is_empty() {
                            self.fire_research(plan);
                        } else if let Some(ref tx) = self.plan_feedback_tx {
                            let _ = tx.send(trimmed);
                            *feedback_pending = true;
                            input.clear();
                        }
                    }
                    None
                }
                Key::Backspace => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.pop();
                    }
                    None
                }
                Key::Left => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.cursor_left();
                    }
                    None
                }
                Key::Right => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.cursor_right();
                    }
                    None
                }
                Key::Home | Key::CtrlHome => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.cursor_home();
                    }
                    None
                }
                Key::End | Key::CtrlEnd => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.cursor_end();
                    }
                    None
                }
                Key::Char(c) if k.modifiers == KeyModifiers::CONTROL => {
                    match c {
                        'a' | 'A' => {
                            if let Phase::PlanReview { ref mut input, .. } = self.phase {
                                input.cursor_home();
                            }
                        }
                        'e' | 'E' => {
                            if let Phase::PlanReview { ref mut input, .. } = self.phase {
                                input.cursor_end();
                            }
                        }
                        'q' | 'Q' | 'c' | 'C' => {
                            return self.handle_quit();
                        }
                        _ => {}
                    }
                    None
                }
                Key::Char(c) => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.push(c);
                    }
                    None
                }
                _ => None,
            },
            Event::Mouse(m) => {
                let is_researching = matches!(self.phase, Phase::Researching { .. });
                let is_plan_review = matches!(self.phase, Phase::PlanReview { .. });
                if is_researching {
                    if let Phase::Researching {
                        ref mut log_scroll,
                        ref log_lines,
                        ..
                    } = self.phase
                    {
                        match m.kind {
                            MouseEventKind::ScrollUp => {
                                let max_scroll = log_lines.len().saturating_sub(12);
                                *log_scroll = (*log_scroll + 1).min(max_scroll);
                            }
                            MouseEventKind::ScrollDown => {
                                *log_scroll = log_scroll.saturating_sub(1);
                            }
                            _ => {}
                        }
                    }
                } else if is_plan_review {
                    if let Phase::PlanReview {
                        ref mut version_scroll,
                        ref versions,
                        ..
                    } = self.phase
                    {
                        match m.kind {
                            MouseEventKind::ScrollUp => {
                                let max_scroll = versions.len().saturating_sub(1);
                                *version_scroll = (*version_scroll + 1).min(max_scroll);
                            }
                            MouseEventKind::ScrollDown => {
                                *version_scroll = version_scroll.saturating_sub(1);
                            }
                            _ => {}
                        }
                    }
                }
                None
            }
            Event::Tick => {
                self.drain_events();
                None
            }
            _ => None,
        }
    }
}
