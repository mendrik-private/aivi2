#[derive(Clone, Default)]
struct RunProgressHandle {
    state: Option<Arc<std::sync::Mutex<RunProgressState>>>,
    stop: Option<Arc<std::sync::atomic::AtomicBool>>,
}

struct RunProgressReporter {
    handle: RunProgressHandle,
    thread: Option<JoinHandle<()>>,
}

struct RunProgressState {
    title: Box<str>,
    prelaunch_current: Option<RunProgressStageState>,
    startup_current: Option<RunProgressStageState>,
    prelaunch_history: Vec<Duration>,
    startup_history: Vec<Duration>,
    recent: VecDeque<(Box<str>, Duration)>,
    rendered_lines: usize,
    frame_index: usize,
    color_enabled: bool,
}

struct RunProgressStageState {
    label: Box<str>,
    started_at: Instant,
}

impl RunProgressReporter {
    fn new(path: &Path, enabled: bool) -> Self {
        if !enabled || !io::stderr().is_terminal() {
            return Self {
                handle: RunProgressHandle::default(),
                thread: None,
            };
        }
        let color_enabled = progress_color_enabled();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let state = Arc::new(std::sync::Mutex::new(RunProgressState {
            title: compact_progress_path(path).into(),
            prelaunch_current: None,
            startup_current: None,
            prelaunch_history: Vec::new(),
            startup_history: Vec::new(),
            recent: VecDeque::new(),
            rendered_lines: 0,
            frame_index: 0,
            color_enabled,
        }));
        let thread = {
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    render_run_progress(&state);
                    thread::sleep(Duration::from_millis(120));
                }
                render_run_progress(&state);
            })
        };
        Self {
            handle: RunProgressHandle {
                state: Some(state),
                stop: Some(stop),
            },
            thread: Some(thread),
        }
    }

    fn handle(&self) -> RunProgressHandle {
        self.handle.clone()
    }
}

