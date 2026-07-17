use tuirealm::{
    command::{Cmd, CmdResult},
    component::{AppComponent, Component},
    event::{Event, Key, MouseEventKind, NoUserEvent},
    props::{AttrValue, Attribute, QueryResult},
    ratatui::{
        Frame,
        layout::{Alignment, Constraint, Direction, Layout, Rect},
        style::{Color, Style, Stylize},
        text::{Line, Span},
        widgets::{Block, Borders, Gauge, Paragraph, Wrap},
    },
    state::State,
};
use zhiyuan_core::{
    ProgressUpdate, QualityScore, ResearchPlan, ResearchQuery, ResearchReport, Uuid,
};

#[derive(Debug, Clone, Default)]
struct TaskStat {
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
}

impl InputBuf {
    fn new() -> Self {
        Self { text: String::new() }
    }
    fn push(&mut self, c: char) {
        self.text.push(c);
    }
    fn pop(&mut self) {
        self.text.pop();
    }
    fn value(&self) -> &str {
        &self.text
    }
}

enum Phase {
    Loading,
    PlanReview {
        plan: ResearchPlan,
        input: InputBuf,
    },
    Researching {
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
    Complete {
        report: ResearchReport,
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
    research_trigger: Option<tokio::sync::oneshot::Sender<(ResearchQuery, Option<ResearchPlan>)>>,
    needs_pdf: bool,
}

impl App {
    pub fn new(
        query_text: String,
        event_rx: tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
        research_trigger: tokio::sync::oneshot::Sender<(ResearchQuery, Option<ResearchPlan>)>,
    ) -> Self {
        Self {
            phase: Phase::Loading,
            event_rx,
            query_text,
            research_trigger: Some(research_trigger),
            needs_pdf: false,
        }
    }

    pub fn take_pdf_request(&mut self) -> bool {
        std::mem::replace(&mut self.needs_pdf, false)
    }

    pub fn report(&self) -> Option<&ResearchReport> {
        match &self.phase {
            Phase::Complete { report } => Some(report),
            Phase::PdfGenerating { report, .. } => Some(report),
            _ => None,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                TuiEvent::PlanReady(plan) => {
                    self.phase = Phase::PlanReview {
                        plan,
                        input: InputBuf::new(),
                    };
                }
                TuiEvent::Progress(u) => self.handle_progress(u),
                TuiEvent::LogLine(l) => self.add_log(l),
                TuiEvent::TokenUsage(prompt_tok, completion_tok) => {
                    if let Phase::Researching { ref mut tokens_in, ref mut tokens_out, .. } = self.phase {
                        *tokens_in += prompt_tok;
                        *tokens_out += completion_tok;
                    }
                }
                TuiEvent::PdfMessage(msg) => {
                    if let Phase::PdfGenerating { ref mut messages, .. } = self.phase {
                        messages.push(msg);
                    }
                }
                TuiEvent::PdfDone => {
                    if let Phase::PdfGenerating { ref mut done, ref mut messages, .. } = self.phase {
                        *done = true;
                        messages.push("PDF 生成完成".into());
                    }
                }
            }
        }
    }

    fn start_researching(&mut self, tasks: Vec<String>) {
        let stats = vec![TaskStat::default(); tasks.len().max(1)];
        self.phase = Phase::Researching {
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

    fn fire_research(&mut self, clarification: Option<String>, plan: ResearchPlan) {
        let tasks: Vec<String> = plan.sub_tasks.iter().map(|t| t.description.clone()).collect();
        let query = ResearchQuery {
            id: Uuid::new_v4(),
            query: self.query_text.clone(),
            clarification,
        };
        if let Some(trigger) = self.research_trigger.take() {
            let _ = trigger.send((query, Some(plan)));
        }
        self.start_researching(tasks);
    }

    fn handle_progress(&mut self, update: ProgressUpdate) {
        match update {
            ProgressUpdate::Started {
                max_iterations: mi, ..
            } => {
                if let Phase::Researching {
                    ref mut max_iterations,
                    ..
                } = self.phase
                {
                    *max_iterations = mi;
                } else {
                    self.start_researching(Vec::new());
                    if let Phase::Researching {
                        ref mut max_iterations,
                        ..
                    } = self.phase
                    {
                        *max_iterations = mi;
                    }
                }
            }
            ProgressUpdate::Phase { name: _, message: _ } => {
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
                self.phase = Phase::Complete { report };
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
                if let Some(n) = trimmed.rsplit('=').next().and_then(|s| s.trim().parse().ok()) {
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
                    Span::styled(format!("── 深度研究 {}  ──  第 {} 轮  ", s, iteration), GOLD),
                    Span::styled("──", GRAY),
                ])
            }
            Phase::Complete { .. } => {
                Line::from(Span::styled("── 研究完成 ──", Style::new().fg(TEAL).bold()))
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
        frame.render_widget(
            Paragraph::new(hdr).alignment(Alignment::Left),
            chunks[0],
        );

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
            Phase::PlanReview { plan, input } => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(4),
                        Constraint::Min(4),
                        Constraint::Length(3),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                frame.render_widget(
                    Paragraph::new(format!("{}", self.query_text)).fg(GRAY),
                    chunks[0],
                );

                let mut task_content = vec![
                    Line::from(Span::styled("── 子任务 ──", GRAY)),
                ];
                for t in &plan.sub_tasks {
                    task_content.push(Line::from(
                        Span::raw(format!("    ◆  {}", t.description)),
                    ));
                }
                frame.render_widget(Paragraph::new(task_content).fg(GRAY), chunks[1]);

                let outline_text = if let Some(ref o) = plan.outline {
                    o.to_string()
                } else {
                    String::new()
                };
                let outline_inner = chunks[2];
                frame.render_widget(
                    Paragraph::new(outline_text).wrap(Wrap { trim: false }),
                    outline_inner,
                );

                let input_display = if input.value().is_empty() {
                    "  输入评论或修改建议，然后 Enter 执行".to_string()
                } else {
                    format!("  > {}", input.value())
                };
                frame.render_widget(
                    Paragraph::new(input_display)
                        .style(Style::new().bg(Color::Rgb(30, 40, 60)).fg(GOLD)),
                    chunks[3],
                );

                frame.render_widget(
                    Paragraph::new("── Enter 确认执行 ──")
                        .alignment(Alignment::Center)
                        .fg(GRAY),
                    chunks[4],
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
                pages_total,
                pages_ok,
                pages_fail,
                tokens_in,
                tokens_out,
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
                        Constraint::Length(10),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                frame.render_widget(
                    Paragraph::new(format!("{}  {}", spinner, self.query_text)).fg(GOLD),
                    chunks[0],
                );

                let gauge = Gauge::default()
                    .ratio(pct as f64)
                    .label(format!("{} / {}", iteration, max_iterations))
                    .use_unicode(true)
                    .gauge_style(Style::new().fg(TEAL));
                frame.render_widget(gauge, chunks[1]);

                // 两栏分割：任务树 + 日志
                let panes = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .split(chunks[2]);

                // 左：任务树（每任务 2 行：名称 + 统计）
                fn fmt_task_stats(s: &TaskStat) -> String {
                    if s.pages_total == 0 && s.tokens_out == 0 {
                        return String::new();
                    }
                    let tok = if s.tokens_out >= 10000 {
                        format!("{:.1}万", s.tokens_out as f64 / 10000.0)
                    } else {
                        s.tokens_out.to_string()
                    };
                    format!("  {}页 ✓{} ✗{}  输出{}", s.pages_total, s.pages_ok, s.pages_fail, tok)
                }
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
                        let label: String = t.chars().take(22).collect();
                        let mut lines = vec![Line::from(Span::styled(
                            format!(" {}  {}", icon, label),
                            style,
                        ))];
                        if i < task_stats.len() {
                            let s = fmt_task_stats(&task_stats[i]);
                            if !s.is_empty() {
                                lines.push(Line::from(Span::styled(s, GRAY)));
                            }
                        }
                        lines
                    })
                    .collect();
                let task_pane = Block::default().borders(Borders::RIGHT);
                let task_area = task_pane.inner(panes[0]);
                frame.render_widget(task_pane, panes[0]);
                frame.render_widget(
                    Paragraph::new(task_lines),
                    task_area,
                );

                // 右：日志
                let visible = 16usize;
                let max_start = log_lines.len().saturating_sub(visible);
                let start = max_start.saturating_sub(*log_scroll).min(max_start);
                let log_text: Vec<Line> = log_lines
                    .iter()
                    .skip(start)
                    .map(|l| {
                        let d: String = l.chars().take(60).collect();
                        let color = if d.starts_with('✓') { TEAL }
                            else if d.starts_with('✗') { RED }
                            else if d.starts_with('→') || d.starts_with('⏭') { GOLD }
                            else if d.starts_with('ℹ') { STEEL }
                            else { GRAY };
                        Line::from(Span::styled(d, color))
                    })
                    .collect();
                frame.render_widget(Paragraph::new(log_text), panes[1]);

                fn bar(v: f64, width: usize, color: Color) -> Line<'static> {
                    let filled = (v * width as f64).round() as usize;
                    let empty = width.saturating_sub(filled);
                    let bar_str: String = format!(
                        "{}{}",
                        "█".repeat(filled),
                        "░".repeat(empty)
                    );
                    Line::from(vec![
                        Span::styled(bar_str, Style::new().fg(color)),
                    ])
                }
                let (cq, cr, cd, cf, co) = match quality {
                    Some(q) => (q.coverage, q.reliability, q.depth, q.freshness, q.overall),
                    None => (0.0, 0.0, 0.0, 0.0, 0.0),
                };
                let gauge_lines = vec![
                    Line::from(vec![
                        Span::styled("覆盖  ", GRAY),
                        Span::styled(format!("{:>5.0}%", cq * 100.0), WARM),
                    ]),
                    bar(cq, 20, TEAL),
                    Line::from(vec![
                        Span::styled("可靠  ", GRAY),
                        Span::styled(format!("{:>5.0}%", cr * 100.0), WARM),
                    ]),
                    bar(cr, 20, TEAL),
                    Line::from(vec![
                        Span::styled("深度  ", GRAY),
                        Span::styled(format!("{:>5.0}%", cd * 100.0), WARM),
                    ]),
                    bar(cd, 20, STEEL),
                    Line::from(vec![
                        Span::styled("多样  ", GRAY),
                        Span::styled(format!("{:>5.0}%", cf * 100.0), WARM),
                    ]),
                    bar(cf, 20, STEEL),
                    Line::from(vec![
                        Span::styled("总评分", GRAY),
                        Span::styled(format!("  {:>.2}", co), Style::new().fg(GOLD).bold()),
                    ]),
                ];
                frame.render_widget(
                    Paragraph::new(gauge_lines)
                        .block(Block::default()
                            .borders(Borders::ALL)
                            .title(Line::from(Span::styled("── 质量 ──", GRAY))),
                        ),
                    chunks[3],
                );

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
                let pages_ratio = if *pages_total > 0 { *pages_ok as f64 / *pages_total as f64 } else { 0.0 };
                let stat_line = Line::from(vec![
                    Span::styled(format!("网页 {}", pages_total), GRAY),
                    Span::raw("  "),
                    Span::styled(micro_bar(pages_ratio, 10), TEAL),
                    Span::raw("  │  "),
                    Span::styled(format!("成功 {}", pages_ok), TEAL),
                    Span::raw("  "),
                    Span::styled(format!("失败 {}", pages_fail), if *pages_fail > 0 { RED } else { GRAY }),
                    Span::raw("  │  "),
                    Span::styled(format!("词元 {}", fmt_tokens(*tokens_in + *tokens_out)), GRAY),
                ]);
                frame.render_widget(Paragraph::new(stat_line), chunks[4]);
            }
            Phase::Complete { report } => {
                let q = &report.quality_score;
                let mut lines: Vec<Line> = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("报告   ", GRAY),
                    Span::raw(&report.title),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("质量   ", GRAY),
                    Span::styled(format!("{:.2}  ", q.overall), Style::new().fg(GOLD).bold()),
                ]));
                lines.push(Line::from(Span::raw("")));

