use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::PathBuf,
    process::{Command, Stdio},
};

use color_eyre::eyre::{Result, bail};

use crate::TraceCli;

pub fn trace(cli: &TraceCli) -> Result<()> {
    let mut command = Command::new("perf");
    command
        .arg("record")
        .arg("-Tda")
        .arg("-c")
        .arg("1")
        .arg("--all-user");

    for event in &cli.events {
        let split = event.split(",").collect::<Vec<_>>();
        if split.len() != 2 {
            bail!(
                "EVENT must be of the form `<perf-event>,<type>'. <type> may be one of miss,major,minor."
            );
        }
        command.arg("-e").arg(split[0]);
    }

    command.arg("-e").arg("major-faults:u");
    command.arg("-e").arg("minor-faults:u");

    for arg in &cli.command {
        command.arg(arg);
    }

    tracing::debug!("starting perf trace with: `{:?}'", command);

    let status = command.status()?;
    if !status.success() {
        bail!("perf record failed");
    }

    let mut command = Command::new("perf");
    command
        .arg("script")
        .arg("-F")
        .arg("time,event,addr,sym,ip,cpu,tid")
        .arg("--show-mmap-events")
        .arg("--reltime")
        .arg("--ns");

    command.stdout(Stdio::piped());

    tracing::debug!("starting perf script with: `{:?}'", command);
    let mut child = command.spawn()?;

    let stdout = child.stdout.take().unwrap();

    let perf_data = crate::perf::parse_perf_data(BufReader::new(stdout))?;

    if !child.wait()?.success() {
        bail!("perf script failed");
    }

    let out_file = File::create(
        cli.output
            .clone()
            .unwrap_or_else(|| PathBuf::from("pfviz.dat")),
    )?;
    let out_file_strings = File::create(
        cli.output
            .clone()
            .unwrap_or_else(|| PathBuf::from("pfviz.json")),
    )?;
    crate::perf::write_perf_data(&perf_data, BufWriter::new(out_file))?;

    serde_json::to_writer(out_file_strings, &perf_data.strings)?;

    Ok(())
}