impl Drop for RunProgressReporter {
    fn drop(&mut self) {
        if let Some(stop) = &self.handle.stop {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl RunProgressHandle {
    fn start_prelaunch(&self, label: &'static str) {
        let Some(state) = &self.state else {
            return;
        };
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        state.prelaunch_current = Some(RunProgressStageState {
            label: label.into(),
            started_at: Instant::now(),
        });
    }

    fn finish_prelaunch(&self, label: &'static str, duration: Duration) {
        let Some(state) = &self.state else {
            return;
        };
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        state.prelaunch_current = None;
        push_progress_sample(&mut state.prelaunch_history, duration, 10);
        push_recent_progress(&mut state.recent, label, duration);
    }

    fn update_prelaunch(&self, label: impl Into<Box<str>>) {
        let Some(state) = &self.state else {
            return;
        };
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        if let Some(current) = state.prelaunch_current.as_mut() {
            current.label = label.into();
        }
    }

    fn mark_launching(&self) {
        let Some(state) = &self.state else {
            return;
        };
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        state.startup_current = Some(RunProgressStageState {
            label: "launching session".into(),
            started_at: Instant::now(),
        });
    }

    fn finish_startup_stage(
        &self,
        stage: run_session::RunStartupStage,
        startup: run_session::RunStartupMetrics,
    ) {
        let Some(state) = &self.state else {
            return;
        };
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        push_progress_sample(&mut state.startup_history, startup.stage_duration(stage), 10);
        push_recent_progress(&mut state.recent, stage.label(), startup.stage_duration(stage));
    }

    fn finish_launch(&self, startup: run_session::RunStartupMetrics) {
        let Some(state) = &self.state else {
            return;
        };
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        state.prelaunch_current = None;
        state.startup_current = Some(RunProgressStageState {
            label: "session live".into(),
            started_at: Instant::now(),
        });
        push_recent_progress(
            &mut state.recent,
            "first present",
            startup.total_to_first_present(),
        );
    }
}

fn push_progress_sample(samples: &mut Vec<Duration>, duration: Duration, max_len: usize) {
    samples.push(duration);
    if samples.len() > max_len {
        let drop_count = samples.len() - max_len;
        samples.drain(0..drop_count);
    }
}

fn push_recent_progress(recent: &mut VecDeque<(Box<str>, Duration)>, label: &str, duration: Duration) {
    recent.push_back((label.into(), duration));
    while recent.len() > 3 {
        recent.pop_front();
    }
}

fn render_run_progress(state: &Arc<std::sync::Mutex<RunProgressState>>) {
    let (lines, previous_lines) = {
        let mut state = state
            .lock()
            .expect("run progress state mutex should not be poisoned");
        let lines = build_run_progress_lines(&state);
        let previous_lines = state.rendered_lines;
        state.rendered_lines = lines.len();
        state.frame_index = state.frame_index.wrapping_add(1);
        (lines, previous_lines)
    };
    let mut stderr = io::stderr().lock();
    if previous_lines > 0 {
        let _ = write!(stderr, "\x1b[{}A", previous_lines);
    }
    for line in &lines {
        let _ = write!(stderr, "\r\x1b[2K{}\n", line);
    }
    let _ = stderr.flush();
}

fn build_run_progress_lines(state: &RunProgressState) -> Vec<String> {
    let prep_status = progress_lane_status(
        state.prelaunch_current.as_ref(),
        state.prelaunch_history.is_empty(),
    );
    let startup_status = progress_lane_status(
        state.startup_current.as_ref(),
        state.startup_history.is_empty(),
    );
    let prep_label = pad_progress_text(
        &current_progress_label(state.prelaunch_current.as_ref(), state.prelaunch_history.is_empty()),
        24,
    );
    let startup_label = pad_progress_text(
        &current_progress_label(state.startup_current.as_ref(), state.startup_history.is_empty()),
        24,
    );
    let lines = vec![
        format!(
            "{} {} {} {}",
            progress_paint_rgb(state.color_enabled, (102, 217, 239), "╭─"),
            progress_paint_rgb(state.color_enabled, (189, 147, 249), "aivi run"),
            progress_paint_dim(state.color_enabled, "•"),
            progress_paint_dim(state.color_enabled, &state.title),
        ),
        format!(
            "{} {} {}  {}  {} {}",
            progress_paint_dim(state.color_enabled, "│"),
            progress_paint_rgb(state.color_enabled, (102, 217, 239), "prep   "),
            progress_orbit(state.frame_index, prep_status, state.color_enabled),
            progress_sparkline(
                &state.prelaunch_history,
                state.color_enabled,
                (77, 208, 225),
                (124, 77, 255),
            ),
            prep_label,
            progress_paint_dim(
                state.color_enabled,
                &current_progress_elapsed(state.prelaunch_current.as_ref()),
            ),
        ),
        format!(
            "{} {} {}  {}  {} {}",
            progress_paint_dim(state.color_enabled, "│"),
            progress_paint_rgb(state.color_enabled, (255, 121, 198), "startup"),
            progress_orbit(state.frame_index.wrapping_add(2), startup_status, state.color_enabled),
            progress_sparkline(
                &state.startup_history,
                state.color_enabled,
                (255, 184, 108),
                (255, 85, 85),
            ),
            startup_label,
            progress_paint_dim(
                state.color_enabled,
                &current_progress_elapsed(state.startup_current.as_ref()),
            ),
        ),
        format!(
            "{} {}  {}",
            progress_paint_dim(state.color_enabled, "╰─"),
            progress_paint_rgb(state.color_enabled, (139, 233, 253), "recent"),
            progress_paint_dim(state.color_enabled, &recent_progress_summary(&state.recent)),
        ),
    ];
    let max_width = progress_terminal_width();
    lines
        .into_iter()
        .map(|line| truncate_ansi_line(&line, max_width))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressLaneStatus {
    Queued,
    Active,
    Complete,
}

fn progress_lane_status(current: Option<&RunProgressStageState>, empty: bool) -> ProgressLaneStatus {
    match current {
        Some(_) => ProgressLaneStatus::Active,
        None if empty => ProgressLaneStatus::Queued,
        None => ProgressLaneStatus::Complete,
    }
}

fn pad_progress_text(text: &str, width: usize) -> String {
    let char_len = text.chars().count();
    if char_len >= width {
        return text.to_owned();
    }
    let mut padded = String::with_capacity(text.len() + width - char_len);
    padded.push_str(text);
    padded.extend(std::iter::repeat_n(' ', width - char_len));
    padded
}

fn current_progress_label(current: Option<&RunProgressStageState>, empty: bool) -> String {
    match current {
        Some(stage) => format!("{}…", stage.label),
        None if empty => "queued".to_owned(),
        None => "complete".to_owned(),
    }
}

fn current_progress_elapsed(current: Option<&RunProgressStageState>) -> String {
    current
        .map(|stage| format_duration_compact(stage.started_at.elapsed()))
        .unwrap_or_default()
}

fn recent_progress_summary(recent: &VecDeque<(Box<str>, Duration)>) -> String {
    if recent.is_empty() {
        return "waiting for stage data".to_owned();
    }
    recent
        .iter()
        .map(|(label, duration)| format!("{} {}", truncate_progress_label(label, 18), format_duration_compact(*duration)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn progress_sparkline(
    samples: &[Duration],
    color_enabled: bool,
    start_color: (u8, u8, u8),
    end_color: (u8, u8, u8),
) -> String {
    const BLOCKS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const WIDTH: usize = 10;
    if samples.is_empty() {
        return progress_paint_rgb(color_enabled, (98, 114, 164), "··········");
    }
    let window = if samples.len() > WIDTH {
        &samples[samples.len() - WIDTH..]
    } else {
        samples
    };
    let max = window
        .iter()
        .copied()
        .max()
        .unwrap_or(Duration::from_millis(1))
        .max(Duration::from_millis(1));
    let mut rendered = String::new();
    for index in 0..(WIDTH - window.len()) {
        rendered.push_str(&progress_paint_rgb(
            color_enabled,
            lerp_progress_color((98, 114, 164), start_color, index, WIDTH.saturating_sub(1)),
            "·",
        ));
    }
    for (position, sample) in window.iter().enumerate() {
        let ratio = sample.as_secs_f64() / max.as_secs_f64();
        let block_index = (ratio * ((BLOCKS.len() - 1) as f64)).round() as usize;
        let glyph = BLOCKS[block_index.min(BLOCKS.len() - 1)].to_string();
        let color_index = WIDTH - window.len() + position;
        rendered.push_str(&progress_paint_rgb(
            color_enabled,
            lerp_progress_color(start_color, end_color, color_index, WIDTH.saturating_sub(1)),
            &glyph,
        ));
    }
    rendered
}

fn progress_orbit(
    frame_index: usize,
    status: ProgressLaneStatus,
    color_enabled: bool,
) -> String {
    const FRAMES: &[char] = &['⣴', '⣾', '⣶', '⣤', '⣄'];
    const ACTIVE_COLORS: &[(u8, u8, u8)] = &[
        (139, 233, 253),
        (80, 250, 123),
        (255, 184, 108),
        (255, 121, 198),
        (189, 147, 249),
    ];
    const COMPLETE_COLORS: &[(u8, u8, u8)] = &[
        (80, 250, 123),
        (98, 220, 140),
        (118, 200, 154),
        (138, 180, 168),
        (158, 160, 182),
    ];
    const QUEUED_COLORS: &[(u8, u8, u8)] = &[
        (98, 114, 164),
        (98, 114, 164),
        (98, 114, 164),
        (98, 114, 164),
        (98, 114, 164),
    ];
    let mut rendered = String::new();
    let colors = match status {
        ProgressLaneStatus::Queued => QUEUED_COLORS,
        ProgressLaneStatus::Active => ACTIVE_COLORS,
        ProgressLaneStatus::Complete => COMPLETE_COLORS,
    };
    for offset in 0..FRAMES.len() {
        let glyph = match status {
            ProgressLaneStatus::Queued => '·',
            ProgressLaneStatus::Active => FRAMES[(frame_index + offset) % FRAMES.len()],
            ProgressLaneStatus::Complete => '⣶',
        }
        .to_string();
        rendered.push_str(&progress_paint_rgb(
            color_enabled,
            colors[offset],
            &glyph,
        ));
    }
    rendered
}

fn truncate_progress_label(label: &str, max_len: usize) -> String {
    let chars = label.chars().collect::<Vec<_>>();
    if chars.len() <= max_len {
        return label.to_owned();
    }
    chars[..max_len.saturating_sub(1)]
        .iter()
        .collect::<String>()
        + "…"
}

fn compact_progress_path(path: &Path) -> String {
    let rendered = path.display().to_string();
    if rendered.chars().count() <= 56 {
        return rendered;
    }
    let tail = rendered
        .chars()
        .rev()
        .take(53)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}

fn format_duration_compact(duration: Duration) -> String {
    if duration >= Duration::from_secs(60) {
        format!("{:.1}m", duration.as_secs_f64() / 60.0)
    } else if duration >= Duration::from_secs(1) {
        format!("{:.1}s", duration.as_secs_f64())
    } else if duration >= Duration::from_millis(1) {
        format!("{:.0}ms", duration.as_secs_f64() * 1000.0)
    } else {
        format!("{:.0}µs", duration.as_secs_f64() * 1_000_000.0)
    }
}

fn progress_color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("FORCE_COLOR").is_some() {
        return true;
    }
    io::stderr().is_terminal()
}

fn progress_terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width >= 24)
        .or_else(|| {
            std::process::Command::new("tput")
                .arg("cols")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|width| *width >= 24)
        })
        .unwrap_or(72)
}

fn progress_paint_rgb(color_enabled: bool, color: (u8, u8, u8), text: &str) -> String {
    if !color_enabled {
        return text.to_owned();
    }
    format!(
        "\x1b[38;2;{};{};{}m{text}\x1b[0m",
        color.0, color.1, color.2
    )
}

fn progress_paint_dim(color_enabled: bool, text: &str) -> String {
    if !color_enabled {
        return text.to_owned();
    }
    format!("\x1b[2m{text}\x1b[0m")
}

fn lerp_progress_color(
    start: (u8, u8, u8),
    end: (u8, u8, u8),
    index: usize,
    max_index: usize,
) -> (u8, u8, u8) {
    if max_index == 0 {
        return start;
    }
    let t = index as f32 / max_index as f32;
    (
        lerp_progress_channel(start.0, end.0, t),
        lerp_progress_channel(start.1, end.1, t),
        lerp_progress_channel(start.2, end.2, t),
    )
}

fn lerp_progress_channel(start: u8, end: u8, t: f32) -> u8 {
    ((start as f32) + ((end as f32) - (start as f32)) * t).round() as u8
}

fn visible_ansi_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        width += 1;
    }
    width
}

fn truncate_ansi_line(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if visible_ansi_width(text) <= max_width {
        return text.to_owned();
    }
    if max_width == 1 {
        return "…".to_owned();
    }
    let mut rendered = String::new();
    let mut visible = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            rendered.push(ch);
            rendered.push(chars.next().expect("peeked ANSI introducer should exist"));
            for next in chars.by_ref() {
                rendered.push(next);
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        if visible >= max_width.saturating_sub(1) {
            break;
        }
        rendered.push(ch);
        visible += 1;
    }
    rendered.push('…');
    if text.contains("\x1b[") {
        rendered.push_str("\x1b[0m");
    }
    rendered
}

fn print_run_timing_report(
    path: &Path,
    load_duration: Duration,
    syntax_duration: Duration,
    hir_duration: Duration,
    query_cache: QueryCacheStats,
    artifact: RunArtifactPreparationMetrics,
    startup: run_session::RunStartupMetrics,
    total_to_first_present: Duration,
) {
    eprintln!("timings for `aivi run` ({}):", path.display());
    print_run_prelaunch_timing_details(
        load_duration,
        syntax_duration,
        hir_duration,
        query_cache,
        artifact,
    );
    eprintln!("  GTK init:                  {:>8.2?}", startup.gtk_init);
    eprintln!(
        "  runtime link:              {:>8.2?}",
        startup.runtime_link
    );
    eprintln!(
        "  session setup:             {:>8.2?}",
        startup.session_setup
    );
    eprintln!(
        "  initial runtime tick:      {:>8.2?}",
        startup.initial_runtime_tick
    );
    eprintln!(
        "  initial hydration wait:    {:>8.2?}",
        startup.initial_hydration_wait
    );
    eprintln!(
        "  root window collect:       {:>8.2?}",
        startup.root_window_collection
    );
    eprintln!(
        "  first present:             {:>8.2?}",
        startup.window_presentation
    );
    eprintln!(
        "  launch startup total:      {:>8.2?}",
        startup.total_to_first_present()
    );
    eprintln!(
        "  total (first present):     {:>8.2?}",
        total_to_first_present
    );
    flush_timing_output();
}

fn print_run_prelaunch_stage_progress(stage: &str, duration: Duration, total: Duration) {
    eprintln!(
        "pre-present stage complete: {:<24} {:>8.2?} (command total {:>8.2?})",
        stage, duration, total
    );
    flush_timing_output();
}

fn print_run_prelaunch_timing_details(
    load_duration: Duration,
    syntax_duration: Duration,
    hir_duration: Duration,
    query_cache: QueryCacheStats,
    artifact: RunArtifactPreparationMetrics,
) {
    eprintln!(
        "  source run cache load:     {:>8.2?} ({})",
        artifact.source_run_cache_load,
        if artifact.source_run_cache_hit {
            "hit"
        } else {
            "miss"
        }
    );
    eprintln!(
        "  frozen image prep:         {:>8.2?}",
        artifact.frozen_image_prepare
    );
    eprintln!(
        "  source run cache store:    {:>8.2?} (artifact write)",
        artifact.source_run_cache_store
    );
    eprintln!("  load + parse:              {:>8.2?}", load_duration);
    eprintln!("  syntax check:              {:>8.2?}", syntax_duration);
    eprintln!("  HIR lowering:              {:>8.2?}", hir_duration);
    eprintln!(
        "  workspace collect:         {:>8.2?}",
        artifact.workspace_collection
    );
    eprintln!(
        "  markup lowering:           {:>8.2?}",
        artifact.markup_lowering
    );
    eprintln!(
        "  GTK bridge lowering:       {:>8.2?}",
        artifact.widget_bridge_lowering
    );
    eprintln!(
        "  run plan validation:       {:>8.2?}",
        artifact.run_plan_validation
    );
    eprintln!(
        "  full-program lowering:     {:>8.2?}",
        artifact.runtime_backend_lowering
    );
    eprintln!(
        "  runtime assembly:          {:>8.2?}",
        artifact.runtime_assembly
    );
    eprintln!(
        "  reactive fragment compile: {:>8.2?}",
        artifact.reactive_fragment_compilation
    );
    eprintln!(
        "  runtime expr sites:        {:>8.2?}",
        artifact.markup_site_collection
    );
    eprintln!(
        "  hydration fragments:       {:>8.2?}",
        artifact.hydration_fragment_compilation
    );
    eprintln!(
        "  event handler resolve:     {:>8.2?}",
        artifact.event_handler_resolution
    );
    eprintln!(
        "  stub signal defaults:      {:>8.2?}",
        artifact.stub_signal_defaults
    );
    eprintln!("  artifact prep total:       {:>8.2?}", artifact.total);
    eprintln!(
        "  workspace modules:         {:>8}",
        artifact.workspace_module_count
    );
    eprintln!(
        "  runtime backend size:      {:>8} items, {:>4} kernels",
        artifact.runtime_backend_item_count, artifact.runtime_backend_kernel_count
    );
    eprintln!(
        "  compiled fragments:        {:>8} hydration, {:>4} reactive ({} guards, {} bodies)",
        artifact.hydration_fragment_count,
        artifact.reactive_fragment_count(),
        artifact.reactive_guard_fragment_count,
        artifact.reactive_body_fragment_count
    );
    eprintln!(
        "  query cache hot/cold:      parsed {}/{}, HIR {}/{}",
        query_cache.parsed_hits,
        query_cache.parsed_misses,
        query_cache.hir_hits,
        query_cache.hir_misses
    );
}

fn print_run_startup_stage_progress(
    stage: run_session::RunStartupStage,
    startup: run_session::RunStartupMetrics,
) {
    eprintln!(
        "  startup stage complete:    {:<24} {:>8.2?} (startup total {:>8.2?})",
        stage.label(),
        startup.stage_duration(stage),
        startup.total_to_session_ready
    );
    flush_timing_output();
}

fn flush_timing_output() {
    let _ = io::stderr().flush();
}

fn event_handler_payload_expr(module: &HirModule, handler: HirExprId) -> Option<HirExprId> {
    let ExprKind::Apply { arguments, .. } = &module.exprs()[handler].kind else {
        return None;
    };
    if arguments.len() != 1 {
        return None;
    }
    arguments.iter().next().copied()
}

fn collect_run_required_signal_globals(
    inputs: &BTreeMap<RuntimeInputHandle, CompiledRunInput>,
) -> BTreeMap<BackendItemId, Box<str>> {
    let mut required = BTreeMap::new();
    for input in inputs.values() {
        extend_run_required_signal_globals(input, &mut required);
    }
    required
}

fn extend_run_required_signal_globals(
    input: &CompiledRunInput,
    required: &mut BTreeMap<BackendItemId, Box<str>>,
) {
    match input {
        CompiledRunInput::Expr(fragment) => {
            for dependency in &fragment.required_signal_globals {
                if !matches!(dependency.kind, CompiledRunGlobalKind::Signal) {
                    continue;
                }
                required
                    .entry(dependency.runtime_item)
                    .or_insert_with(|| dependency.name.clone());
            }
        }
        CompiledRunInput::Text(text) => {
            for segment in &text.segments {
                let CompiledRunTextSegment::Interpolation(fragment) = segment else {
                    continue;
                };
                for dependency in &fragment.required_signal_globals {
                    if !matches!(dependency.kind, CompiledRunGlobalKind::Signal) {
                        continue;
                    }
                    required
                        .entry(dependency.runtime_item)
                        .or_insert_with(|| dependency.name.clone());
                }
            }
        }
    }
}

fn run_hydration_globals_ready(
    required: &BTreeMap<BackendItemId, Box<str>>,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> bool {
    required.keys().all(|item| globals.contains_key(item))
}

/// For each workspace Signal import in the assembly's stub Input signals, compute
/// a type-based default runtime value to pre-seed the signal before the first
/// hydration cycle. This prevents hydration from blocking on cross-module signals
/// that have no daemon publisher.
fn collect_stub_signal_defaults(
    module: &HirModule,
    assembly: &HirRuntimeAssembly,
) -> Vec<(RuntimeInputHandle, DetachedRuntimeValue)> {
    let hir_item_count =
        u32::try_from(module.items().iter().count()).expect("HIR item count fits u32");
    let mut defaults = Vec::new();
    for signal_binding in assembly.signals() {
        let raw = signal_binding.item.as_raw();
        if raw < hir_item_count {
            continue; // Real HIR item, not a stub.
        }
        let import_id = ImportId::from_raw(raw - hir_item_count);
        let Some(import_binding) = module.imports().get(import_id) else {
            continue;
        };
        let ImportBindingMetadata::Value {
            ty: ImportValueType::Signal(inner_ty),
        } = &import_binding.metadata
        else {
            continue;
        };
        let Some(input) = signal_binding.input() else {
            continue;
        };
        let Some(default_value) = default_runtime_value_for_import_type(inner_ty) else {
            continue;
        };
        let default_value = DetachedRuntimeValue::from_runtime_owned(default_value);
        defaults.push((input, default_value));
    }
    defaults
}

fn default_runtime_value_for_import_type(ty: &ImportValueType) -> Option<RuntimeValue> {
    match ty {
        ImportValueType::Primitive(builtin) => match builtin {
            BuiltinType::Text => Some(RuntimeValue::Text("".into())),
            BuiltinType::Int => Some(RuntimeValue::Int(0)),
            BuiltinType::Bool => Some(RuntimeValue::Bool(false)),
            BuiltinType::Float => Some(RuntimeValue::Float(
                RuntimeFloat::new(0.0_f64).expect("0.0 is a valid float"),
            )),
            BuiltinType::Unit => Some(RuntimeValue::Unit),
            _ => None,
        },
        ImportValueType::List(_) => Some(RuntimeValue::List(vec![])),
        ImportValueType::Set(_) => Some(RuntimeValue::Set(vec![])),
        ImportValueType::Map { .. } => Some(RuntimeValue::Map(Default::default())),
        ImportValueType::Option(_) => Some(RuntimeValue::OptionNone),
        ImportValueType::Result { error, .. } => default_runtime_value_for_import_type(error)
            .map(|error| RuntimeValue::ResultErr(Box::new(error))),
        ImportValueType::Validation { error, .. } => default_runtime_value_for_import_type(error)
            .map(|error| RuntimeValue::ValidationInvalid(Box::new(error))),
        ImportValueType::Tuple(elements) => elements
            .iter()
            .map(default_runtime_value_for_import_type)
            .collect::<Option<Vec<_>>>()
            .map(RuntimeValue::Tuple),
        ImportValueType::Record(fields) => fields
            .iter()
            .map(|field| {
                Some(RuntimeRecordField {
                    label: field.name.clone(),
                    value: default_runtime_value_for_import_type(&field.ty)?,
                })
            })
            .collect::<Option<Vec<_>>>()
            .map(RuntimeValue::Record),
        ImportValueType::Signal(inner) => default_runtime_value_for_import_type(inner)
            .map(|inner| RuntimeValue::Signal(Box::new(inner))),
        // Functions, tasks, and named/variable types cannot be safely defaulted.
        ImportValueType::Arrow { .. }
        | ImportValueType::Task { .. }
        | ImportValueType::TypeVariable { .. }
        | ImportValueType::Named { .. } => None,
    }
}

#[derive(Clone)]
struct CompiledRuntimeFragmentUnit {
    core: Arc<aivi_core::LoweredRuntimeFragment>,
    backend: Arc<BackendProgram>,
}

struct RunFragmentCompiler<'a> {
    sources: &'a SourceDatabase,
    module: &'a HirModule,
    workspace_hirs: &'a [(&'a str, &'a HirModule)],
    view_owner: aivi_hir::ItemId,
    sites: &'a aivi_hir::MarkupRuntimeExprSites,
    runtime_backend: &'a BackendProgram,
    runtime_backend_by_hir: &'a BTreeMap<aivi_hir::ItemId, BackendItemId>,
    query_context: Option<BackendQueryContext<'a>>,
    compiled_fragments: BTreeMap<HirExprId, CompiledRunFragment>,
}

impl<'a> RunFragmentCompiler<'a> {
    fn new(
        sources: &'a SourceDatabase,
        module: &'a HirModule,
        workspace_hirs: &'a [(&'a str, &'a HirModule)],
        view_owner: aivi_hir::ItemId,
        sites: &'a aivi_hir::MarkupRuntimeExprSites,
        runtime_backend: &'a BackendProgram,
        runtime_backend_by_hir: &'a BTreeMap<aivi_hir::ItemId, BackendItemId>,
        query_context: Option<BackendQueryContext<'a>>,
    ) -> Self {
        Self {
            sources,
            module,
            workspace_hirs,
            view_owner,
            sites,
            runtime_backend,
            runtime_backend_by_hir,
            query_context,
            compiled_fragments: BTreeMap::new(),
        }
    }

