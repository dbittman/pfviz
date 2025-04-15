use std::{collections::HashMap, time::Duration};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    widgets::{Block, Borders, Paragraph, Sparkline, SparklineBar, Widget},
};

use crate::{
    PlayCli,
    app::App,
    perf::{EventKind, EventRecord, FaultData, PAGE_SIZE},
};

#[derive(Debug)]
pub struct Ui {
    pub fault_vis: FaultVis,
    pub status: Status,
    pub map: HashMap<usize, usize>,
}

impl Ui {
    pub fn new(cli: &PlayCli, data: &FaultData) -> Self {
        let mut map = HashMap::new();
        Self {
            fault_vis: FaultVis::new(cli, data, &mut map),
            status: Status::new(cli, data),
            map,
        }
    }

    pub fn reset(&mut self) {
        self.status.reset();
        self.fault_vis.reset();
    }
}

impl Widget for &App {
    /// Renders the user interface widgets.
    ///
    // This is where you add new widgets.
    // See the following resources:
    // - https://docs.rs/ratatui/latest/ratatui/widgets/index.html
    // - https://github.com/ratatui/ratatui/tree/master/examples
    fn render(self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::new(
            Direction::Vertical,
            &[Constraint::Fill(1), Constraint::Length(8)],
        );
        let split = layout.split(area);

        self.ui.fault_vis.render(split[0], buf);
        self.ui.status.render(split[1], buf);
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct RegionInfo {
    last_addr: u64,
    value: Option<u64>,
    time: Duration,
    style: Style,
    has_major: Option<u32>,
}

impl RegionInfo {
    pub const fn new(last_addr: u64, time: Duration, style: Style) -> Self {
        Self {
            last_addr,
            time,
            style,
            value: None,
            has_major: None,
        }
    }
}

#[derive(Debug)]
pub struct FileVis {
    faultdata: Vec<RegionInfo>,
    cachedata: Vec<RegionInfo>,
    name: String,
    start_off: u64,
    end_off: u64,
    bar_size: u64,
    faults: usize,
    misses: usize,
    is_highlighted: bool,
    breakpoint: bool,
}

impl Into<SparklineBar> for &RegionInfo {
    fn into(self) -> SparklineBar {
        SparklineBar::from(self.value).style(self.style)
    }
}

#[derive(Clone, Copy)]
pub struct FaultProcessResult {
    pub hit_breakpoint: bool,
    pub count: usize,
}

impl FileVis {
    pub fn new(name: String, start_off: u64, end_off: u64, bar_size: u64) -> Self {
        let len = ((1 + end_off - start_off) / bar_size) - 1;
        let data = vec![
            RegionInfo::new(0, Duration::ZERO, Style::default().bg(Color::DarkGray));
            len.try_into().unwrap()
        ];
        Self {
            faults: 0,
            faultdata: data.clone(),
            cachedata: data,
            name,
            start_off,
            end_off,
            bar_size,
            misses: 0,
            is_highlighted: false,
            breakpoint: false,
        }
    }

    pub fn reset(&mut self) {
        let len = ((1 + self.end_off - self.start_off) / self.bar_size) - 1;
        let data = vec![
            RegionInfo::new(0, Duration::ZERO, Style::default().bg(Color::DarkGray));
            len.try_into().unwrap()
        ];
        self.faultdata = data.clone();
        self.cachedata = data.clone();
        self.misses = 0;
        self.faults = 0;
    }

    pub fn toggle_break(&mut self) {
        self.breakpoint = !self.breakpoint;
    }

    pub fn fault(&mut self, faults: &[EventRecord], _fd: &FaultData) -> FaultProcessResult {
        for (idx, fault) in faults.iter().enumerate() {
            let pos = ((fault.offset() - self.start_off) / self.bar_size) as usize;
            let region_vec = if fault.kind().is_miss() {
                self.misses += 1;
                &mut self.cachedata
            } else {
                self.faults += 1;
                &mut self.faultdata
            };
            if pos >= region_vec.len() {
                continue;
            }

            let mut has_recent_major = if let Some(count) = &mut region_vec[pos].has_major {
                if *count == 0 {
                    false
                } else {
                    *count -= 1;
                    true
                }
            } else {
                false
            };

            if fault.kind() == EventKind::MajorFault {
                region_vec[pos].has_major = Some(100);
                has_recent_major = true;
            }

            if !has_recent_major {
                region_vec[pos].has_major = None;
            }

            let mut colors = if fault.kind() == EventKind::MajorFault {
                (Color::LightRed, Color::Red)
            } else if has_recent_major {
                (Color::LightMagenta, Color::Magenta)
            } else {
                (Color::LightBlue, Color::Blue)
            };

            if fault.kind().is_miss() {
                colors = (Color::LightGreen, Color::Green);
            }

            region_vec[pos as usize] = RegionInfo::new(
                fault.offset(),
                fault.time(),
                Style::default().fg(colors.0).bg(colors.1),
            );
            for i in 0..region_vec.len() {
                if i != pos as usize {
                    if region_vec[i].value == Some(1) {
                        region_vec[i].value = Some(0);
                    }
                }
            }
            region_vec[pos as usize].value = Some(1);

            if self.breakpoint {
                return FaultProcessResult {
                    count: idx + 1,
                    hit_breakpoint: true,
                };
            }
        }
        FaultProcessResult {
            count: faults.len(),
            hit_breakpoint: false,
        }
    }
}

impl Widget for &FileVis {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let start = humansize::format_size(self.start_off, humansize::BINARY);
        let end = humansize::format_size(self.end_off, humansize::BINARY);
        let bs = humansize::format_size(self.bar_size, humansize::BINARY);
        let style = if self.is_highlighted {
            Style::default().bold()
        } else {
            Style::default()
        };
        let title = if self.breakpoint {
            &format!("(B) {}", self.name.as_str())
        } else {
            &self.name
        };
        let block = Block::new()
            .title(title.as_str())
            .title_style(style)
            .borders(Borders::ALL)
            .title_bottom(format!(
                "[{}-{}): {} {} bars, {}/{} f/m",
                start,
                end,
                self.faultdata.len(),
                bs,
                self.faults,
                self.misses
            ));

        let inner = block.inner(area);
        let inner_layout = Layout::new(
            Direction::Vertical,
            &[Constraint::Length(1), Constraint::Length(1)],
        );
        let splits = inner_layout.split(inner);

        let fault_sparkline = Sparkline::default().max(1).data(&self.faultdata);
        let cache_sparkline = Sparkline::default().max(1).data(&self.cachedata);
        block.render(area, buf);
        cache_sparkline.render(splits[0], buf);
        fault_sparkline.render(splits[1], buf);
    }
}

#[derive(Debug)]
pub struct FaultVis {
    file_vis: Vec<FileVis>,
    width: u16,
    highlighted: Option<usize>,
}

impl FaultVis {
    pub fn new(cli: &PlayCli, data: &FaultData, map: &mut HashMap<usize, usize>) -> Self {
        let mut file_vis = Vec::new();
        for object in data.json.objects.values() {
            if cli.cutoff > object.faults || !object.show {
                continue;
            }
            let mut name = data.json.strings.resolve(object.file).unwrap().to_string();
            let start = object
                .smallest_offset
                .next_multiple_of(PAGE_SIZE)
                .saturating_sub(PAGE_SIZE);
            let end = object.biggest_offset.next_multiple_of(PAGE_SIZE);
            let bar_size = ((end - start) / cli.width as u64)
                .max(PAGE_SIZE)
                .next_multiple_of(PAGE_SIZE);
            if name.len() > cli.width - 8 {
                name = spat::shorten(name).to_string_lossy().to_string();
                let cut = name.len().saturating_sub(cli.width - 8);
                name = "...".to_string() + &name[cut..name.len()];
            }
            map.insert(object.idx, file_vis.len());
            file_vis.push(FileVis::new(name, start, end, bar_size));
        }
        Self {
            file_vis,
            width: cli.width as u16,
            highlighted: None,
        }
    }

