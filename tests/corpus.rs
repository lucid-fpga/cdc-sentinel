//! Corpus validation — the strong check.
//!
//! Each directory under `fixtures/` is a synthesized reproduction of one archetype
//! from the 31-core clock/PLL/CDC survey the lints were derived from: its `.sdc` +
//! `.v` reproduce the STRUCTURE the survey recorded (blanket async cut, which PLL
//! modules ship, external-memory presence, added datapath timing) — no vendor RTL
//! is copied. `expected.json` carries the survey's classification for that core.
//!
//! Running cdc-sentinel over the corpus and asserting agreement with those
//! classifications is the correctness proof: phantom cores flagged by Lint A, the
//! external-memory-no-timing cores flagged by Lint B, and the re-timed and
//! BRAM-only counter-examples NOT flagged.

use cdc_sentinel::lint::{analyze, Lint};
use cdc_sentinel::source::FsSource;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct Expected {
    core: String,
    blanket_async: bool,
    external_memory: bool,
    added_timing: bool,
    phantom: Vec<String>,
    lint_a: bool,
    lint_b: bool,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn fixture_cases() -> Vec<(String, PathBuf, Expected)> {
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(fixtures_dir()).expect("fixtures/ exists").flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let exp_path = dir.join("expected.json");
        let txt = std::fs::read_to_string(&exp_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", exp_path.display()));
        let exp: Expected = serde_json::from_str(&txt)
            .unwrap_or_else(|e| panic!("parse {}: {e}", exp_path.display()));
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        cases.push((name, dir, exp));
    }
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(cases.len() >= 10, "expected the full archetype corpus, got {}", cases.len());
    cases
}

#[test]
fn corpus_reproduces_survey_classifications() {
    let mut a_fired = Vec::new();
    let mut b_fired = Vec::new();

    for (name, dir, exp) in fixture_cases() {
        let source = FsSource::open(&dir).expect("open fixture");
        let report = analyze(exp.core.clone(), &source);
        let s = &report.summary;

        // structural facts the survey observed
        assert_eq!(s.blanket_async, exp.blanket_async, "{name}: blanket_async");
        assert_eq!(s.external_memory, exp.external_memory, "{name}: external_memory");
        assert_eq!(
            !s.added_timing.is_empty(),
            exp.added_timing,
            "{name}: added_timing (got {:?})",
            s.added_timing
        );

        // Lint A: exact phantom set
        let mut a_subjects: Vec<String> = report
            .findings
            .iter()
            .filter(|f| f.lint == Lint::A)
            .map(|f| f.subject.clone())
            .collect();
        a_subjects.sort();
        let mut want = exp.phantom.clone();
        want.sort();
        assert_eq!(a_subjects, want, "{name}: Lint A phantom set");

        // Lint firing verdicts match the survey classification
        assert_eq!(report.fired(Lint::A), exp.lint_a, "{name}: Lint A verdict");
        assert_eq!(report.fired(Lint::B), exp.lint_b, "{name}: Lint B verdict");

        if report.fired(Lint::A) {
            a_fired.push(name.clone());
        }
        if report.fired(Lint::B) {
            b_fired.push(name.clone());
        }
    }

    a_fired.sort();
    b_fired.sort();

    // The named populations from the survey, reproduced exactly:
    // Lint A (phantom) = m72 + snes; Lint B (ext-mem, no timing) = the 4 targets.
    assert_eq!(a_fired, vec!["m72".to_string(), "snes".to_string()], "Lint A population");
    assert_eq!(
        b_fired,
        vec![
            "msx".to_string(),
            "pokemonmini".to_string(),
            "supervision".to_string(),
            "wonderswan".to_string(),
        ],
        "Lint B population"
    );
}

#[test]
fn msx_archetype_cli_shape() {
    // the named Lint B archetype: multi-PLL external-memory core, no added timing
    let dir = fixtures_dir().join("msx");
    let report = analyze("computer-msx", &FsSource::open(&dir).expect("open msx"));
    assert!(report.fired(Lint::B), "msx must fire Lint B");
    assert!(!report.fired(Lint::A), "msx has no phantom");
    let b = report.findings.iter().find(|f| f.lint == Lint::B).unwrap();
    assert!(b.reason.contains("external-memory"));
    assert!(b.evidence.contains("sdram"), "evidence cites the controller: {}", b.evidence);
}