    fn compile(&mut self, expr: HirExprId) -> Result<(CompiledRunFragment, bool), String> {
        if let Some(cached) = self.compiled_fragments.get(&expr) {
            return Ok((cached.clone(), false));
        }

        let compiled = self.compile_uncached(expr)?;
        self.compiled_fragments.insert(expr, compiled.clone());
        Ok((compiled, true))
    }

    fn compile_uncached(&mut self, expr: HirExprId) -> Result<CompiledRunFragment, String> {
        compile_run_fragment_for_input(
            self.module,
            self.sources,
            self.workspace_hirs,
            self.view_owner,
            self.sites,
            self.runtime_backend,
            self.runtime_backend_by_hir,
            self.query_context,
            expr,
        )
    }
}

fn compile_run_fragment_for_input(
    module: &HirModule,
    sources: &SourceDatabase,
    workspace_hirs: &[(&str, &HirModule)],
    view_owner: aivi_hir::ItemId,
    sites: &aivi_hir::MarkupRuntimeExprSites,
    runtime_backend: &BackendProgram,
    runtime_backend_by_hir: &BTreeMap<aivi_hir::ItemId, BackendItemId>,
    query_context: Option<BackendQueryContext<'_>>,
    expr: HirExprId,
) -> Result<CompiledRunFragment, String> {
    let site = sites.get(expr).ok_or_else(|| {
        format!(
            "run view references expression {} at {} without a collected runtime environment",
            expr.as_raw(),
            source_location(sources, module.exprs()[expr].span)
        )
    })?;
    let body = elaborate_runtime_expr_with_env(module, expr, &site.parameters, Some(&site.ty))
        .map_err(|blocked| {
            format!(
                "failed to elaborate runtime expression at {}: {}",
                source_location(sources, site.span),
                blocked
            )
        })?;
    let fragment = RuntimeFragmentSpec {
        name: format!("__run_fragment_{}", expr.as_raw()).into_boxed_str(),
        owner: view_owner,
        body_expr: expr,
        parameters: site.parameters.clone(),
        body,
    };
    let unit = compile_runtime_fragment_backend_unit(
        module,
        workspace_hirs,
        &fragment,
        query_context,
        &format!(
            "failed to compile runtime expression at {}",
            source_location(sources, site.span)
        ),
    )?;
    let execution = Arc::new(RunFragmentExecutionUnit::new(
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(unit.backend.clone()),
        Arc::new(aivi_backend::NativeKernelArtifactSet::default()),
    ));
    let backend = unit.backend.as_ref();
    let item = backend
        .items()
        .iter()
        .find_map(|(item_id, item)| (item.name == unit.core.entry_name).then_some(item_id))
        .ok_or_else(|| {
            format!(
                "backend lowering did not preserve runtime fragment `{}` for expression at {}",
                unit.core.entry_name,
                source_location(sources, site.span)
            )
        })?;
    let required_signal_globals = collect_fragment_signal_global_items_for_run(
        runtime_backend,
        runtime_backend_by_hir,
        module,
        &unit,
        backend,
        item,
        expr,
    )?;
    Ok(CompiledRunFragment {
        expr,
        parameters: runtime_fragment_parameters(&site.parameters),
        execution,
        item,
        required_signal_globals,
    })
}

fn collect_fragment_signal_global_items_for_run(
    runtime_backend: &BackendProgram,
    runtime_backend_by_hir: &BTreeMap<aivi_hir::ItemId, BackendItemId>,
    module: &HirModule,
    unit: &CompiledRuntimeFragmentUnit,
    backend: &BackendProgram,
    entry_item: BackendItemId,
    expr: HirExprId,
) -> Result<Vec<CompiledRunSignalGlobal>, String> {
    let mut required = BTreeSet::new();
    let mut visited_items = BTreeSet::new();
    let mut kernels = backend.items()[entry_item]
        .body
        .into_iter()
        .collect::<Vec<_>>();
    while let Some(kernel_id) = kernels.pop() {
        let kernel = &backend.kernels()[kernel_id];
        for &fragment_item in &kernel.global_items {
            if !visited_items.insert(fragment_item) {
                continue;
            }
            let decl = backend.items().get(fragment_item).ok_or_else(|| {
                format!(
                    "compiled runtime fragment {} references missing backend item {}",
                    expr.as_raw(),
                    fragment_item
                )
            })?;
            match decl.kind {
                aivi_backend::ItemKind::Signal(_) => {
                    required.insert(fragment_item);
                }
                _ => {
                    if let Some(body) = decl.body {
                        kernels.push(body);
                    } else {
                        required.insert(fragment_item);
                    }
                }
            }
        }
    }
    required
        .into_iter()
        .map(|fragment_item| {
            link_fragment_signal_global_for_run(
                runtime_backend,
                runtime_backend_by_hir,
                module,
                unit,
                backend,
                expr,
                fragment_item,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|globals| globals.into_iter().flatten().collect())
}

fn link_fragment_signal_global_for_run(
    runtime_backend: &BackendProgram,
    runtime_backend_by_hir: &BTreeMap<aivi_hir::ItemId, BackendItemId>,
    module: &HirModule,
    unit: &CompiledRuntimeFragmentUnit,
    backend: &BackendProgram,
    expr: HirExprId,
    fragment_item: BackendItemId,
) -> Result<Option<CompiledRunSignalGlobal>, String> {
    let fragment_decl = backend.items().get(fragment_item).ok_or_else(|| {
        format!(
            "compiled runtime fragment {} references missing backend item {}",
            expr.as_raw(),
            fragment_item
        )
    })?;
    let core_item = unit
        .core
        .module
        .items()
        .get(fragment_decl.origin)
        .ok_or_else(|| {
            format!(
                "compiled runtime fragment {} lost core→HIR origin for backend item {}",
                expr.as_raw(),
                fragment_item
            )
        })?;
    let hir_item = core_item.origin;
    let hir_lookup = module.items().get(hir_item);
    let signal_name: Box<str> = match hir_lookup {
        Some(Item::Signal(signal)) => signal.name.text().into(),
        Some(_) => return Ok(None),
        None => core_item.name.clone(),
    };
    let runtime_item = if hir_lookup.is_some() {
        runtime_backend_by_hir
            .get(&hir_item)
            .copied()
            .ok_or_else(|| {
                format!(
                    "runtime fragment {} needs signal `{signal_name}` but the live run backend has no matching item",
                    expr.as_raw(),
                )
            })?
    } else {
        runtime_backend
            .items()
            .iter()
            .find_map(|(backend_item, item)| {
                (item.name.as_ref() == signal_name.as_ref()).then_some(backend_item)
            })
            .ok_or_else(|| {
                format!(
                    "runtime fragment {} needs global `{signal_name}` (synthetic origin) but no matching runtime item found",
                    expr.as_raw(),
                )
            })?
    };
    let runtime_decl = runtime_backend.items().get(runtime_item).ok_or_else(|| {
        format!(
            "live run backend is missing runtime item {} for signal `{signal_name}`",
            runtime_item,
        )
    })?;
    let kind = if matches!(runtime_decl.kind, aivi_backend::ItemKind::Signal(_)) {
        CompiledRunGlobalKind::Signal
    } else if runtime_decl.body.is_some() {
        CompiledRunGlobalKind::RuntimeItem
    } else {
        return Err(format!(
            "compiled runtime fragment {} references global item {} ({}) without a body kernel or live signal binding",
            expr.as_raw(),
            fragment_item,
            signal_name,
        ));
    };
    Ok(Some(CompiledRunSignalGlobal {
        fragment_item,
        runtime_item,
        name: signal_name,
        kind,
    }))
}

fn compile_runtime_fragment_backend_unit(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    fragment: &RuntimeFragmentSpec,
    query_context: Option<BackendQueryContext<'_>>,
    error_context: &str,
) -> Result<CompiledRuntimeFragmentUnit, String> {
    if let Some(query_context) = query_context {
        return runtime_fragment_backend_unit(query_context.db, query_context.entry, fragment)
            .map(|unit| CompiledRuntimeFragmentUnit {
                core: unit.core_arc(),
                backend: unit.backend_arc(),
            })
            .map_err(|error| format!("{error_context}: {error}"));
    }

    compile_local_runtime_fragment_backend_unit(module, workspace_hirs, fragment, error_context)
}

fn compile_local_runtime_fragment_backend_unit(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    fragment: &RuntimeFragmentSpec,
    error_context: &str,
) -> Result<CompiledRuntimeFragmentUnit, String> {
    let core = if workspace_hirs.is_empty() {
        lower_runtime_fragment(module, fragment)
            .map_err(|error| format!("{error_context} into typed core: {error}"))?
    } else {
        aivi_core::lower_runtime_fragment_with_workspace(module, workspace_hirs, fragment)
            .map_err(|error| format!("{error_context} into typed core: {error}"))?
    };
    let lambda = lower_lambda_module(&core.module)
        .map_err(|error| format!("{error_context} into typed lambda: {error}"))?;
    let backend = lower_backend_module(&lambda, module)
        .map_err(|error| format!("{error_context} into backend IR: {error}"))?;
    Ok(CompiledRuntimeFragmentUnit {
        core: Arc::new(core),
        backend: Arc::new(backend),
    })
}

fn runtime_fragment_parameters(parameters: &[GeneralExprParameter]) -> Vec<RunFragmentParameter> {
    parameters
        .iter()
        .map(|parameter| RunFragmentParameter {
            binding: parameter.binding,
            name: parameter.name.clone(),
        })
        .collect()
}

type RuntimeBindingEnv = BTreeMap<aivi_hir::BindingId, RuntimeValue>;
type EvaluatorCache<'a> = BTreeMap<usize, BackendExecutionEngineHandle<'a>>;

fn plan_run_hydration(
    shared: &RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> Result<RunHydrationPlan, String> {
    let mut profiler = RunHydrationProfiler::disabled();
    plan_run_hydration_with_profiler(shared, globals, &mut profiler)
}

#[cfg_attr(not(test), allow(dead_code))]
fn plan_run_hydration_profiled(
    shared: &RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> Result<(RunHydrationPlan, RunHydrationProfile), String> {
    let mut profiler = RunHydrationProfiler::enabled();
    let plan = plan_run_hydration_with_profiler(shared, globals, &mut profiler)?;
    let profile = profiler
        .into_profile()
        .expect("enabled hydration profiler should produce a profile");
    Ok((plan, profile))
}

fn plan_run_hydration_with_profiler(
    shared: &RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
    profiler: &mut RunHydrationProfiler,
) -> Result<RunHydrationPlan, String> {
    let started_at = Instant::now();
    let runtime_globals = runtime_globals_from_detached(globals);
    let mut evaluators = EvaluatorCache::new();
    let plan = RunHydrationPlan {
        root: plan_run_node(
            shared,
            &runtime_globals,
            &GtkNodeInstance::root(shared.bridge.root()),
            &RuntimeBindingEnv::new(),
            &mut evaluators,
            profiler,
        )?,
    };
    profiler.finish(started_at.elapsed(), &evaluators);
    Ok(plan)
}

fn runtime_globals_from_detached(
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> BTreeMap<BackendItemId, RuntimeValue> {
    globals
        .iter()
        .map(|(&item, value)| (item, value.to_runtime()))
        .collect()
}

fn plan_run_node<'a>(
    shared: &'a RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    instance: &GtkNodeInstance,
    env: &RuntimeBindingEnv,
    evaluators: &mut EvaluatorCache<'a>,
    profiler: &mut RunHydrationProfiler,
) -> Result<HydratedRunNode, String> {
    profiler.record_node();
    let view_name = shared.view_name.as_ref();
    let node = shared.bridge.node(instance.node.plan).ok_or_else(|| {
        format!(
            "run view `{view_name}` is missing GTK node {}",
            instance.node
        )
    })?;
    match &node.kind {
        GtkBridgeNodeKind::Widget(widget) => {
            let mut properties = Vec::new();
            for property in &widget.properties {
                if let RuntimePropertyBinding::Setter(setter) = property {
                    properties.push(HydratedRunProperty {
                        input: setter.input,
                        value: DetachedRuntimeValue::from_runtime_owned(evaluate_run_input(
                            shared,
                            globals,
                            setter.input,
                            env,
                            evaluators,
                            profiler,
                        )?),
                    });
                }
            }
            let mut event_inputs = Vec::new();
            for event in &widget.event_hooks {
                if !shared.inputs.contains_key(&event.input) {
                    continue;
                }
                event_inputs.push(HydratedRunProperty {
                    input: event.input,
                    value: DetachedRuntimeValue::from_runtime_owned(evaluate_run_input(
                        shared,
                        globals,
                        event.input,
                        env,
                        evaluators,
                        profiler,
                    )?),
                });
            }
            Ok(HydratedRunNode::Widget {
                instance: instance.clone(),
                properties: properties.into_boxed_slice(),
                event_inputs: event_inputs.into_boxed_slice(),
                children: plan_run_child_group(
                    shared,
                    globals,
                    &widget.default_children.roots,
                    instance.path.clone(),
                    env,
                    evaluators,
                    profiler,
                )?,
            })
        }
        GtkBridgeNodeKind::Group(group) => Ok(HydratedRunNode::Fragment {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &group.body.roots,
                instance.path.clone(),
                env,
                evaluators,
                profiler,
            )?,
        }),
        GtkBridgeNodeKind::Show(show) => {
            let when = runtime_truthy_bool(evaluate_run_input(
                shared,
                globals,
                show.when.input,
                env,
                evaluators,
                profiler,
            )?)
            .ok_or_else(|| {
                format!(
                    "run view `{view_name}` expected `<show when>` on {instance} to evaluate to Bool or a canonical truthy/falsy carrier"
                )
            })?;
            let (keep_mounted_input, keep_mounted) = match &show.mount {
                RuntimeShowMountPolicy::UnmountWhenHidden => (None, false),
                RuntimeShowMountPolicy::KeepMounted { decision } => (
                    Some(decision.input),
                    runtime_bool(evaluate_run_input(
                        shared,
                        globals,
                        decision.input,
                        env,
                        evaluators,
                        profiler,
                    )?)
                    .ok_or_else(|| {
                        format!(
                            "run view `{view_name}` expected `<show keepMounted>` on {instance} to evaluate to Bool"
                        )
                    })?,
                ),
            };
            let children = if when || keep_mounted {
                plan_run_child_group(
                    shared,
                    globals,
                    &show.body.roots,
                    instance.path.clone(),
                    env,
                    evaluators,
                    profiler,
                )?
            } else {
                Vec::new().into_boxed_slice()
            };
            Ok(HydratedRunNode::Show {
                instance: instance.clone(),
                when_input: show.when.input,
                when,
                keep_mounted_input,
                keep_mounted,
                children,
            })
        }
        GtkBridgeNodeKind::Each(each) => {
            let values = runtime_list_values(evaluate_run_input(
                shared,
                globals,
                each.collection.input,
                env,
                evaluators,
                profiler,
            )?)
            .ok_or_else(|| {
                format!(
                    "run view `{view_name}` expected `<each>` collection on {instance} to evaluate to a List"
                )
            })?;
            let collection_is_empty = values.is_empty();
            let kind = match &each.child_policy {
                RepeatedChildPolicy::Positional { .. } => {
                    let mut items = Vec::with_capacity(values.len());
                    for (index, value) in values.into_iter().enumerate() {
                        let mut child_env = env.clone();
                        child_env.insert(each.binding, value);
                        let path = instance.path.pushed(
                            instance.node,
                            aivi_gtk::GtkRepeatedChildIdentity::Positional(index),
                        );
                        items.push(HydratedRunEachItem {
                            children: plan_run_child_group(
                                shared,
                                globals,
                                &each.item_template.roots,
                                path,
                                &child_env,
                                evaluators,
                                profiler,
                            )?,
                        });
                    }
                    HydratedRunEachKind::Positional {
                        item_count: items.len(),
                        items: items.into_boxed_slice(),
                    }
                }
                RepeatedChildPolicy::Keyed { .. } => {
                    let key_input = each.key_input.as_ref().ok_or_else(|| {
                        format!(
                            "run view `{view_name}` is missing a keyed `<each>` runtime input on {instance}"
                        )
                    })?;
                    let mut items = Vec::with_capacity(values.len());
                    let mut keys = Vec::with_capacity(values.len());
                    for value in values {
                        let mut child_env = env.clone();
                        child_env.insert(each.binding, value);
                        let collection_key = runtime_collection_key(evaluate_run_input(
                            shared,
                            globals,
                            key_input.input,
                            &child_env,
                            evaluators,
                            profiler,
                        )?)
                        .ok_or_else(|| {
                            format!(
                                "run view `{view_name}` expected `<each>` key on {instance} to be displayable"
                            )
                        })?;
                        let path = instance.path.pushed(
                            instance.node,
                            aivi_gtk::GtkRepeatedChildIdentity::Keyed(collection_key.clone()),
                        );
                        keys.push(collection_key);
                        items.push(HydratedRunEachItem {
                            children: plan_run_child_group(
                                shared,
                                globals,
                                &each.item_template.roots,
                                path,
                                &child_env,
                                evaluators,
                                profiler,
                            )?,
                        });
                    }
                    HydratedRunEachKind::Keyed {
                        key_input: key_input.input,
                        keys: keys.into_boxed_slice(),
                        items: items.into_boxed_slice(),
                    }
                }
            };
            let empty_branch = if collection_is_empty {
                each.empty_branch
                    .as_ref()
                    .map(|empty| {
                        plan_run_node(
                            shared,
                            globals,
                            &GtkNodeInstance::with_path(empty.empty, instance.path.clone()),
                            env,
                            evaluators,
                            profiler,
                        )
                    })
                    .transpose()?
                    .map(Box::new)
            } else {
                None
            };
            Ok(HydratedRunNode::Each {
                instance: instance.clone(),
                collection_input: each.collection.input,
                kind,
                empty_branch,
            })
        }
        GtkBridgeNodeKind::Match(match_node) => {
            let value = evaluate_run_input(
                shared,
                globals,
                match_node.scrutinee.input,
                env,
                evaluators,
                profiler,
            )?;
            let mut matched = None;
            for (index, branch) in match_node.cases.iter().enumerate() {
                let mut bindings = RuntimeBindingEnv::new();
                if match_pattern(&shared.patterns, branch.pattern, &value, &mut bindings)? {
                    matched = Some((index, branch, bindings));
                    break;
                }
            }
            let Some((active_case, branch, bindings)) = matched else {
                return Err(format!(
                    "run view `{view_name}` found no matching `<match>` case for node {instance}"
                ));
            };
            let mut case_env = env.clone();
            case_env.extend(bindings);
            Ok(HydratedRunNode::Match {
                instance: instance.clone(),
                scrutinee_input: match_node.scrutinee.input,
                active_case,
                branch: Box::new(plan_run_node(
                    shared,
                    globals,
                    &GtkNodeInstance::with_path(branch.case, instance.path.clone()),
                    &case_env,
                    evaluators,
                    profiler,
                )?),
            })
        }
        GtkBridgeNodeKind::Case(case) => Ok(HydratedRunNode::Case {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &case.body.roots,
                instance.path.clone(),
                env,
                evaluators,
                profiler,
            )?,
        }),
        GtkBridgeNodeKind::Fragment(fragment) => Ok(HydratedRunNode::Fragment {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &fragment.body.roots,
                instance.path.clone(),
                env,
                evaluators,
                profiler,
            )?,
        }),
        GtkBridgeNodeKind::With(with_node) => {
            let value = evaluate_run_input(
                shared,
                globals,
                with_node.value.input,
                env,
                evaluators,
                profiler,
            )?;
            let mut child_env = env.clone();
            child_env.insert(with_node.binding, strip_signal_runtime_value(value));
            Ok(HydratedRunNode::With {
                instance: instance.clone(),
                value_input: with_node.value.input,
                children: plan_run_child_group(
                    shared,
                    globals,
                    &with_node.body.roots,
                    instance.path.clone(),
                    &child_env,
                    evaluators,
                    profiler,
                )?,
            })
        }
        GtkBridgeNodeKind::Empty(empty) => Ok(HydratedRunNode::Empty {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &empty.body.roots,
                instance.path.clone(),
                env,
                evaluators,
                profiler,
            )?,
        }),
    }
}

fn plan_run_child_group<'a>(
    shared: &'a RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    roots: &[aivi_gtk::GtkBridgeNodeRef],
    path: GtkExecutionPath,
    env: &RuntimeBindingEnv,
    evaluators: &mut EvaluatorCache<'a>,
    profiler: &mut RunHydrationProfiler,
) -> Result<Box<[HydratedRunNode]>, String> {
    let mut children = Vec::with_capacity(roots.len());
    for &root in roots {
        children.push(plan_run_node(
            shared,
            globals,
            &GtkNodeInstance::with_path(root, path.clone()),
            env,
            evaluators,
            profiler,
        )?);
    }
    Ok(children.into_boxed_slice())
}