    pub fn reset(&mut self) {
        for fv in &mut self.file_vis {
            fv.reset();
        }
    }

    pub fn fault(
        &mut self,
        faults: &[EventRecord],
        data: &FaultData,
        map: &HashMap<usize, usize>,
    ) -> FaultProcessResult {
        let mut count = 0;
        for fault in faults {
            let Some(idx) = map.get(&fault.obj_id()) else {
                continue;
            };
            let res = self.file_vis[*idx].fault(&[*fault], data);
            if res.hit_breakpoint {
                return FaultProcessResult {
                    hit_breakpoint: true,
                    count: count + res.count,
                };
            }
            count += res.count;
        }

        FaultProcessResult {
            hit_breakpoint: false,
            count: faults.len(),
        }
    }

    pub fn toggle_break(&mut self) {
        let Some(highlight) = self.highlighted else {
            return;
        };

        self.file_vis[highlight].toggle_break()
    }

    pub fn move_highlight(&mut self, up: bool) {
        if let Some(old) = self.highlighted {
            self.file_vis[old].is_highlighted = false;
        }
        let value = self
            .highlighted
            .map(|old| {
                if up {
                    old.saturating_sub(1)
                } else {
                    old.saturating_add(1).min(self.file_vis.len() - 1)
                }
            })
            .unwrap_or(0);
        self.highlighted = Some(value);
        self.file_vis[value].is_highlighted = true;
    }
}

impl Widget for &FaultVis {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        const MAX_H: usize = 32;
        const MAX_V: usize = 32;
        let hcount: usize = usize::try_from(area.as_size().width / (self.width + 4))
            .unwrap()
            .min(MAX_H);
        let vcount = (self.file_vis.len() / hcount + 1).min(MAX_V);
        let layout = Layout::new(
            Direction::Vertical,
            Constraint::from_lengths(vec![4u16; vcount]),
        )
        .flex(Flex::SpaceAround);
        let splits = layout.split(area);

