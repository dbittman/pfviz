use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    time::Instant,
};

use color_eyre::eyre::Result;
use string_interner::{DefaultStringInterner, DefaultSymbol, StringInterner};

pub struct PerfData {}

#[derive(Debug, Clone, Copy)]
pub struct Event {
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

#[derive(Debug, Clone, Copy)]
pub struct MMap {
    file: DefaultSymbol,
    offset: u64,
    addr: u64,
    len: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Object {
    file: DefaultSymbol,
    maps: usize,
    faults: usize,
    biggest_offset: u64,
    smallest_offset: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PageFault {
    obj_idx: usize,
    offset: u64,
    was_write: bool,
}

pub fn parse_perf_data<P: AsRef<Path>>(path: P, target_pid: u64) -> Result<PerfData> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut strings = StringInterner::<
        string_interner::DefaultBackend,
        string_interner::DefaultHashBuilder,
    >::new();
    let mut events = Vec::new();
    let mut maps = Vec::new();
    for line in reader.lines() {
        if let Ok(line) = line {
            let split = line.split_whitespace().collect::<Vec<_>>();
            let pid = split[0];
            let time = sscanf::sscanf!(split[1], "{u64}.{u64}:")
                .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
            let name = split[2];
            if name == "PERF_RECORD_MMAP" || name == "PERF_RECORD_MMAP2" {
                let pids = sscanf::sscanf!(split[3], "{i64}/{i64}:")
                    .map_err(|e| color_eyre::eyre::eyre!(e.to_string()))?;
                if pids.0 < 0 || pids.1 < 0 {
                    continue;
                }

                if pids.0 == pids.1 && pids.0 == target_pid as i64 {
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
                if u64::from_str_radix(pid, 10)? != target_pid {
                    continue;
                }
                let addr = u64::from_str_radix(split[3], 16)?;
                let sym = split[4];
                let ip = u64::from_str_radix(split[5], 16)?;

                let name = strings.get_or_intern(name);
                let sym = strings.get_or_intern(sym);

                events.push(Event {
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

    let mut addrs = HashMap::new();
    let mut objects = Vec::new();
    let mut objmap = HashMap::new();
    tracing::info!("parsing {} maps", maps.len());
    for map in maps {
        let objmap_entry = objmap.entry(map.file).or_insert_with(|| {
            let idx = objects.len();
            objects.push(Object {
                file: map.file,
                maps: 0,
                faults: 0,
                biggest_offset: 0,
                smallest_offset: u64::MAX,
            });
            idx
        });
        objects[*objmap_entry].maps += 1;
        for a in (map.addr..(map.addr + map.len)).step_by(0x1000) {
            addrs.insert(a, (*objmap_entry, map));
        }
    }

    tracing::info!("parsing {} events", events.len());
    let faults = events
        .into_iter()
        .filter_map(|event| {
            let addr = event.addr & !0xfff;
            if let Some(info) = addrs.get(&addr) {
                let map_offset = addr.checked_sub(info.1.addr).unwrap();
                let offset = map_offset + info.1.offset;
                if offset > 0x1000000 {
                    println!(
                        "==> {:x}: {:x} {:x} {:x} {:x}",
                        offset, map_offset, addr, info.1.addr, info.1.offset
                    );
                }
                Some(PageFault {
                    obj_idx: info.0,
                    offset,
                    was_write: false, //TODO
                })
            } else {
                tracing::warn!("page-fault to untracked address {:x}", event.addr);
                None
            }
        })
        .collect::<Vec<_>>();

    for fault in faults {
        //let name = strings.resolve(objects[fault.obj_idx].file).unwrap();
        //println!("fault to {} : {:x}", name, fault.offset);
        objects[fault.obj_idx].faults += 1;
        objects[fault.obj_idx].biggest_offset =
            objects[fault.obj_idx].biggest_offset.max(fault.offset);
        objects[fault.obj_idx].smallest_offset =
            objects[fault.obj_idx].smallest_offset.min(fault.offset);
    }

    for obj in &mut objects {
        if obj.smallest_offset == u64::MAX {
            obj.smallest_offset = 0;
        }
    }

    println!("==> {:#?}", objects);

    todo!()
}