fn apply_run_hydration_plan(
    plan: &RunHydrationPlan,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    apply_run_node(&plan.root, executor)
}

fn apply_run_children(
    children: &[HydratedRunNode],
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    for child in children {
        apply_run_node(child, executor)?;
    }
    Ok(())
}

fn apply_run_node(
    node: &HydratedRunNode,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    match node {
        HydratedRunNode::Widget {
            instance,
            properties,
            event_inputs,
            children,
        } => {
            for property in properties {
                executor
                    .set_input_for_instance(
                        instance,
                        property.input,
                        RunHostValue(property.value.clone()),
                    )
                    .map_err(|error| {
                        format!(
                            "failed to apply dynamic input {} on {}: {error}",
                            property.input.as_raw(),
                            instance
                        )
                    })?;
            }
            for event_input in event_inputs {
                executor
                    .set_input_for_instance(
                        instance,
                        event_input.input,
                        RunHostValue(event_input.value.clone()),
                    )
                    .map_err(|error| {
                        format!(
                            "failed to apply event input {} on {}: {error}",
                            event_input.input.as_raw(),
                            instance
                        )
                    })?;
            }
            apply_run_children(children, executor)
        }
        HydratedRunNode::Show {
            instance,
            when,
            keep_mounted,
            children,
            ..
        } => {
            executor
                .update_show(instance, *when, *keep_mounted)
                .map_err(|error| format!("failed to update `<show>` node {instance}: {error}"))?;
            apply_run_children(children, executor)
        }
        HydratedRunNode::Each {
            instance,
            kind,
            empty_branch,
            ..
        } => {
            match kind {
                HydratedRunEachKind::Positional { item_count, items } => {
                    executor
                        .update_each_positional(instance, *item_count)
                        .map_err(|error| {
                            format!("failed to update positional `<each>` node {instance}: {error}")
                        })?;
                    for item in items {
                        apply_run_children(&item.children, executor)?;
                    }
                }
                HydratedRunEachKind::Keyed { keys, items, .. } => {
                    executor
                        .update_each_keyed(instance, keys)
                        .map_err(|error| {
                            format!("failed to update keyed `<each>` node {instance}: {error}")
                        })?;
                    for item in items {
                        apply_run_children(&item.children, executor)?;
                    }
                }
            }
            if let Some(empty_branch) = empty_branch {
                apply_run_node(empty_branch, executor)?;
            }
            Ok(())
        }
        HydratedRunNode::Match {
            instance,
            active_case,
            branch,
            ..
        } => {
            executor
                .update_match(instance, *active_case)
                .map_err(|error| format!("failed to update `<match>` node {instance}: {error}"))?;
            apply_run_node(branch, executor)
        }
        HydratedRunNode::Case { children, .. }
        | HydratedRunNode::Fragment { children, .. }
        | HydratedRunNode::With { children, .. }
        | HydratedRunNode::Empty { children, .. } => apply_run_children(children, executor),
    }
}

