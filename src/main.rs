use crate::app::App;

pub mod app;
pub mod event;
pub mod perf;
pub mod ui;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let data = perf::parse_perf_data("trace", 1529647)?;

    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}
