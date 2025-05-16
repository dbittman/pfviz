use std::{collections::HashMap, path::PathBuf, time::Duration};

use crate::app::App;
use clap::{Parser, Subcommand};
use perf::EventKind;

pub mod app;
pub mod event;
pub mod perf;
pub mod single_file_ui;
pub mod trace;
pub mod ui;

#[derive(Parser, Clone, Copy, Debug, clap::ValueEnum)]
enum PlaybackMode {
    FrameStep,
    FrameTime,
    Realtime,
}

impl ToString for PlaybackMode {
    fn to_string(&self) -> String {
        match self {
            PlaybackMode::FrameStep => "frame-step",
            PlaybackMode::FrameTime => "frame-time",
            PlaybackMode::Realtime => "realtime",
        }
        .to_string()
    }
}

#[derive(Parser, Clone, Debug)]
pub struct PlayCli {
    #[arg(
        value_name = "FILE",
        help = "Path of trace file to use (default pfviz.json)"
    )]
    trace_file: Option<PathBuf>,
    #[arg(
        value_name = "FILE",
        help = "Path of trace file to use (default pfviz.dat)"
    )]
    data_file: Option<PathBuf>,
    #[arg(
        short,
        long,
        help = "Don't show files with fault counts below this value",
        default_value_t = 0
    )]
    cutoff: usize,
    #[arg(short, long, help = "Width of file bar", default_value_t = 40)]
    width: usize,
    #[arg(short, long, help = "Playback mode", default_value_t = PlaybackMode::Realtime)]
    play_mode: PlaybackMode,
    #[arg(
        short = 's',
        long,
        help = "Playback speed (meaning depends on mode)",
        default_value_t = 1.0
    )]
    play_speed: f32,
}

#[derive(Parser, Clone, Debug)]
pub struct InfoCli {
    #[arg(
        value_name = "FILE",
        help = "Path of trace file to use (default pfviz.json)"
    )]
    trace_file: Option<PathBuf>,
    #[arg(
        value_name = "FILE",
        help = "Path of trace file to use (default pfviz.dat)"
    )]
    data_file: Option<PathBuf>,
    #[arg(long, short, help = "List all events")]
    list: bool,
    #[arg(long, short, help = "Show stats for each object")]
    stats: bool,
}

#[derive(Parser, Clone, Debug)]
pub struct TraceCli {
    #[arg(short, long, value_name = "FILE", help = "Path of trace file to use")]
    output: Option<PathBuf>,
    #[arg(
        short,
        long = "event",
        value_name = "EVENT",
        help = "Perf event to trace, can be specified multiple times"
    )]
    events: Vec<String>,
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "COMMAND",
        help = "Command to trace"
    )]
    command: Vec<String>,
}

#[derive(Clone, Debug, Subcommand)]
enum SubCmd {
    Play(PlayCli),
    Trace(TraceCli),
    Info(InfoCli),
}

#[derive(Parser, Clone, Debug)]
struct Cli {
    #[command(subcommand)]
    sub_cmd: SubCmd,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();

    ctrlc::set_handler(|| {}).unwrap();

    let result = match cli.sub_cmd {
        SubCmd::Play(play_cli) => {
            let jsonfile = play_cli.trace_file.clone().unwrap_or("pfviz.json".into());
            let datafile = play_cli.data_file.clone().unwrap_or("pfviz.dat".into());
            let data = perf::FaultData::open(datafile, jsonfile)?;
            let terminal = ratatui::init();
            let app = App::new(play_cli, data);
            let result = app.run(terminal);
            ratatui::restore();
            result
        }
        SubCmd::Trace(trace_cli) => trace::trace(&trace_cli),
        SubCmd::Info(info_cli) => {
            let jsonfile = info_cli.trace_file.clone().unwrap_or("pfviz.json".into());
            let datafile = info_cli.data_file.clone().unwrap_or("pfviz.dat".into());
            let data = perf::FaultData::open(&datafile, &jsonfile)?;

            println!(
                "{} ({}): {} objects, {} events",
                jsonfile.display(),
                datafile.display(),
                data.json.objects.len(),
                data.records.slice().len()
            );
            println!("objects:");
            let mut v = vec![];
            let mut event_map = HashMap::new();
            for obj in &data.json.objects {
                let name = data.json.strings.resolve(obj.1.file).unwrap_or("[unknown]");
                v.push((obj, name));
                if info_cli.stats {
                    let events = data.records.slice().iter().filter(|r| r.obj_id() == *obj.0);
                    event_map.insert(*obj.0, events.collect::<Vec<_>>());
                }
            }

            v.sort_by(|a, b| a.0.1.faults.cmp(&b.0.1.faults));

            for obj in v {
                println!("{:4}: {} {}", obj.0.0, obj.0.1.faults, obj.1);
                if info_cli.stats {
                    let events = event_map.get(obj.0.0).unwrap();
                    let misses = events.iter().filter(|e| e.kind().is_miss()).count();
                    let faults = events.iter().filter(|e| e.kind().is_fault()).count();
                    println!("      {} misses, {} faults", misses, faults);
                }
            }

            if info_cli.list {
                for event in data.records.slice() {
                    println!(
                        "{:?}: {:?} at {:x}: {}",
                        event.time(),
                        event.kind(),
                        event.offset(),
                        data.json
                            .strings
                            .resolve(data.json.objects[&event.obj_id()].file)
                            .unwrap()
                    );
                }
            }

            Ok(())
        }
    };

    result
}