        let hlayout = Layout::new(
            Direction::Horizontal,
            Constraint::from_lengths(vec![self.width + 4; hcount]),
        )
        .flex(Flex::SpaceAround);

        let allsplits = splits
            .into_iter()
            .map(|vs| hlayout.split(*vs))
            .collect::<Vec<_>>();

        for (idx, fv) in self.file_vis.iter().enumerate() {
            let area = &allsplits[idx / hcount];
            let area = area[idx % hcount];
            fv.render(area, buf);
        }
    }
}

#[derive(Debug)]
pub struct Status {
    pub num_events: usize,
    pub cur_event: usize,
    pub end_time: Duration,
    pub cur_time: Duration,
    trace_file: String,
    pub current: String,
    pub marker_a: Option<usize>,
    pub marker_b: Option<usize>,
    pub looping: bool,
    pub paused: bool,
}

impl Status {
    pub fn new(cli: &PlayCli, data: &FaultData) -> Self {
        let end_time = data
            .records
            .slice()
            .iter()
            .max_by(|a, b| a.time().cmp(&b.time()))
            .map_or(Duration::ZERO, |f| f.time());
        Self {
            num_events: data.records.slice().len(),
            cur_event: 0,
            end_time,
            cur_time: Duration::ZERO,
            trace_file: cli
                .trace_file
                .as_ref()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or("pfviz.json".into()),
            current: "".into(),
            marker_a: None,
            marker_b: None,
            looping: true,
            paused: true,
        }
    }

    pub fn reset(&mut self) {
        self.cur_event = 0;
        self.cur_time = Duration::ZERO;
        self.current = "".into();
    }

    pub fn fault(
        &mut self,
        idx: usize,
        faults: &[EventRecord],
        data: &FaultData,
        hit_breakpoint: bool,
    ) {
        if faults.len() == 0 {
            return;
        }
        let fault = faults.last().unwrap();
        let off = humansize::format_size(fault.offset(), humansize::BINARY);
        let s = format!(
            "(..{}) {:10}: {} to {} within {}{}",
            faults.len() - 1,
            idx,
            fault.kind().to_string(),
            off,
            data.object_name(fault),
            if hit_breakpoint { "[BREAKPOINT]" } else { "" }
        );
        self.current = s;
    }
}