fn evaluate_run_input<'a>(
    shared: &'a RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    input: RuntimeInputHandle,
    env: &RuntimeBindingEnv,
    evaluators: &mut EvaluatorCache<'a>,
    profiler: &mut RunHydrationProfiler,
) -> Result<RuntimeValue, String> {
    profiler.record_input();
    let compiled = shared.inputs.get(&input).ok_or_else(|| {
        format!(
            "missing compiled runtime input {} for live run hydration",
            input.as_raw()
        )
    })?;
    match compiled {
        CompiledRunInput::Expr(fragment) => {
            evaluate_compiled_run_fragment(shared, fragment, globals, env, evaluators, profiler)
        }
        CompiledRunInput::Text(text) => {
            evaluate_compiled_run_text(shared, text, globals, env, evaluators, profiler)
        }
    }
}

fn evaluate_compiled_run_fragment<'a>(
    shared: &'a RunHydrationStaticState,
    fragment: &'a CompiledRunFragment,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    env: &RuntimeBindingEnv,
    evaluators: &mut EvaluatorCache<'a>,
    profiler: &mut RunHydrationProfiler,
) -> Result<RuntimeValue, String> {
    let started_at = profiler.kernel_profiling_enabled().then(Instant::now);
    let args = fragment
        .parameters
        .iter()
        .map(|parameter| {
            env.get(&parameter.binding).cloned().ok_or_else(|| {
                format!(
                    "missing runtime value for binding `{}` while evaluating expression {}",
                    parameter.name,
                    fragment.expr.as_raw()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let execution_key = Arc::as_ptr(&fragment.execution) as usize;
    let kernel_profiling_enabled = profiler.kernel_profiling_enabled();
    let runtime_execution_key = Arc::as_ptr(&shared.runtime_execution) as usize;
    let mut required_globals = BTreeMap::new();
    for dep in &fragment.required_signal_globals {
        let value = match dep.kind {
            CompiledRunGlobalKind::Signal => globals.get(&dep.runtime_item).cloned().ok_or_else(|| {
                format!(
                    "runtime expression {} requires current signal `{}` (runtime item {}) but no committed snapshot exists",
                    fragment.expr.as_raw(),
                    dep.name,
                    dep.runtime_item
                )
            })?,
            CompiledRunGlobalKind::RuntimeItem => {
                let runtime_evaluator = evaluators
                    .entry(runtime_execution_key)
                    .or_insert_with(|| shared.runtime_execution.create_engine(kernel_profiling_enabled));
                runtime_evaluator
                    .evaluate_item(dep.runtime_item, globals)
                    .map_err(|error| {
                        format!(
                            "runtime expression {} could not evaluate global `{}` (runtime item {}): {error}",
                            fragment.expr.as_raw(),
                            dep.name,
                            dep.runtime_item
                        )
                    })?
            }
        };
        required_globals.insert(dep.fragment_item, value);
    }
    let evaluator = evaluators
        .entry(execution_key)
        .or_insert_with(|| fragment.execution.create_engine(kernel_profiling_enabled));
    let item = fragment
        .execution
        .backend_view()
        .item(fragment.item)
        .ok_or_else(|| {
            format!(
                "compiled runtime fragment {} references missing backend item {}",
                fragment.expr.as_raw(),
                fragment.item.as_raw()
            )
        })?;
    let result = if let Some(kernel) = item.body {
        evaluator
            .evaluate_kernel(kernel, None, &args, &required_globals)
            .map_err(|error| format!("{error}"))
    } else if args.is_empty() {
        evaluator
            .evaluate_item(fragment.item, &required_globals)
            .map_err(|error| format!("{error}"))
    } else {
        Err(format!(
            "compiled runtime fragment {} has no executable body",
            fragment.expr.as_raw()
        ))
    };
    if let Some(started_at) = started_at {
        profiler.record_fragment(fragment, started_at.elapsed());
    }
    result
}

fn backend_items_by_hir(
    core: &aivi_core::Module,
    backend: &BackendProgram,
) -> BTreeMap<aivi_hir::ItemId, BackendItemId> {
    let core_to_hir = core
        .items()
        .iter()
        .map(|(core_id, item)| (core_id, item.origin))
        .collect::<BTreeMap<_, _>>();
    backend
        .items()
        .iter()
        .filter_map(|(backend_id, item)| {
            core_to_hir
                .get(&item.origin)
                .copied()
                .map(|hir_id| (hir_id, backend_id))
        })
        .collect()
}

fn evaluate_compiled_run_text<'a>(
    shared: &'a RunHydrationStaticState,
    text: &'a CompiledRunText,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    env: &RuntimeBindingEnv,
    evaluators: &mut EvaluatorCache<'a>,
    profiler: &mut RunHydrationProfiler,
) -> Result<RuntimeValue, String> {
    profiler.record_text();
    let mut rendered = String::new();
    for segment in &text.segments {
        match segment {
            CompiledRunTextSegment::Text(text) => rendered.push_str(text),
            CompiledRunTextSegment::Interpolation(fragment) => {
                let value = strip_signal_runtime_value(evaluate_compiled_run_fragment(
                    shared, fragment, globals, env, evaluators, profiler,
                )?);
                if matches!(value, RuntimeValue::Callable(_)) {
                    return Err(format!(
                        "text interpolation for expression {} produced a callable runtime value",
                        fragment.expr.as_raw()
                    ));
                }
                rendered.push_str(&value.to_string());
            }
        }
    }
    Ok(RuntimeValue::Text(rendered.into_boxed_str()))
}

fn runtime_bool(value: RuntimeValue) -> Option<bool> {
    strip_signal_runtime_value(value).as_bool()
}

fn runtime_truthy_bool(value: RuntimeValue) -> Option<bool> {
    match strip_signal_runtime_value(value) {
        RuntimeValue::Bool(value) => Some(value),
        RuntimeValue::OptionNone
        | RuntimeValue::ResultErr(_)
        | RuntimeValue::ValidationInvalid(_) => Some(false),
        RuntimeValue::OptionSome(_)
        | RuntimeValue::ResultOk(_)
        | RuntimeValue::ValidationValid(_) => Some(true),
        _ => None,
    }
}

fn runtime_list_values(value: RuntimeValue) -> Option<Vec<RuntimeValue>> {
    match strip_signal_runtime_value(value) {
        RuntimeValue::List(values) => Some(values),
        _ => None,
    }
}

fn runtime_collection_key(value: RuntimeValue) -> Option<GtkCollectionKey> {
    let value = strip_signal_runtime_value(value);
    (!matches!(value, RuntimeValue::Callable(_))).then(|| GtkCollectionKey::new(value.to_string()))
}

fn strip_signal_runtime_value(mut value: RuntimeValue) -> RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = *inner;
    }
    value
}

fn strip_signal_runtime_ref(mut value: &RuntimeValue) -> &RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = inner.as_ref();
    }
    value
}

