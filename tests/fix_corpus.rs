//! Fixer validation over the fixture corpus: the safe phantom fixes reproduce
//! (dead members removed, result re-parses clean), and every crossing suggestion is
//! flagged UNVERIFIED with no bare value and the user's SDC left intact.

use cdc_sentinel::fix::{plan_fix, UNVERIFIED_BANNER};
use cdc_sentinel::source::{CoreSource, FsSource, SourceFile};
use cdc_sentinel::{design, sdc};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct Expected {
    lint_a: bool,
    lint_b: bool,
    phantom: Vec<String>,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn cases() -> Vec<(String, PathBuf, Expected)> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(fixtures()).unwrap().flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let txt = std::fs::read_to_string(dir.join("expected.json")).unwrap();
        let exp: Expected = serde_json::from_str(&txt).unwrap();
        v.push((dir.file_name().unwrap().to_string_lossy().into_owned(), dir, exp));
    }
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[test]
fn fixer_reproduces_safe_fixes_and_flags_all_suggestions() {
    let mut safe_fixed = Vec::new();
    let mut suggested = Vec::new();

    for (name, dir, exp) in cases() {
        let source = FsSource::open(&dir).expect("open fixture");
        let plan = plan_fix(name.clone(), &source);

        // --- Lint A: phantom cores get a safe fix that removes exactly the phantoms ---
        if exp.lint_a {
            assert!(plan.has_safe_fixes(), "{name}: phantom core must produce a safe fix");
            let removed: usize = plan.fixed_files.iter().map(|f| f.removed_groups.len()).sum();
            assert_eq!(removed, exp.phantom.len(), "{name}: removed one group per phantom");

            // the corrected SDC re-parses with no phantom tokens remaining
            for ff in plan.fixed_files.iter().filter(|f| !f.removed_groups.is_empty()) {
                let files = source.files();
                let dm = design::scan_design(&files);
                let corrected =
                    sdc::parse_sdc(&[SourceFile::new(ff.path.clone(), ff.fixed.clone())]);
                let universe = dm.clock_universe(&corrected.created_clocks);
                for t in corrected.async_group_pll_tokens() {
                    assert!(universe.contains(&t), "{name}: phantom {t} still present after fix");
                }
                // each named phantom is gone from the fixed text
                for ph in &exp.phantom {
                    assert!(!ff.fixed.contains(ph.as_str()), "{name}: {ph} removed from fixed SDC");
                }
            }
            safe_fixed.push(name.clone());
        } else {
            assert!(!plan.has_safe_fixes(), "{name}: no phantom → no safe fix");
        }

        // --- Lint B: crossing cores get an UNVERIFIED suggestion, never a value ---
        if exp.lint_b {
            assert!(!plan.suggestions.is_empty(), "{name}: crossing core must get a suggestion");
            for s in &plan.suggestions {
                assert!(s.block.contains(UNVERIFIED_BANNER), "{name}: banner present");
                assert!(s.block.contains("<N>"), "{name}: placeholder, not a bare number");
                for line in s.block.lines().filter(|l| l.contains("set_multicycle_path") || l.contains("set_false_path")) {
                    assert!(line.trim_start().starts_with('#'), "{name}: suggestion line commented");
                }
            }
            // the user's SDC is only appended to, never rewritten
            for ff in plan.fixed_files.iter().filter(|f| f.suggestions_appended > 0) {
                assert!(ff.fixed.starts_with(&ff.original), "{name}: user SDC untouched above the block");
            }
            suggested.push(name.clone());
        } else {
            assert!(plan.suggestions.is_empty(), "{name}: no crossing → no suggestion");
        }
    }

    safe_fixed.sort();
    suggested.sort();
    // the phantom population (safe-fixed) and the Lint-B population (suggested) match the survey
    assert_eq!(safe_fixed, vec!["m72".to_string(), "snes".to_string()]);
    assert_eq!(
        suggested,
        vec![
            "msx".to_string(),
            "pokemonmini".to_string(),
            "supervision".to_string(),
            "wonderswan".to_string()
        ]
    );
}
