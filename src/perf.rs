use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    time::{Duration, Instant},
};

use color_eyre::eyre::Result;
use smallvec::SmallVec;
use stable_vec::StableVec;
use string_interner::{DefaultStringInterner, DefaultSymbol, StringInterner};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq)]
pub enum EventKind {
    MajorFault,
    MinorFault,
    CacheMiss,
}

impl ToString for EventKind {
    fn to_string(&self) -> String {
        match self {
            EventKind::MajorFault => "major-fault",
            EventKind::MinorFault => "minor-fault",
            EventKind::CacheMiss => "cache-miss",
        }
        .to_string()
    }
}

#[derive(Debug)]
pub struct PerfData {
    pub faults: Vec<Event>,
    pub objects: StableVec<Object>,
    pub strings: DefaultStringInterner,
}

impl PerfData {
    pub fn object_name(&self, idx: usize) -> &str {
        let symbol = self.objects[idx].file;
        self.strings.resolve(symbol).unwrap_or("<unknown>")
    }
}

pub const PAGE_SIZE: u64 = 0x1000;

#[derive(Debug, Clone, Copy)]
pub struct PerfEvent {
    name: DefaultSymbol,
    sym: DefaultSymbol,
    addr: u64,
    ip: u64,
    time: Timestamp,
}

#[derive(Debug, Clone, Copy)]
pub struct Timestamp {
    sec: u64,
    nsec: u64,
}

impl Into<Duration> for Timestamp {
    fn into(self) -> Duration {
        Duration::new(self.sec, self.nsec as u32)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MMap {
    file: DefaultSymbol,
    offset: u64,
    addr: u64,
    len: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Object {
    pub file: DefaultSymbol,
    pub idx: usize,
    pub maps: usize,
    pub faults: usize,
    pub biggest_offset: u64,
    pub smallest_offset: u64,
    pub vis_idx: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct Event {
    pub obj_idx: usize,
    pub offset: u64,
    pub was_write: bool,
    pub time: Duration,
    pub kind: EventKind,
}

pub fn parse_perf_data<P: AsRef<Path>>(path: P) -> Result<PerfData> {
    tracing::info!("reading file {}", path.as_ref().display());
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut strings = StringInterner::<
        string_interner::DefaultBackend,
        string_interner::DefaultHashBuilder,
    >::new();
    let mut events = Vec::new();
    let mut maps = Vec::new();
    for line in reader.lines().enumerate() {
        if let Ok(line) = line.1 {
            let split = line.split_whitespace().collect::<SmallVec<[_; 16]>>();
            let pid = split[0];
            let timesplit = split[1].split(".").collect::<SmallVec<[_; 2]>>();
            let time = (
                u64::from_str_radix(timesplit[0], 10)?,
                u64::from_str_radix(&timesplit[1][..(timesplit[1].len() - 1)], 10)?,
            );
            //  let time = sscanf::sscanf!(split[1], "{u64}.{u64}:")
            //     .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
            let name = split[2];
            if name == "PERF_RECORD_MMAP" || name == "PERF_RECORD_MMAP2" {
                let pids = sscanf::sscanf!(split[3], "{i64}/{i64}:")
                    .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
                if pids.0 < 0 || pids.1 < 0 {
                    continue;
                }

                if pids.0 == pids.1 && pids.0 != 0 {
                    let addr = sscanf::sscanf!(split[4], "[{u64:x}({u64:x})")
                        .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
                    let offset = sscanf::sscanf!(split[6], "{u64:x}")
                        .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
                    let mapfile = split[11];
                    maps.push(MMap {
                        file: strings.get_or_intern(mapfile),
                        offset,
                        addr: addr.0,
                        len: addr.1,
                    })
                }
            } else {
                if u64::from_str_radix(pid, 10)? == 0 {
                    continue;
                }
                let addr = u64::from_str_radix(split[3], 16)?;
                if addr == 0 {
                    continue;
                }
                let sym = split[4];
                let ip = u64::from_str_radix(split[5], 16)
                    .inspect_err(|_| tracing::warn!("invalid line: {}", line))?;

                let name = strings.get_or_intern(name);
                let sym = strings.get_or_intern(sym);

                events.push(PerfEvent {
                    name,
                    sym,
                    addr,
                    ip,
                    time: Timestamp {
                        sec: time.0,
                        nsec: time.1 * 1000,
                    },
                });
            }
        }
    }

    let mut objects = StableVec::new();
    let mut addrmap = nonoverlapping_interval_tree::NonOverlappingIntervalTree::new();
    let mut objmap = HashMap::new();
    tracing::info!("parsing {} maps", maps.len());
    for map in maps {
        let entry = objmap.entry(map.file).or_insert_with(|| {
            let idx = objects.next_push_index();
            objects.push(Object {
                idx,
                file: map.file,
                maps: 0,
                faults: 0,
                biggest_offset: 0,
                smallest_offset: u64::MAX,
                vis_idx: None,
            });
            idx
        });
        addrmap.insert(map.addr..(map.addr + map.len), (*entry, map));
    }

    tracing::info!("parsing {} events", events.len());
    let faults = events
        .into_iter()
        .filter_map(|event| {
            let addr = event.addr & !0xfff;
            if let Some(info) = addrmap.get(&addr) {
                if addr != 0 {
                    let map_offset = addr.checked_sub(info.1.addr).unwrap();
                    let offset = map_offset + info.1.offset;
                    strings.resolve(event.name).and_then(|event_name| {
                        let kind = if event_name.starts_with("minor-faults") {
                            Some(EventKind::MinorFault)
                        } else if event_name.starts_with("major-faults") {
                            Some(EventKind::MajorFault)
                        } else if event_name.starts_with("cache-misses") {
                            Some(EventKind::CacheMiss)
                        } else {
                            None
                        };
                        kind.map(|kind| Event {
                            obj_idx: info.0,
                            offset,
                            was_write: false, //TODO
                            time: event.time.into(),
                            kind,
                        })
                    })
                } else {
                    None
                }
            } else {
                //tracing::warn!("page-fault to untracked address {:x}", event.addr);
                None
            }
        })
        .collect::<Vec<_>>();

    for fault in &faults {
        objects[fault.obj_idx].faults += 1;
        objects[fault.obj_idx].biggest_offset =
            objects[fault.obj_idx].biggest_offset.max(fault.offset);
        objects[fault.obj_idx].smallest_offset =
            objects[fault.obj_idx].smallest_offset.min(fault.offset);
    }

    // Filter objects
    for idx in 0..objects.num_elements() {
        let Some(object) = objects.get_mut(idx) else {
            continue;
        };
        if object.faults == 0 {
            objects.remove(idx);
            continue;
        }
        if object.biggest_offset == 0 {
            object.biggest_offset = PAGE_SIZE;
        }
        object.biggest_offset = object.biggest_offset.next_multiple_of(PAGE_SIZE);
        object.smallest_offset = object
            .smallest_offset
            .next_multiple_of(PAGE_SIZE)
            .saturating_sub(PAGE_SIZE);
    }

    tracing::info!(
        "parsing complete: {} events, {} objects",
        faults.len(),
        objects.num_elements()
    );

    Ok(PerfData {
        faults,
        objects,
        strings,
    })
}
