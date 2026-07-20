//! The fixer (detect → explain → **fix**), governed by one honesty guardrail:
//!
//! - **Lint A (phantom clock-group member) is safely auto-fixable.** Removing a
//!   `-group {}` that names only a clock the design never instantiates deletes a
//!   no-op line and changes nothing real. The fixer emits the corrected SDC —
//!   byte-identical except the dead member is gone.
//! - **Lint B (unconstrained real crossing) is NOT safely auto-fixable.** The
//!   correct multicycle/false-path value depends on the real timing relationship,
//!   which a heuristic text scan does not know. The fixer therefore NEVER emits a
//!   specific value as if correct — it appends a clearly-marked **UNVERIFIED**
//!   guided suggestion (a commented constraint template with placeholders and a
//!   "set the value and confirm in STA" banner), never a silent auto-fix.
//!
//! Fix generation is a pure function of the parsed models + the raw SDC text, so it
//! is unit-tested directly. The user's SDC is never overwritten unless the caller
//! opts in.

use crate::design::DesignModel;
use crate::sdc::{SdcModel, PLL_PRIMITIVES};
use crate::source::{CoreSource, SourceFile};
use regex::Regex;

/// A corrected SDC file: the safe phantom removals applied, plus (on the file that
/// carries the blanket cut) any appended UNVERIFIED crossing suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedFile {
    /// Source path (relative).
    pub path: String,
    /// The original text, verbatim.
    pub original: String,
    /// The corrected text.
    pub fixed: String,
    /// Human descriptions of the dead group members removed (empty ⇒ none).
    pub removed_groups: Vec<String>,
    /// How many UNVERIFIED suggestion blocks were appended to this file.
    pub suggestions_appended: usize,
}

impl FixedFile {
    /// True if this file's content actually changed.
    pub fn changed(&self) -> bool {
        self.original != self.fixed
    }
}

/// A guided, UNVERIFIED suggestion for a real crossing (Lint B). Never a bare value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossingSuggestion {
    /// A short description of the crossing.
    pub crossing: String,
    /// The commented suggestion block (banner + placeholders).
    pub block: String,
}

/// The complete fix plan for a core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixPlan {
    /// Core label.
    pub core: String,
    /// Corrected SDC files (only those that changed).
    pub fixed_files: Vec<FixedFile>,
    /// The UNVERIFIED crossing suggestions produced.
    pub suggestions: Vec<CrossingSuggestion>,
    /// Notes / limits.
    pub notes: Vec<String>,
}

impl FixPlan {
    /// True if any safe auto-fix was produced.
    pub fn has_safe_fixes(&self) -> bool {
        self.fixed_files.iter().any(|f| !f.removed_groups.is_empty())
    }
}

/// The banner every crossing suggestion carries. Its presence is asserted by tests.
pub const UNVERIFIED_BANNER: &str = "UNVERIFIED — set the real value and confirm in STA";

