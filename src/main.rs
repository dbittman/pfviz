use std::{fs::File, io::BufReader, path::PathBuf, str::FromStr};

use crate::app::App;
use clap::{Parser, Subcommand};

pub mod app;
pub mod event;
pub mod perf;
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
struct PlayCli {
    #[arg(value_name = "FILE", help = "Path of trace file to use")]
    trace_file: PathBuf,
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
struct TraceCli {
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
}

#[derive(Parser, Clone, Debug)]
struct Cli {
    #[command(subcommand)]
    sub_cmd: SubCmd,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let cli = Cli::parse();

    let result = match cli.sub_cmd {
        SubCmd::Play(play_cli) => {
            let file = File::open(&play_cli.trace_file)?;
            let data = perf::parse_perf_data(BufReader::new(file))?;
            let terminal = ratatui::init();
            let app = App::new(play_cli, data);
            let result = app.run(terminal);
            ratatui::restore();
            result
        }
        SubCmd::Trace(trace_cli) => trace::trace(&trace_cli),
    };

    result
}
