use std::time::Duration;

use ratatui::{
    layout::{Constraint, Direction, Flex, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Sparkline, SparklineBar, Widget},
};

use crate::{
    perf::{EventKind, EventRecord, FaultData},
    ui::{CACHE_MAX, CACHE_SET, FaultProcessResult, FileVis, RegionInfo},
};

#[derive(Debug)]
pub struct SingleFileVis {
    components: Vec<FileComponent>,
    objid: usize,
}

impl SingleFileVis {
    pub fn new(fv: &FileVis) -> Self {
        let mut components = Vec::new();

        if components.is_empty() {
            let mut start = 0;
            let ps = 0x1000 * 512;
            let len = fv.end_off / 8;
            for _ in 0..8 {
                let name = format!("{} ({} - {})", fv.name, start, start + len);
                components.push(FileComponent::new(name, ps, start, len, fv.objid));
                start += len;
            }
        }
        Self {
            components,
            objid: fv.objid,
        }
    }

    pub fn obj_id(&self) -> usize {
        self.objid
    }

    pub fn calculate_decay(&mut self, time: Duration) {
        for comp in &mut self.components {
            comp.calculate_decay(time);
        }
    }

    pub fn fault(&mut self, faults: &[EventRecord], data: &FaultData) -> FaultProcessResult {
        for comp in &mut self.components {
            comp.fault(faults, data);
        }

        FaultProcessResult {
            hit_breakpoint: false,
            count: faults.len(),
        }
    }

    pub fn reset(&mut self) {
        for comp in &mut self.components {
            comp.reset();
        }
    }
}

impl Widget for &SingleFileVis {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        const MAX_V: usize = 32;
        let vcount = self.components.len().min(MAX_V);
        let layout = Layout::new(
            Direction::Vertical,
            Constraint::from_lengths(vec![4u16; vcount]),
        )
        .flex(Flex::SpaceAround);
        let splits = layout.split(area);

        for (idx, fv) in self.components.iter().enumerate() {
            let area = &splits[idx];
            fv.render(*area, buf);
        }
    }
}

impl Widget for &FileComponent {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let block = Block::new()
            .title(self.name.as_str())
            .borders(Borders::ALL)
            .title_bottom(format!(
                "{} pages, {}/{} f/m",
                self.len / self.page_size,
                self.faults,
                self.misses
            ));

        let inner = block.inner(area);
        let inner_layout = Layout::new(
            Direction::Vertical,
            &[Constraint::Length(1), Constraint::Length(1)],
        );
        let splits = inner_layout.split(inner);

        let fault_sparkline = Sparkline::default().max(CACHE_MAX).data(&self.faultdata);
        let cache_sparkline = Sparkline::default().max(CACHE_MAX).data(&self.cachedata);
        block.render(area, buf);
        cache_sparkline.render(splits[0], buf);
        fault_sparkline.render(splits[1], buf);
    }
}

#[derive(Debug)]
pub struct FileComponent {
    name: String,
    page_size: u64,
    faultdata: Vec<PageInfo>,
    cachedata: Vec<PageInfo>,
    faults: usize,
    misses: usize,
    start: u64,
    len: u64,
    objid: usize,
}

impl FileComponent {
    pub fn new(name: String, page_size: u64, start: u64, len: u64, objid: usize) -> Self {
        Self {
            name,
            objid,
            page_size,
            faultdata: Vec::new(),
            cachedata: Vec::new(),
            faults: 0,
            misses: 0,
            start,
            len,
        }
    }

    pub fn reset(&mut self) {
        self.faultdata.clear();
        self.cachedata.clear();
        self.misses = 0;
        self.faults = 0;
    }

    fn calculate_decay(&mut self, time: Duration) {
        for page in &mut self.cachedata {
            if page.time >= time {
                continue;
            }

            let decay_max = Duration::from_millis(1000);
            let diff = decay_max - (time - page.time).min(decay_max);

            let micros = diff.as_micros() as u64;
            if page.value.is_some() {
                page.value = Some(micros / 100);
            }
            if let Some(v) = page.value {
                if v > 0 {
                    //      page.value = Some(v - 100);
                }
            }
        }
        for page in &mut self.faultdata {
            if page.time >= time {
                continue;
            }

            let decay_max = Duration::from_millis(1000);
            let diff = decay_max - (time - page.time).min(decay_max);

            let micros = diff.as_micros() as u64;
            if page.value.is_some() {
                page.value = Some(micros / 100);
            }
            if let Some(v) = page.value {
                if v > 0 {
                    //page.value = Some(v - 1);
                }
            }
        }
    }

    pub fn fault(&mut self, faults: &[EventRecord], _fd: &FaultData) -> FaultProcessResult {
        for (idx, fault) in faults.iter().enumerate() {
            if fault.obj_id() != self.objid {
                continue;
            }
            if fault.offset() >= self.start + self.len || fault.offset() < self.start {
                continue;
            }
            let pos = ((fault.offset() - self.start) / self.page_size) as usize;
            let region_vec = if fault.kind().is_miss() {
                self.misses += 1;
                &mut self.cachedata
            } else {
                self.faults += 1;
                &mut self.faultdata
            };
            if pos >= region_vec.len() {
                region_vec.resize_with(pos + 1, || PageInfo::default());
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

            region_vec[pos as usize] =
                PageInfo::new(fault, Style::default().fg(colors.0).bg(colors.1));
            region_vec[pos as usize].value = Some(if fault.kind().is_miss() {
                CACHE_SET
            } else {
                CACHE_SET
            });
        }
        FaultProcessResult {
            count: faults.len(),
            hit_breakpoint: false,
        }
    }
}

#[derive(Debug, Default)]
struct PageInfo {
    last_addr: u64,
    value: Option<u64>,
    time: Duration,
    style: Style,
    has_major: Option<u32>,
}

impl PageInfo {
    fn new(fault: &EventRecord, style: Style) -> Self {
        Self {
            last_addr: fault.offset(),
            value: None,
            time: fault.time(),
            style,
            has_major: None,
        }
    }
}

impl Into<SparklineBar> for &PageInfo {
    fn into(self) -> SparklineBar {
        SparklineBar::from(self.value).style(self.style)
    }
}
