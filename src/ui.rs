use std::{
    char::MAX,
    io::{Write, stdin, stdout},
    time::Duration,
};

use itertools::Itertools;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    widgets::{Block, BorderType, Borders, Paragraph, Sparkline, SparklineBar, Widget},
};
use smallvec::{SmallVec, smallvec};
use tracing_subscriber::Layer;

use crate::{
    app::App,
    perf::{Object, PAGE_SIZE, PageFault, PerfData},
};

#[derive(Debug)]
pub struct Ui {
    pub fault_vis: FaultVis,
    pub status: Status,
}

impl Ui {
    pub fn new(perf: &mut PerfData, width: u16, trace_file: impl ToString) -> Self {
        Self {
            fault_vis: FaultVis::new(perf, width),
            status: Status::new(perf, trace_file),
        }
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
        let layout = Layout::new(Direction::Vertical, Constraint::from_percentages([70, 30]));
        let split = layout.split(area);

        self.ui.fault_vis.render(split[0], buf);
        self.ui.status.render(split[1], buf);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FaultInfo {
    last_addr: u64,
    value: u64,
    time: Duration,
    style: Style,
}

impl FaultInfo {
    pub const fn new(last_addr: u64, time: Duration, style: Style) -> Self {
        Self {
            last_addr,
            time,
            style,
            value: 0,
        }
    }
}

#[derive(Debug)]
pub struct FileFaultVis {
    data: Vec<FaultInfo>,
    name: String,
    start_off: u64,
    end_off: u64,
    bar_size: u64,
    obj_idx: usize,
    faults: usize,
}

impl Into<SparklineBar> for &FaultInfo {
    fn into(self) -> SparklineBar {
        SparklineBar::from(self.value).style(self.style)
    }
}

impl FileFaultVis {
    pub fn new(name: String, start_off: u64, end_off: u64, bar_size: u64, obj_idx: usize) -> Self {
        let len = ((1 + end_off - start_off) / bar_size) - 1;
        let data = vec![
            FaultInfo::new(0, Duration::ZERO, Style::default().bg(Color::DarkGray));
            len.try_into().unwrap()
        ];
        Self {
            faults: 0,
            obj_idx,
            data,
            name,
            start_off,
            end_off,
            bar_size,
        }
    }

    pub fn fault(&mut self, fault: &PageFault, perf: &PerfData) {
        let pos = (fault.offset - self.start_off) / self.bar_size;
        if pos as usize >= self.data.len() {
            return;
        }
        self.faults += 1;
        self.data[pos as usize] = FaultInfo::new(
            fault.offset,
            fault.time,
            Style::default().bg(Color::LightYellow),
        );
        self.data[pos as usize].value = 1;
    }
}

impl Widget for &FileFaultVis {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let start = humansize::format_size(self.start_off, humansize::BINARY);
        let end = humansize::format_size(self.end_off, humansize::BINARY);
        let bs = humansize::format_size(self.bar_size, humansize::BINARY);
        let sparkline = Sparkline::default()
            .max(1)
            .block(
                Block::new()
                    .title(&*self.name)
                    .borders(Borders::ALL)
                    .title_bottom(format!(
                        "[{}-{}): {} {} bars, {} faults",
                        start,
                        end,
                        self.data.len(),
                        bs,
                        self.faults,
                    )),
            )
            .data(&self.data);
        sparkline.render(area, buf);
    }
}

#[derive(Debug)]
pub struct FaultVis {
    file_vis: Vec<FileFaultVis>,
    width: u16,
}

impl FaultVis {
    pub fn new(perf: &mut PerfData, width: u16) -> Self {
        let mut file_vis = Vec::new();
        for object in &mut perf.objects {
            let mut name = perf.strings.resolve(object.1.file).unwrap().to_string();
            let start = object
                .1
                .smallest_offset
                .next_multiple_of(PAGE_SIZE)
                .saturating_sub(PAGE_SIZE);
            let end = object.1.biggest_offset.next_multiple_of(PAGE_SIZE);
            let bar_size = ((end - start) / width as u64)
                .max(PAGE_SIZE)
                .next_multiple_of(PAGE_SIZE);
            if name.len() > width as usize - 8 {
                let cut = (width as usize - 8).saturating_sub(name.len());
                name = "...".to_string() + &name[cut..name.len()];
            }
            object.1.vis_idx = Some(file_vis.len());
            file_vis.push(FileFaultVis::new(name, start, end, bar_size, object.1.idx));
        }
        Self { file_vis, width }
    }

    pub fn fault(&mut self, fault: &PageFault, perf: &PerfData) {
        let obj = &perf.objects[fault.obj_idx];
        let Some(idx) = obj.vis_idx else {
            return;
        };
        self.file_vis[idx].fault(fault, perf);
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
            Constraint::from_lengths(vec![3u16; vcount]),
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
            let area = allsplits[idx / hcount][idx % hcount];
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
    pub log: Vec<String>,
}

impl Status {
    pub fn new(perf: &PerfData, trace_file: impl ToString) -> Self {
        let end_time = perf
            .faults
            .iter()
            .max_by(|a, b| a.time.cmp(&b.time))
            .map_or(Duration::ZERO, |f| f.time);
        Self {
            num_events: perf.faults.len(),
            cur_event: 0,
            end_time,
            cur_time: Duration::ZERO,
            trace_file: trace_file.to_string(),
            log: Vec::new(),
        }
    }

    pub fn fault(&mut self, idx: usize, fault: &PageFault, perf: &PerfData) {
        let off = humansize::format_size(fault.offset, humansize::BINARY);
        let s = format!(
            "{:10}: fault to {} within {}",
            idx,
            off,
            perf.object_name(fault.obj_idx)
        );
        self.log.push(s);
    }
}

const LOG_LEN: usize = 5;
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
        let cur_time = format!("{:02}:{:02}:{:02}.{:06}", ch, cm, cs, cn);
        let end_time = format!("{:02}:{:02}:{:02}.{:06}", eh, em, es, en);
        let line = Paragraph::new(format!(
            "trace {}: event {} / {} at time {} of {}",
            &self.trace_file, self.cur_event, self.num_events, cur_time, end_time
        ))
        .block(Block::default().borders(Borders::ALL).title("Status"));

        let layout = Layout::vertical([Constraint::Min(LOG_LEN as u16 + 2), Constraint::Length(3)]);
        let splits = layout.split(area);

        let log = Paragraph::new(
            self.log
                .iter()
                .rev()
                .take(splits[0].as_size().height as usize - 2)
                .join("\n"),
        )
        .block(Block::default().borders(Borders::ALL).title("Fault Log"));

        line.render(splits[1], buf);
        log.render(splits[0], buf);
    }
}