fn match_pattern(
    patterns: &RunPatternTable,
    pattern_id: HirPatternId,
    value: &RuntimeValue,
    bindings: &mut RuntimeBindingEnv,
) -> Result<bool, String> {
    let Some(pattern) = patterns.get(pattern_id) else {
        return Err(format!(
            "run artifact is missing serialized pattern {}",
            pattern_id.as_raw()
        ));
    };
    match &pattern.kind {
        RunPatternKind::Wildcard => Ok(true),
        RunPatternKind::Binding { binding, .. } => {
            bindings.insert(*binding, strip_signal_runtime_value(value.clone()));
            Ok(true)
        }
        RunPatternKind::Integer { raw } => Ok(matches!(
            strip_signal_runtime_value(value.clone()),
            RuntimeValue::Int(found) if raw.parse::<i64>().ok() == Some(found)
        )),
        RunPatternKind::Text { value: expected } => Ok(matches!(
            strip_signal_runtime_value(value.clone()),
            RuntimeValue::Text(found) if expected.as_ref() == found.as_ref()
        )),
        RunPatternKind::Tuple(elements) => {
            let RuntimeValue::Tuple(found) = strip_signal_runtime_value(value.clone()) else {
                return Ok(false);
            };
            let expected = elements.iter().copied().collect::<Vec<_>>();
            if expected.len() != found.len() {
                return Ok(false);
            }
            let mut matches = true;
            for (pattern, value) in expected.into_iter().zip(found.iter()) {
                matches &= match_pattern(patterns, pattern, value, bindings)?;
            }
            Ok(matches)
        }
        RunPatternKind::List { elements, rest } => {
            let RuntimeValue::List(found) = strip_signal_runtime_value(value.clone()) else {
                return Ok(false);
            };
            if found.len() < elements.len() {
                return Ok(false);
            }
            if rest.is_none() && found.len() != elements.len() {
                return Ok(false);
            }
            let mut matches = true;
            for (pattern, value) in elements.iter().copied().zip(found.iter()) {
                matches &= match_pattern(patterns, pattern, value, bindings)?;
            }
            if let Some(rest) = rest {
                let remaining = RuntimeValue::List(found[elements.len()..].to_vec());
                matches &= match_pattern(patterns, *rest, &remaining, bindings)?;
            }
            Ok(matches)
        }
        RunPatternKind::Record(fields) => {
            let RuntimeValue::Record(found) = strip_signal_runtime_value(value.clone()) else {
                return Ok(false);
            };
            let mut matches = true;
            for field in fields {
                let Some(found_field) = found
                    .iter()
                    .find(|candidate| candidate.label.as_ref() == field.label.as_ref())
                else {
                    return Ok(false);
                };
                matches &= match_pattern(patterns, field.pattern, &found_field.value, bindings)?;
            }
            Ok(matches)
        }
        RunPatternKind::Constructor { callee, arguments } => match callee {
            RunPatternConstructor::Builtin(term) => {
                match_builtin_pattern(*term, arguments, value, patterns, bindings)
            }
            RunPatternConstructor::Item { item, variant_name } => {
                let RuntimeValue::Sum(found) = strip_signal_runtime_value(value.clone()) else {
                    return Ok(false);
                };
                if found.item != *item || found.variant_name.as_ref() != variant_name.as_ref() {
                    return Ok(false);
                }
                if arguments.len() != found.fields.len() {
                    return Ok(false);
                }
                let mut matches = true;
                for (pattern, field) in arguments.iter().copied().zip(found.fields.iter()) {
                    matches &= match_pattern(patterns, pattern, field, bindings)?;
                }
                Ok(matches)
            }
        },
        RunPatternKind::UnresolvedName => Ok(false),
    }
}

