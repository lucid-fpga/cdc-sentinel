//! The single upstream fix. Every Pocket core inherits its blanket
//! `set_clock_groups -asynchronous` from one root — the official openFPGA
//! core-template (root commit `ad20a21`). Patching that one file inoculates every
//! future core against the STA-blind trap.
//!
//! This module **emits a proposed patch + PR body** for a human to review and
//! submit. cdc-sentinel does not open a pull request — submitting upstream is an
//! outward-facing action left to the maintainer.

/// The template file the patch targets.
pub const TEMPLATE_FILE: &str = "src/fpga/core/core_constraints.sdc";
/// The template's root commit where the blanket cut originates (public).
pub const ROOT_COMMIT: &str = "ad20a21";

/// A proposed change to the upstream template.
#[derive(Debug, Clone)]
pub struct TemplatePatch {
    /// A unified-diff-style patch (apply against the current template file).
    pub patch: String,
    /// A proposed pull-request body for the maintainer to review and submit.
    pub pr_body: String,
}

/// The safe-scaffold replacement for the blanket cut.
fn safe_scaffold() -> String {
    let mut s = String::new();
    s.push_str("# Clock groups: name the real clock domains explicitly. Do NOT blanket-cut\n");
    s.push_str("# every crossing as -asynchronous -- that tells static timing analysis to\n");
    s.push_str("# ignore EVERY clock crossing in the design, hiding real core<->memory\n");
    s.push_str("# crossings that need datapath timing.\n");
    s.push_str("#\n");
    s.push_str("# A single-clock-domain core needs nothing here.\n");
    s.push_str("#\n");
    s.push_str("# TODO(per-core): if this core crosses into an external-memory clock (SDRAM/\n");
    s.push_str("# PSRAM), add the datapath timing for that crossing here -- a\n");
    s.push_str("# set_multicycle_path or set_false_path scoped to the memory clock -- with the\n");
    s.push_str("# value confirmed in STA. Do not rely on a blanket -asynchronous cut.\n");
    s
}

/// Build the proposed template patch + PR body.
pub fn emit_template_patch() -> TemplatePatch {
    let scaffold = safe_scaffold();

    // A unified-diff-style hunk. The removed line is the public blanket-cut line the
    // template ships; the added lines are the safe scaffold. Line numbers are
    // illustrative -- apply against the current file.
    let mut patch = String::new();
    patch.push_str(&format!("--- a/{TEMPLATE_FILE}\n"));
    patch.push_str(&format!("+++ b/{TEMPLATE_FILE}\n"));
    patch.push_str("@@ clock constraints @@\n");
    patch.push_str("-set_clock_groups -asynchronous \\\n");
    patch.push_str("-  -group { <clkA> } \\\n");
    patch.push_str("-  -group { <clkB> } ...\n");
    for line in scaffold.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }

    let mut pr_body = String::new();
    pr_body.push_str("## Replace the blanket `set_clock_groups -asynchronous` in the core constraints\n\n");
    pr_body.push_str(&format!(
        "The template's `{TEMPLATE_FILE}` ships a blanket `set_clock_groups -asynchronous` \
         (originating at commit `{ROOT_COMMIT}`). Because every core is scaffolded from this \
         template, every core inherits it.\n\n"
    ));
    pr_body.push_str(
        "### Why this is a problem\n\n\
         A blanket `-asynchronous` cut tells static timing analysis to ignore **every** clock \
         crossing in the design. That is fine for a single-clock-domain core, but silently wrong \
         for a core that genuinely crosses into an external-memory clock (SDRAM/PSRAM) — the real \
         crossing is left with no datapath timing and STA never flags it. An analysis of 31 public \
         Pocket cores found this inherited cut is near-universal, and that several external-memory \
         cores carry it with no added timing.\n\n",
    );
    pr_body.push_str(
        "### The change\n\n\
         Replace the blanket cut with a documented, empty-by-default section plus a per-core TODO, \
         so a porter names their real domains and adds the specific datapath timing (confirmed in \
         STA) instead of inheriting a cut that hides the crossing. A single-clock-domain core needs \
         nothing here, so the common case is unaffected.\n\n",
    );
    pr_body.push_str(
        "> Note: the correct multicycle/false-path value is per-core and must be set and verified \
         in STA — this change intentionally does not prescribe a value.\n",
    );

    TemplatePatch { patch, pr_body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_replaces_blanket_cut_with_scaffold_and_todo() {
        let p = emit_template_patch();
        assert!(p.patch.contains(TEMPLATE_FILE));
        assert!(p.patch.contains("-set_clock_groups -asynchronous"), "removes the blanket cut");
        assert!(p.patch.contains("+# TODO(per-core)"), "adds the per-core TODO scaffold");
        assert!(!p.patch.contains("+set_clock_groups -asynchronous"), "does not re-add the cut");
    }

    #[test]
    fn pr_body_cites_the_public_root_and_prescribes_no_value() {
        let p = emit_template_patch();
        assert!(p.pr_body.contains(ROOT_COMMIT));
        assert!(p.pr_body.to_lowercase().contains("does not prescribe a value"));
        // public-only: the rationale cites public repos/analysis, no internal refs.
        // (The negative-vocabulary enforcement is the pre-commit leakguard's job,
        // run on every commit, so it is not re-embedded here.)
        assert!(p.pr_body.contains("public Pocket cores"), "cites the public evidence");
    }
}
