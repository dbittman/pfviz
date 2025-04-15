use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, BufWriter},
    path::PathBuf,
    process::{Command, Stdio},
};

use color_eyre::eyre::{Result, bail};

use crate::{TraceCli, perf::EventKind};

pub fn trace(cli: &TraceCli) -> Result<()> {
    let mut command = Command::new("perf");
    command
        .arg("record")
        .arg("-Tda")
        .arg("-c")
        .arg("1")
        .arg("--all-user");

    let mut ev_map = HashMap::new();
    for event in &cli.events {
        let split = event.split(",").collect::<Vec<_>>();
        if split.len() != 2 {
            bail!(
                "EVENT must be of the form `<perf-event>,<type>'. <type> may be one of miss,major,minor."
            );
        }
        command.arg("-e").arg(split[0]);

        ev_map.insert(split[0], EventKind::from(split[1]));
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
        .arg("--no-demangle")
        .arg("--ns");

    command.stdout(Stdio::piped());

    tracing::debug!("starting perf script with: `{:?}'", command);
    let mut child = command.spawn()?;

    let stdout = child.stdout.take().unwrap();

    let perf_data = crate::perf::parse_perf_data(BufReader::new(stdout), ev_map)?;

    if !child.wait()?.success() {
        bail!("perf script failed");
    }

    let out_file = File::create(
        cli.output
            .clone()
            .unwrap_or_else(|| PathBuf::from("pfviz.dat")),
    )?;
    let out_file_json = File::create(
        cli.output
            .clone()
            .unwrap_or_else(|| PathBuf::from("pfviz.json")),
    )?;
    crate::perf::write_perf_data(
        &perf_data,
        BufWriter::new(out_file),
        BufWriter::new(out_file_json),
    )?;

    Ok(())
}
