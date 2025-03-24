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

    let data = perf::parse_perf_data("trace")?;

    let terminal = ratatui::init();
    let app = App::new(data, 40, "trace");
    tracing::info!("ready");
    let result = app.run(terminal);
    ratatui::restore();
    result
}