                fn gauge(v: f64, width: usize, color: Color) -> Line<'static> {
                    let filled = (v * width as f64).round() as usize;
                    let bar: String = format!(
                        "{}{}",
                        "█".repeat(filled),
                        "░".repeat(width.saturating_sub(filled))
                    );
                    Line::from(vec![
                        Span::styled(bar, Style::new().fg(color)),
                        Span::raw(" "),
                        Span::styled(format!("{:>3.0}%", v * 100.0), WARM),
                    ])
                }
                lines.push(Line::from(vec![
                    Span::styled("覆盖 ", GRAY),
                ]));
                lines.push(gauge(q.coverage, 24, TEAL));
                lines.push(Line::from(vec![
                    Span::styled("可靠 ", GRAY),
                ]));
                lines.push(gauge(q.reliability, 24, TEAL));
                lines.push(Line::from(vec![
                    Span::styled("深度 ", GRAY),
                ]));
                lines.push(gauge(q.depth, 24, STEEL));
                lines.push(Line::from(vec![
                    Span::styled("多样 ", GRAY),
                ]));
                lines.push(gauge(q.freshness, 24, STEEL));

                lines.push(Line::from(Span::raw("")));
                let findings_total: usize = report.sections.iter().map(|s| s.citations.len()).sum();
                lines.push(Line::from(vec![
                    Span::styled("章节", GRAY),
                    Span::raw(format!(" {}  ", report.sections.len())),
                    Span::styled("引用", GRAY),
                    Span::raw(format!(" {}", findings_total)),
                ]));
                lines.push(Line::from(Span::styled(
                    "\n研究完成，按 q 开始生成 PDF",
                    TEAL,
                )));
                frame.render_widget(Paragraph::new(lines), inner);
            }
            Phase::PdfGenerating { messages, done, .. } => {
                let mut lines: Vec<Line> = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("PDF 生成", Style::new().fg(TEAL).bold()),
                ]));
                for msg in messages.iter().rev().take(8).rev() {
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
                    lines.push(Line::from(Span::styled(
                        "\n正在生成...",
                        GRAY,
                    )));
                } else {
                    lines.push(Line::from(
                        Span::styled("\n按 q 退出", GRAY),
                    ));
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

impl AppComponent<Msg, NoUserEvent> for App {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(k) => match k.code {
                Key::Char('q') | Key::Esc => {
                    match &self.phase {
                        Phase::Complete { .. } => {
                            // 切换到 PDF 生成阶段
                            let phase = std::mem::replace(&mut self.phase, Phase::Loading);
                            if let Phase::Complete { report } = phase {
                                self.needs_pdf = true;
                                self.phase = Phase::PdfGenerating {
                                    messages: vec!["正在准备 PDF...".into()],
                                    done: false,
                                    report,
                                };
                            }
                            None
                        }
                        Phase::PdfGenerating { done, .. } if *done => Some(Msg::Quit),
                        Phase::PdfGenerating { .. } => None, // 编译中，忽略 q
                        _ => Some(Msg::Quit),
                    }
                }
                Key::Enter => {
                    if let Phase::PlanReview {
                        ref mut input,
                        ref plan,
                        ..
                    } = self.phase
                    {
                        let feedback = {
                            let trimmed = input.value().trim().to_string();
                            if trimmed.is_empty() { None } else { Some(trimmed) }
                        };
                        let plan = plan.clone();
                        self.fire_research(feedback, plan);
                    }
                    None
                }
                Key::Backspace => {
                    if let Phase::PlanReview { ref mut input, .. } = self.phase {
                        input.pop();
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