fn pll_tokens(body: &str) -> Vec<String> {
    let re = Regex::new(r"(?i)\b([a-z0-9]*_pll)\b").unwrap();
    let mut out = Vec::new();
    for c in re.captures_iter(body) {
        let t = c[1].to_ascii_lowercase();
        if PLL_PRIMITIVES.contains(&t.as_str()) {
            continue;
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// A `-group {}` clause is a pure-phantom member when it names at least one
/// non-primitive `*_pll` token and every such token is outside the clock universe.
fn group_is_pure_phantom(body: &str, universe: &[String]) -> Option<Vec<String>> {
    let tokens = pll_tokens(body);
    if tokens.is_empty() {
        return None;
    }
    if tokens.iter().all(|t| !universe.contains(t)) {
        Some(tokens)
    } else {
        None
    }
}

/// Remove pure-phantom `-group {}` clauses from an SDC file's text, preserving every
/// other byte. Returns the corrected text and the removed members' descriptions.
fn remove_phantom_groups(text: &str, universe: &[String]) -> (String, Vec<String>) {
    let group_re = Regex::new(r"-group\s*\{[^}]*\}").unwrap();
    let trailing_bs = Regex::new(r"\s*\\\s*$").unwrap();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let is_header = line.to_ascii_lowercase().contains("set_clock_groups")
            && !line.trim_start().starts_with('#');
        if !is_header {
            out.push(line.to_string());
            i += 1;
            continue;
        }

        // collect the physical lines of this statement (continued by trailing '\')
        let mut j = i;
        while lines[j].trim_end().ends_with('\\') && j + 1 < lines.len() {
            j += 1;
        }
        let stmt = &lines[i..=j];

        // rewrite each statement line, dropping pure-phantom -group clauses
        let mut kept: Vec<String> = Vec::new();
        for sl in stmt {
            let mut new_line = String::new();
            let mut last = 0usize;
            for m in group_re.find_iter(sl) {
                let body = &sl[m.start()..m.end()];
                if let Some(tokens) = group_is_pure_phantom(body, universe) {
                    new_line.push_str(&sl[last..m.start()]);
                    last = m.end();
                    removed.push(format!("{} (clock: {})", body.trim(), tokens.join(", ")));
                }
            }
            new_line.push_str(&sl[last..]);
            // drop the line entirely if nothing meaningful remains (a bare group line)
            let stripped = new_line.trim_end().trim_end_matches('\\').trim();
            if stripped.is_empty() {
                continue;
            }
            kept.push(new_line);
        }

        // fix continuation backslashes: all but the last kept line end with '\',
        // the last must not.
        let n = kept.len();
        for (idx, k) in kept.iter_mut().enumerate() {
            let has_bs = k.trim_end().ends_with('\\');
            if idx + 1 == n && has_bs {
                *k = trailing_bs.replace(k, "").into_owned();
            } else if idx + 1 < n && !has_bs {
                k.push_str(" \\");
            }
        }
        out.extend(kept);
        i = j + 1;
    }

    (out.join("\n"), removed)
}

/// Build the UNVERIFIED guided suggestion block for one crossing. It is fully
/// commented (so it is never applied blind), uses placeholders (never a bare
/// number), and carries the banner.
fn suggestion_block(crossing: &str, evidence: &str) -> String {
    let mut b = String::new();
    b.push_str("\n# ============================================================\n");
    b.push_str("# cdc-sentinel: GUIDED SUGGESTION — do NOT apply blind.\n");
    b.push_str(&format!("# Crossing: {crossing}\n"));
    b.push_str(&format!("# Evidence: {evidence}\n"));
    b.push_str("# This crossing sits under the blanket -asynchronous cut with no datapath\n");
    b.push_str("# timing, so static timing analysis is blind to it. Replace the blanket cut\n");
    b.push_str("# for THIS crossing with real datapath timing. The correct value depends on\n");
    b.push_str("# your actual clock relationship — cdc-sentinel does NOT know it.\n");
    b.push_str(&format!("#\n#   {UNVERIFIED_BANNER}:\n"));
    b.push_str("#   set_multicycle_path -from [get_clocks <CORE_CLK>] -to [get_clocks <MEM_CLK>] <N>  ;# set <N>, verify in STA\n");
    b.push_str("#   # -- or, only if the path is genuinely a don't-care: --\n");
    b.push_str("#   set_false_path -from [get_clocks <CORE_CLK>] -to [get_clocks <MEM_CLK>]           ;# confirm the path is truly async\n");
    b.push_str("# ============================================================\n");
    b
}

/// Produce the fix plan from parsed models + the raw source files. Pure.
pub fn plan_fix_models(
    core: impl Into<String>,
    sdc: &SdcModel,
    design: &DesignModel,
    files: &[SourceFile],
) -> FixPlan {
    let core = core.into();
    let universe = design.clock_universe(&sdc.created_clocks);

    // --- Lint A: safe phantom-group removal, per SDC file ---
    let mut fixed_files: Vec<FixedFile> = Vec::new();
    for f in files.iter().filter(|f| f.is_sdc()) {
        let (fixed, removed) = remove_phantom_groups(&f.text, &universe);
        if !removed.is_empty() {
            fixed_files.push(FixedFile {
                path: f.path.clone(),
                original: f.text.clone(),
                fixed,
                removed_groups: removed,
                suggestions_appended: 0,
            });
        }
    }

    // --- Lint B: UNVERIFIED guided suggestions (never a bare value) ---
    let mut suggestions: Vec<CrossingSuggestion> = Vec::new();
    let fires_b =
        sdc.blanket_async() && design.has_external_crossing() && sdc.added_timing().is_empty();
    if fires_b {
        for mc in design.memory.iter().filter(|m| m.kind.is_crossing()) {
            let crossing = format!("core \u{2194} external memory ({:?})", mc.kind);
            suggestions.push(CrossingSuggestion {
                block: suggestion_block(&crossing, &mc.evidence),
                crossing,
            });
        }
    }

    // Append the suggestion block(s) to the file that carries the blanket cut. If
    // that file also had a phantom removal, extend its FixedFile; otherwise create one.
    let mut notes = Vec::new();
    if !suggestions.is_empty() {
        let target = sdc.first_async_file();
        match target {
            Some(path) => {
                let appended: String = suggestions.iter().map(|s| s.block.clone()).collect();
                if let Some(ff) = fixed_files.iter_mut().find(|f| f.path == path) {
                    ff.fixed.push_str(&appended);
                    ff.suggestions_appended = suggestions.len();
                } else if let Some(orig) = files.iter().find(|f| f.path == path) {
                    let mut fixed = orig.text.clone();
                    fixed.push_str(&appended);
                    fixed_files.push(FixedFile {
                        path: path.clone(),
                        original: orig.text.clone(),
                        fixed,
                        removed_groups: Vec::new(),
                        suggestions_appended: suggestions.len(),
                    });
                }
            }
            None => notes.push(
                "a crossing suggestion was produced but no blanket-async SDC file was found to \
                 append it to; see the suggestion list"
                    .to_string(),
            ),
        }
    }

    if fixed_files.is_empty() && suggestions.is_empty() {
        notes.push("nothing to fix: no phantom groups and no unconstrained crossing".to_string());
    }
    notes.push(
        "heuristic fixer: phantom removal is safe (a no-op line deleted); crossing suggestions are \
         UNVERIFIED templates — you must set the value and confirm in static timing analysis"
            .to_string(),
    );

    FixPlan { core, fixed_files, suggestions, notes }
}

/// Convenience: build the models from a source and produce the fix plan.
pub fn plan_fix(core: impl Into<String>, source: &dyn CoreSource) -> FixPlan {
    let files = source.files();
    let sdc = crate::sdc::parse_sdc(&files);
    let design = crate::design::scan_design(&files);
    plan_fix_models(core, &sdc, &design, &files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn m72_like() -> MemSource {
        // blanket async cut naming two phantom PLLs (video_pll, sdram_pll) that ship
        // nowhere, plus real core_pll + audio_pll (mf_audio_pll ships). Author added
        // timing so Lint B must NOT fire here.
        MemSource::new()
            .with(
                "core.sdc",
                "set_clock_groups -asynchronous \\\n  \
                 -group {ic|core_pll|inst clk} \\\n  \
                 -group {ic|audio_pll|inst clk} \\\n  \
                 -group {ic|video_pll|video_pll_inst clk} \\\n  \
                 -group {ic|sdram_pll|inst clk}\nset_false_path -from a -to b\n",
            )
            .with("rtl/core_pll.v", "module core_pll(); endmodule")
            .with("rtl/mf_audio_pll.v", "module mf_audio_pll(); endmodule")
            .with("rtl/sdram_4w.v", "module sdram_4w(); endmodule")
    }

    #[test]
    fn phantom_fix_removes_only_dead_members_and_is_otherwise_identical() {
        let plan = plan_fix("m72", &m72_like());
        assert!(plan.has_safe_fixes());
        let ff = plan.fixed_files.iter().find(|f| f.path == "core.sdc").unwrap();
        // two dead members removed: video_pll and sdram_pll
        assert_eq!(ff.removed_groups.len(), 2);
        assert!(ff.fixed.contains("core_pll"), "real clock kept");
        assert!(ff.fixed.contains("audio_pll"), "real clock kept");
        assert!(!ff.fixed.contains("video_pll"), "phantom removed");
        assert!(!ff.fixed.contains("sdram_pll"), "phantom removed");
        // the unrelated line is byte-identical
        assert!(ff.fixed.contains("set_false_path -from a -to b"));
        // the fixed statement is still valid: header + 2 groups, last has no '\'
        let stmt: Vec<&str> = ff.fixed.lines().filter(|l| l.contains("group") || l.contains("set_clock_groups")).collect();
        assert!(!stmt.last().unwrap().trim_end().ends_with('\\'), "last group line has no continuation");
    }

    #[test]
    fn phantom_fix_result_reparses_with_no_phantoms() {
        let plan = plan_fix("m72", &m72_like());
        let ff = plan.fixed_files.iter().find(|f| f.path == "core.sdc").unwrap();
        // feed the corrected SDC back through the parser + the same design → no phantom tokens
        let files = m72_like().files();
        let design = crate::design::scan_design(&files);
        let corrected = crate::sdc::parse_sdc(&[SourceFile::new("core.sdc", ff.fixed.clone())]);
        let universe = design.clock_universe(&corrected.created_clocks);
        for t in corrected.async_group_pll_tokens() {
            assert!(universe.contains(&t), "no phantom token remains: {t}");
        }
    }

    #[test]
    fn crossing_gets_unverified_suggestion_never_a_bare_number() {
        // external memory + blanket cut + NO added timing → Lint B → guided suggestion
        let src = MemSource::new()
            .with("core.sdc", "set_clock_groups -asynchronous -group {core_pll}\n")
            .with("rtl/core_pll.v", "module core_pll(); endmodule")
            .with("rtl/sdram.v", "module sdram(); endmodule");
        let plan = plan_fix("mem", &src);
        assert_eq!(plan.suggestions.len(), 1);
        let block = &plan.suggestions[0].block;
        assert!(block.contains(UNVERIFIED_BANNER), "banner present");
        assert!(block.contains("<N>"), "placeholder, not a bare number");
        assert!(block.contains("<CORE_CLK>") && block.contains("<MEM_CLK>"));
        // every suggested constraint line is commented out (never applied blind)
        for line in block.lines().filter(|l| l.contains("set_multicycle_path") || l.contains("set_false_path")) {
            assert!(line.trim_start().starts_with('#'), "suggestion line is commented: {line}");
        }
        // and the appended file keeps the user's original content intact above the block
        let ff = plan.fixed_files.iter().find(|f| f.path == "core.sdc").unwrap();
        assert!(ff.fixed.starts_with(&ff.original), "user SDC untouched; suggestion only appended");
        assert_eq!(ff.suggestions_appended, 1);
    }

    #[test]
    fn bram_only_core_gets_no_suggestion() {
        let src = MemSource::new()
            .with("core.sdc", "set_clock_groups -asynchronous -group {core_pll}\n")
            .with("rtl/core_pll.v", "module core_pll(); endmodule");
        let plan = plan_fix("bram", &src);
        assert!(plan.suggestions.is_empty(), "no crossing → no suggestion");
        assert!(!plan.has_safe_fixes());
    }

    #[test]
    fn retimed_core_gets_no_suggestion() {
        let src = MemSource::new()
            .with(
                "core.sdc",
                "set_clock_groups -asynchronous -group {core_pll}\nset_multicycle_path -from a -to b 2\n",
            )
            .with("rtl/core_pll.v", "module core_pll(); endmodule")
            .with("rtl/sdram.v", "module sdram(); endmodule");
        let plan = plan_fix("retimed", &src);
        assert!(plan.suggestions.is_empty(), "author already added timing → no suggestion");
    }
}
