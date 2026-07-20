//! cdc-sentinel CLI: lint one or more core directories.
//!
//! ```text
//! cdc-sentinel [--json] <core-dir> [<core-dir> ...]
//! ```
//!
//! Human output by default; `--json` emits a machine-readable array shaped for a
//! downstream tool's CDC-hotspot ingest. Exit status is non-zero if any high-
//! severity (Lint B) finding fired, so it can be used as a gate.

use cdc_sentinel::error::{Error, Result};
use cdc_sentinel::lint::{analyze, CoreReport, Lint};
use cdc_sentinel::report::{corpus_json, to_human};
use cdc_sentinel::source::FsSource;
use std::path::Path;
use std::process::ExitCode;

fn run() -> Result<Vec<CoreReport>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut json = false;
    let mut dirs: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            "-h" | "--help" => {
                return Err(Error::Usage(
                    "cdc-sentinel [--json] <core-dir> [<core-dir> ...]".into(),
                ))
            }
            _ => dirs.push(a),
        }
    }
    if dirs.is_empty() {
        return Err(Error::Usage(
            "cdc-sentinel [--json] <core-dir> [<core-dir> ...]".into(),
        ));
    }

    let mut reports = Vec::new();
    for d in &dirs {
        let source = FsSource::open(d)?;
        let name = Path::new(d)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| d.clone());
        reports.push(analyze(name, &source));
    }

    if json {
        println!("{}", corpus_json(&reports));
    } else {
        for r in &reports {
            print!("{}", to_human(r));
        }
    }
    Ok(reports)
}

fn main() -> ExitCode {
    match run() {
        Ok(reports) => {
            let any_high = reports.iter().any(|r| r.fired(Lint::B));
            if any_high {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("cdc-sentinel: {e}");
            ExitCode::from(2)
        }
    }
}