fn match_builtin_pattern(
    term: BuiltinTerm,
    arguments: &[HirPatternId],
    value: &RuntimeValue,
    patterns: &RunPatternTable,
    bindings: &mut RuntimeBindingEnv,
) -> Result<bool, String> {
    let Some(payload) = truthy_falsy_payload(value, term) else {
        return Ok(false);
    };
    match (payload, arguments) {
        (None, []) => Ok(true),
        (Some(payload), [argument]) => match_pattern(patterns, *argument, &payload, bindings),
        _ => Ok(false),
    }
}

fn truthy_falsy_payload(
    value: &RuntimeValue,
    constructor: BuiltinTerm,
) -> Option<Option<RuntimeValue>> {
    match (constructor, strip_signal_runtime_value(value.clone())) {
        (BuiltinTerm::True, RuntimeValue::Bool(true))
        | (BuiltinTerm::False, RuntimeValue::Bool(false))
        | (BuiltinTerm::None, RuntimeValue::OptionNone) => Some(None),
        (BuiltinTerm::Some, RuntimeValue::OptionSome(payload))
        | (BuiltinTerm::Ok, RuntimeValue::ResultOk(payload))
        | (BuiltinTerm::Err, RuntimeValue::ResultErr(payload))
        | (BuiltinTerm::Valid, RuntimeValue::ValidationValid(payload))
        | (BuiltinTerm::Invalid, RuntimeValue::ValidationInvalid(payload)) => Some(Some(*payload)),
        _ => None,
    }
}

fn text_literal_static_text(text: &aivi_hir::TextLiteral) -> Option<String> {
    let mut rendered = String::new();
    for segment in &text.segments {
        match segment {
            aivi_hir::TextSegment::Text(fragment) => rendered.push_str(fragment.raw.as_ref()),
            aivi_hir::TextSegment::Interpolation(_) => return None,
        }
    }
    Some(rendered)
}
