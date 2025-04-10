use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::Path,
    time::Duration,
};

use color_eyre::eyre::{Result, bail};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stable_vec::StableVec;
use string_interner::{DefaultStringInterner, DefaultSymbol, StringInterner, Symbol};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq)]
pub enum EventKind {
    Unknown,
    MajorFault,
    MinorFault,
    CacheMiss,
}

impl EventKind {
    pub fn is_miss(&self) -> bool {
        matches!(self, EventKind::CacheMiss) || matches!(self, EventKind::Unknown)
    }

    pub fn is_fault(&self) -> bool {
        // maybe will expand in future
        !self.is_miss()
    }
}

impl Into<u32> for EventKind {
    fn into(self) -> u32 {
        match self {
            EventKind::MajorFault => 1,
            EventKind::MinorFault => 2,
            EventKind::CacheMiss => 3,
            EventKind::Unknown => 0,
        }
    }
}

impl From<u32> for EventKind {
    fn from(value: u32) -> Self {
        match value {
            1 => EventKind::MajorFault,
            2 => EventKind::MinorFault,
            3 => EventKind::CacheMiss,
            _ => EventKind::Unknown,
        }
    }
}

impl ToString for EventKind {
    fn to_string(&self) -> String {
        match self {
            EventKind::MajorFault => "major-fault",
            EventKind::MinorFault => "minor-fault",
            EventKind::CacheMiss => "cache-miss",
            EventKind::Unknown => "unknown",
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

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct PerfEvent {
    name: DefaultSymbol,
    sym: DefaultSymbol,
    addr_sym: DefaultSymbol,
    addr: u64,
    ip: u64,
    time: Timestamp,
    tid: u32,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Object {
    pub file: DefaultSymbol,
    pub idx: usize,
    pub maps: usize,
    pub faults: usize,
    pub biggest_offset: u64,
    pub smallest_offset: u64,
    pub show: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Event {
    pub obj_idx: usize,
    pub offset: u64,
    pub was_write: bool,
    pub time: Duration,
    pub kind: EventKind,
    pub event_name: DefaultSymbol,
    pub addr: u64,
    pub ip: u64,
    pub tid: u32,
}

pub fn parse_perf_data<Io: Read>(reader: BufReader<Io>) -> Result<PerfData> {
    let mut strings = StringInterner::<
        string_interner::DefaultBackend,
        string_interner::DefaultHashBuilder,
    >::new();
    let mut events = Vec::new();
    let mut maps = Vec::new();
    let mut count = 0;
    for line in reader.lines().enumerate() {
        if let Ok(line) = line.1 {
            let split = line.split_whitespace().collect::<SmallVec<[_; 16]>>();
            let tid = split[0];
            let tid = u32::from_str_radix(tid, 10)?;
            if tid == 0 {
                continue;
            }
            count += 1;
            if count % 1000 == 0 {
                eprint!("event: {count}              \r");
            }
            let _cpu = split[1];
            let timesplit = split[2].split(".").collect::<SmallVec<[_; 2]>>();
            let time = (
                u64::from_str_radix(timesplit[0], 10)?,
                u64::from_str_radix(&timesplit[1][..(timesplit[1].len() - 1)], 10)?,
            );
            let name = split[3];
            if name == "PERF_RECORD_MMAP" || name == "PERF_RECORD_MMAP2" {
                let pids = sscanf::sscanf!(split[4], "{i64}/{i64}:")
                    .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
                if pids.0 < 0 || pids.1 < 0 {
                    continue;
                }

                if pids.0 != 0 && pids.0 != 0 {
                    let addr = sscanf::sscanf!(split[5], "[{u64:x}({u64:x})")
                        .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))
                        .inspect_err(|_| tracing::warn!("invalid line: {}", line))?;
                    let offset = sscanf::sscanf!(split[7], "{u64:x}")
                        .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))
                        .inspect_err(|_| tracing::warn!("invalid line: {}", line))?;
                    let mapfile = split[12];
                    maps.push(MMap {
                        file: strings.get_or_intern(mapfile),
                        offset,
                        addr: addr.0,
                        len: addr.1,
                    })
                }
            } else {
                let addr = u64::from_str_radix(split[4], 16)?;
                if addr == 0 {
                    continue;
                }
                let sym = split.get(7).unwrap_or(&"[unknown]");
                let mut addr_sym = split[5];
                let ip_nr = if split.len() == 7 {
                    5
                } else if split.len() >= 8 {
                    6
                } else {
                    bail!("invalid line: {}", line);
                };
                let ip = u64::from_str_radix(split[ip_nr], 16)
                    .inspect_err(|_| tracing::warn!("invalid line: {}", line))?;

                if u64::from_str_radix(addr_sym, 16).is_ok() {
                    // Probably means the symbol wasn't printed.
                    addr_sym = "[unknown]";
                }

                let name = strings.get_or_intern(name);
                let sym = strings.get_or_intern(sym);
                let addr_sym = strings.get_or_intern(addr_sym);

                events.push(PerfEvent {
                    name,
                    sym,
                    addr_sym,
                    addr,
                    ip,
                    tid,
                    time: Timestamp {
                        sec: time.0,
                        nsec: time.1,
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
        tracing::debug!(
            "map: {:?} {} {}",
            strings.resolve(map.file),
            map.addr,
            map.len
        );
        let entry = objmap.entry(map.file).or_insert_with(|| {
            let idx = objects.next_push_index();
            objects.push(Object {
                idx,
                file: map.file,
                maps: 0,
                faults: 0,
                biggest_offset: 0,
                smallest_offset: u64::MAX,
                show: true,
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
                    let map_offset = event.addr.checked_sub(info.1.addr).unwrap();
                    let offset = map_offset + info.1.offset;
                    strings.resolve(event.name).and_then(|event_name| {
                        let kind = if event_name.starts_with("minor-faults") {
                            Some(EventKind::MinorFault)
                        } else if event_name.starts_with("major-faults") {
                            Some(EventKind::MajorFault)
                        } else if event_name.starts_with("cache-misses") {
                            Some(EventKind::CacheMiss)
                        } else {
                            Some(EventKind::Unknown)
                        };
                        kind.map(|kind| Event {
                            obj_idx: info.0,
                            offset,
                            was_write: false, //TODO
                            time: event.time.into(),
                            kind,
                            event_name: event.name,
                            addr: event.addr,
                            ip: event.ip,
                            tid: event.tid,
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
        tracing::debug!(
            "object: {} {}",
            strings.resolve(object.file).unwrap_or("[unknown]"),
            object.faults
        );
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

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EventRecord {
    addr: u64,
    ip: u64,
    offset: u64,
    time_ns: u64,
    _resv: u64,
    kind: u32,
    flags: u32,
    event_name: u32,
    obj_id: u32,
    tid: u32,
    cpu: u32,
}

impl EventRecord {
    pub fn time(&self) -> Duration {
        Duration::from_nanos(self.time_ns)
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn kind(&self) -> EventKind {
        self.kind.into()
    }

    pub fn obj_id(&self) -> usize {
        self.obj_id as usize
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RecordHeader {
    magic: u64,
    count: u64,
    _resv: [u64; 6],
}

impl RecordHeader {
    pub fn record_count(&self) -> usize {
        self.count as usize
    }

    pub fn is_valid(&self) -> bool {
        self.magic == 0xAAAA1111CAFED00D
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRoot {
    pub objects: HashMap<usize, Object>,
    pub strings: DefaultStringInterner,
}

pub fn write_perf_data<W: Write, WJ: Write>(
    pd: &PerfData,
    mut out: BufWriter<W>,
    out_json: BufWriter<WJ>,
) -> Result<()> {
    out.write(bytemuck::bytes_of(&RecordHeader {
        magic: 0xAAAA1111CAFED00D,
        count: pd.faults.len() as u64,
        _resv: [0; 6],
    }))?;
    for ev in &pd.faults {
        let record = EventRecord {
            addr: ev.addr,
            ip: ev.ip,
            offset: ev.offset,
            time_ns: ev.time.as_nanos() as u64,
            kind: ev.kind.into(),
            flags: 0,
            event_name: ev.event_name.to_usize() as u32,
            obj_id: ev.obj_idx as u32,
            tid: ev.tid,
            cpu: 0,
            _resv: 0,
        };

        out.write(bytemuck::bytes_of(&record))?;
    }
    out.flush()?;

    let root = JsonRoot {
        strings: pd.strings.clone(),
        objects: pd.objects.iter().map(|x| (x.0, x.1.clone())).collect(),
    };

    serde_json::to_writer(out_json, &root)?;
    Ok(())
}

#[derive(Debug)]
pub struct Records {
    map: Mmap,
}

impl Records {
    fn record_start(&self) -> *const EventRecord {
        unsafe {
            self.map
                .as_ptr()
                .add(size_of::<RecordHeader>())
                .cast::<EventRecord>()
        }
    }

    pub fn header(&self) -> &RecordHeader {
        unsafe { self.map.as_ptr().cast::<RecordHeader>().as_ref().unwrap() }
    }

    pub fn slice(&self) -> &[EventRecord] {
        unsafe { core::slice::from_raw_parts(self.record_start(), self.header().record_count()) }
    }
}

pub fn mmap_records<P: AsRef<Path>>(path: P) -> Result<Records> {
    let file = File::open(path)?;
    let recs = unsafe { memmap2::Mmap::map(&file) }.map(|map| Records { map })?;
    if !recs.header().is_valid() {
        bail!("invalid header in records file");
    }
    Ok(recs)
}

pub fn open_json_root<P: AsRef<Path>>(path: P) -> Result<JsonRoot> {
    let file = File::open(path)?;
    let root = serde_json::from_reader(BufReader::new(file))?;
    Ok(root)
}

#[derive(Debug)]
pub struct FaultData {
    pub json: JsonRoot,
    pub records: Records,
}

impl FaultData {
    pub fn open<P: AsRef<Path>, P2: AsRef<Path>>(data: P, json: P2) -> Result<Self> {
        Ok(Self {
            json: open_json_root(json)?,
            records: mmap_records(data)?,
        })
    }

    pub fn object(&self, fault: &EventRecord) -> &Object {
        &self.json.objects[&(fault.obj_id as usize)]
    }

    pub fn object_name(&self, fault: &EventRecord) -> &str {
        let obj = self.object(fault);
        self.json.strings.resolve(obj.file).unwrap_or("[unknown]")
    }
}