impl Widget for &Status {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let cs = self.cur_time.as_secs() % 60;
        let cm = (self.cur_time.as_secs() / 60) % 60;
        let ch = (self.cur_time.as_secs() / 3600) % 60;
        let cn = self.cur_time.as_nanos() % 1_000_000_000;
        let es = self.end_time.as_secs() % 60;
        let em = (self.end_time.as_secs() / 60) % 60;
        let eh = (self.end_time.as_secs() / 3600) % 60;
        let en = self.end_time.as_nanos() % 1_000_000_000;
        let cur_time = format!("{:02}:{:02}:{:02}.{:09}", ch, cm, cs, cn);
        let end_time = format!("{:02}:{:02}:{:02}.{:09}", eh, em, es, en);

        let mut status_title = format!("Playback [{}]", &self.trace_file);
        if let Some(a) = self.marker_a {
            status_title += &format!("(Marker A: {:10})", a);
        }
        if let Some(b) = self.marker_b {
            status_title += &format!("(Marker B: {:10})", b);
        }

        if self.paused {
            status_title += "(paused)";
        }

        if self.looping {
            status_title += "(looping)";
        }

        let playback_block = Block::default().borders(Borders::ALL).title(status_title).title_bottom("Help: (q) Quit; (Left/Right) Move Events; (Up/Down) Select File; (,/.) Set Marker A/B; (</>) Goto Marker A/B; (Space) Pause; (b) Set Breakpoint");

        let playback_inner = playback_block.inner(area);

        let layout = Layout::new(
            Direction::Vertical,
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ],
        );
        let playback_inner_splits = layout.split(playback_inner);

        let prog_bar_layout = Layout::new(
            Direction::Horizontal,
            [Constraint::Length(50), Constraint::Fill(1)],
        );

        let prog_bar_splits = prog_bar_layout.split(playback_inner_splits[0]);
        let time_bar_splits = prog_bar_layout.split(playback_inner_splits[1]);

        let progress_len = prog_bar_splits[1].width as usize;
        let progress = self.cur_event as f32 / self.num_events as f32;
        let progress_style = Style::default().fg(Color::LightRed).bg(Color::Red);
        let mut progress_data = vec![
            SparklineBar::from(Some(0)).style(progress_style);
            (progress * progress_len as f32) as usize
        ];
        if let Some(last) = progress_data.last_mut() {
            *last = SparklineBar::from(Some(1)).style(progress_style);
        }

        if let Some(a) = self.marker_a {
            let pos = a * (progress_len - 1) / self.num_events;
            if progress_data.get(pos).is_some() {
                progress_data[pos] =
                    progress_data[pos].style(Style::default().fg(Color::DarkGray).bg(Color::Black));
            }
        }

        if let Some(b) = self.marker_b {
            let pos = b * (progress_len - 1) / self.num_events;
            if progress_data.get(pos).is_some() {
                progress_data[pos] =
                    progress_data[pos].style(Style::default().fg(Color::White).bg(Color::Gray));
            }
        }

        let progress_bar = Sparkline::default()
            .data(progress_data)
            .style(progress_style)
            .max(1)
            .absent_value_style(Style::default().bg(Color::Black));

        let time_progress_style = Style::default().fg(Color::LightMagenta).bg(Color::Magenta);
        let time_progress_len = time_bar_splits[1].width as usize;
        let time_progress = self.cur_time.as_secs_f32() / self.end_time.as_secs_f32();
        let mut time_progress_data =
            vec![SparklineBar::from(Some(0)); (time_progress * time_progress_len as f32) as usize];

        if let Some(last) = time_progress_data.last_mut() {
            *last = SparklineBar::from(Some(1)).style(time_progress_style);
        }
        let time_progress_bar = Sparkline::default()
            .data(time_progress_data)
            .max(1)
            .style(time_progress_style)
            .absent_value_style(Style::default().bg(Color::Black));

        let time_bar_text = Paragraph::new(format!(" time: {} / {}", cur_time, end_time));
        let prog_bar_text = Paragraph::new(format!(
            "event: {:18} / {:18}",
            self.cur_event, self.num_events
        ));

        let log = Paragraph::new(self.current.as_str());

        playback_block.render(area, buf);
        progress_bar.render(prog_bar_splits[1], buf);
        prog_bar_text.render(prog_bar_splits[0], buf);
        time_progress_bar.render(time_bar_splits[1], buf);
        time_bar_text.render(time_bar_splits[0], buf);
        log.render(playback_inner_splits[2], buf);
    }
}
