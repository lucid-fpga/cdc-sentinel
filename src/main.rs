//! cdc-sentinel CLI: lint core directories, or fix them.
//!
//! ```text
//! cdc-sentinel [--json] <core-dir> ...              # lint (default)
//! cdc-sentinel --fix [--in-place] <core-dir> ...    # emit corrected SDC + guided suggestions
//! cdc-sentinel --emit-template-patch                # the upstream root fix (patch + PR body)
//! ```
//!
//! Lint mode exits non-zero if any high-severity (Lint B) finding fired. `--fix`
//! writes the corrected SDC to a new `*.fixed.sdc` file next to each source (the
//! user's SDC is never overwritten unless `--in-place` is given), and prints the
//! UNVERIFIED crossing suggestions. `--emit-template-patch` prints the proposed
//! upstream patch + PR body for a maintainer to review and submit.

use cdc_sentinel::error::{Error, Result};
use cdc_sentinel::fix::{plan_fix, FixPlan};
use cdc_sentinel::lint::{analyze, CoreReport, Lint};
use cdc_sentinel::patch::emit_template_patch;
use cdc_sentinel::report::{corpus_json, to_human};
use cdc_sentinel::source::FsSource;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "cdc-sentinel [--json] <core-dir> ...            (lint)\n       \
                     cdc-sentinel --fix [--in-place] <core-dir> ...  (fix)\n       \
                     cdc-sentinel --emit-template-patch              (upstream root fix)";

#[derive(Default)]
struct Opts {
    json: bool,
    fix: bool,
    in_place: bool,
    template_patch: bool,
    dirs: Vec<String>,
}

fn parse() -> Result<Opts> {
    let mut o = Opts::default();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--json" => o.json = true,
            "--fix" => o.fix = true,
            "--in-place" => o.in_place = true,
            "--emit-template-patch" => o.template_patch = true,
            "-h" | "--help" => return Err(Error::Usage(USAGE.into())),
            _ => o.dirs.push(a),
        }
    }
    Ok(o)
}

fn core_name(dir: &str) -> String {
    Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string())
}

/// `sdc/foo.sdc` → `sdc/foo.fixed.sdc`; anything else gets `.fixed` appended.
fn fixed_sibling(dir: &str, rel: &str) -> PathBuf {
    let base = Path::new(dir).join(rel);
    if let Some(stem) = rel.strip_suffix(".sdc") {
        Path::new(dir).join(format!("{stem}.fixed.sdc"))
    } else {
        base.with_extension(format!(
            "{}.fixed",
            base.extension().and_then(|e| e.to_str()).unwrap_or("")
        ))
    }
}

fn print_fix_summary(plan: &FixPlan) {
    println!("=== fix: {} ===", plan.core);
    if !plan.has_safe_fixes() {
        println!("  no safe auto-fixes (no phantom groups)");
    }
    for ff in &plan.fixed_files {
        for g in &ff.removed_groups {
            println!("  [safe-fix] {}: removed dead group {}", ff.path, g);
        }
        if ff.suggestions_appended > 0 {
            println!("  [suggest ] {}: appended {} UNVERIFIED crossing suggestion(s)", ff.path, ff.suggestions_appended);
        }
    }
    for s in &plan.suggestions {
        println!("  [UNVERIFIED] {}", s.crossing);
    }
    for n in &plan.notes {
        println!("  note: {n}");
    }
}

fn run_fix(o: &Opts) -> Result<()> {
    if o.dirs.is_empty() {
        return Err(Error::Usage(USAGE.into()));
    }
    for d in &o.dirs {
        let source = FsSource::open(d)?;
        let plan = plan_fix(core_name(d), &source);
        print_fix_summary(&plan);
        for ff in &plan.fixed_files {
            if !ff.changed() {
                continue;
            }
            let target = if o.in_place {
                Path::new(d).join(&ff.path)
            } else {
                fixed_sibling(d, &ff.path)
            };
            std::fs::write(&target, &ff.fixed).map_err(|source| Error::Source {
                path: target.display().to_string(),
                source,
            })?;
            println!("  wrote {}", target.display());
        }
        println!();
    }
    Ok(())
}

fn run_lint(o: &Opts) -> Result<Vec<CoreReport>> {
    if o.dirs.is_empty() {
        return Err(Error::Usage(USAGE.into()));
    }
    let mut reports = Vec::new();
    for d in &o.dirs {
        let source = FsSource::open(d)?;
        reports.push(analyze(core_name(d), &source));
    }
    if o.json {
        println!("{}", corpus_json(&reports));
    } else {
        for r in &reports {
            print!("{}", to_human(r));
        }
    }
    Ok(reports)
}

fn run() -> Result<i32> {
    let o = parse()?;

    if o.template_patch {
        let p = emit_template_patch();
        println!("# ===== proposed patch (apply against the current template) =====");
        println!("{}", p.patch);
        println!("# ===== proposed PR body (for a maintainer to review + submit) =====");
        println!("{}", p.pr_body);
        println!("# NOTE: cdc-sentinel does not open the PR — this is for a maintainer to submit.");
        return Ok(0);
    }

    if o.fix {
        run_fix(&o)?;
        return Ok(0);
    }

    let reports = run_lint(&o)?;
    Ok(if reports.iter().any(|r| r.fired(Lint::B)) { 1 } else { 0 })
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("cdc-sentinel: {e}");
            ExitCode::from(2)
        }
    }
}
