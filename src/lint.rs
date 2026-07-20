//! The lint engine: two pure functions over the parsed [`SdcModel`] +
//! [`DesignModel`], plus the [`analyze`] orchestration that builds both models from
//! a [`CoreSource`] and runs them. Detect-and-explain only — no constraint is ever
//! rewritten.

use crate::design::{scan_design, DesignModel, MemController};
use crate::sdc::{parse_sdc, DatapathKind, SdcModel};
use crate::source::CoreSource;
use serde::Serialize;

/// How much silicon risk a finding carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A real, unconstrained clock crossing — a genuine timing hazard.
    High,
    /// A dead/misleading constraint — a correctness smell, not itself a hazard.
    Warning,
}

/// Which lint produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Lint {
    /// Lint A — dead / phantom clock-group member.
    A,
    /// Lint B — unconstrained real crossing.
    B,
}

impl Lint {
    /// The stable machine id for this lint.
    pub fn id(self) -> &'static str {
        match self {
            Lint::A => "cdc-a-phantom-clock-group",
            Lint::B => "cdc-b-unconstrained-crossing",
        }
    }
}

/// Whether a finding's core claim is read verbatim from the files (observed) or
/// derived by heuristic (inferred). The lints are honest about which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Read directly from a cited file.
    Observed,
    /// Derived heuristically; may false-positive/negative (see the limits note).
    Inferred,
}

/// A single lint finding, shaped to serialize as a downstream tool's "CDC hotspot".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Which lint.
    pub lint: Lint,
    /// Stable machine id.
    pub id: &'static str,
    /// Severity.
    pub severity: Severity,
    /// The offending clock (Lint A) or the crossing (Lint B).
    pub subject: String,
    /// A file + excerpt pinning the finding to a source line.
    pub evidence: String,
    /// Why it is a problem, in one sentence.
    pub reason: String,
    /// The constraint a fixer would write (pointer to the eventual fix).
    pub fix_hint: String,
    /// Observed vs inferred.
    pub confidence: Confidence,
}

/// Lint A — dead / phantom clock-group members. A `*_pll` token named inside a
/// blanket `-asynchronous` group that resolves to no shipped PLL module and no
/// `create_clock` is the mechanical fingerprint of a multi-PLL group template
/// inherited onto a core that never instantiated those PLLs.
pub fn lint_phantom(sdc: &SdcModel, design: &DesignModel) -> Vec<Finding> {
    let universe = design.clock_universe(&sdc.created_clocks);
    let mut out = Vec::new();
    for token in sdc.async_group_pll_tokens() {
        if universe.contains(&token) {
            continue;
        }
        let evidence = sdc
            .evidence_for(&token)
            .unwrap_or_else(|| format!("clock group naming `{token}`"));
        out.push(Finding {
            lint: Lint::A,
            id: Lint::A.id(),
            severity: Severity::Warning,
            subject: token.clone(),
            evidence,
            reason: format!(
                "clock `{token}` is named in a set_clock_groups but resolves to no \
                 instantiated PLL output and no create_clock — a dead group member \
                 (an un-pruned multi-PLL group template inherited onto this core)"
            ),
            fix_hint: format!(
                "remove the dead `{token}` group member, or instantiate the PLL it \
                 assumes; a future cdc-sentinel version will emit the pruned group"
            ),
            confidence: Confidence::Inferred,
        });
    }
    out
}

/// Lint B — an unconstrained real crossing. The core ships an external-memory
/// controller (so a genuine core↔memory clock crossing exists), sits under a
/// blanket `-asynchronous` cut, and adds **no** datapath timing on any path. A
/// BRAM-only single-clock core is never flagged: it has no crossing to constrain.
pub fn lint_crossing(sdc: &SdcModel, design: &DesignModel) -> Vec<Finding> {
    if !sdc.blanket_async() {
        return Vec::new();
    }
    if !design.has_external_crossing() {
        return Vec::new();
    }
    if !sdc.added_timing().is_empty() {
        return Vec::new();
    }
    let MemController { kind, evidence } = match design.crossing_evidence() {
        Some(c) => c.clone(),
        None => return Vec::new(),
    };
    let async_file = sdc.first_async_file().unwrap_or_else(|| "(clock-groups sdc)".into());
    vec![Finding {
        lint: Lint::B,
        id: Lint::B.id(),
        severity: Severity::High,
        subject: format!("core \u{2194} external memory ({kind:?})"),
        evidence: format!("{async_file}: set_clock_groups -asynchronous (blanket); controller {evidence}"),
        reason: format!(
            "an external-memory ({kind:?}) crossing is left under a blanket \
             -asynchronous cut with NO added multicycle / false-path / set_*_delay \
             on any path — static timing analysis is blind to a real crossing"
        ),
        fix_hint: "add datapath timing covering the core\u{2194}memory crossing (a \
                   set_multicycle_path or set_false_path scoped to the SDRAM clock); \
                   a future cdc-sentinel version will emit the constraint"
            .to_string(),
        confidence: Confidence::Inferred,
    }]
}

/// The at-a-glance structural facts behind a core's findings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// A blanket `set_clock_groups -asynchronous` is present.
    pub blanket_async: bool,
    /// An external-memory crossing (SDRAM/PSRAM) is present.
    pub external_memory: bool,
    /// Datapath timing kinds the author added (empty ⇒ none).
    pub added_timing: Vec<DatapathKind>,
    /// PLL-module tokens the core ships/declares.
    pub pll_modules: Vec<String>,
    /// The full clock-name universe used for phantom resolution.
    pub clock_universe: Vec<String>,
    /// External-memory controllers detected.
    pub memory: Vec<MemController>,
}

/// A core's complete cdc-sentinel report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoreReport {
    /// The core's name/path (as passed to [`analyze`]).
    pub core: String,
    /// Structural summary.
    pub summary: Summary,
    /// The findings, Lint B (high) before Lint A (warning).
    pub findings: Vec<Finding>,
    /// The scan's honest limits for this core.
    pub limits: Vec<String>,
}

impl CoreReport {
    /// True if any finding fired.
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    /// True if a finding of the given lint fired.
    pub fn fired(&self, lint: Lint) -> bool {
        self.findings.iter().any(|f| f.lint == lint)
    }
}

/// Build both models from a source and run both lints. `core` labels the report.
pub fn analyze(core: impl Into<String>, source: &dyn CoreSource) -> CoreReport {
    let files = source.files();
    let sdc = parse_sdc(&files);
    let design = scan_design(&files);
    analyze_models(core, &sdc, &design)
}

/// Run the lints over already-parsed models — the pure seam the unit tests drive.
pub fn analyze_models(core: impl Into<String>, sdc: &SdcModel, design: &DesignModel) -> CoreReport {
    let mut findings = lint_crossing(sdc, design);
    findings.extend(lint_phantom(sdc, design));

    let mut limits = Vec::new();
    if design.rtl_files_scanned == 0 {
        limits.push(
            "no RTL scanned — PLL/memory topology unknown; phantom and crossing \
             results are unreliable for this core"
                .to_string(),
        );
    }
    limits.push(
        "heuristic text scan, not an elaborated netlist: a PLL built by an unseen \
         .qip/.ip is missed, and 'added timing' is detected by presence, not by \
         whether it actually covers the specific crossing"
            .to_string(),
    );

    let summary = Summary {
        blanket_async: sdc.blanket_async(),
        external_memory: design.has_external_crossing(),
        added_timing: sdc.added_timing(),
        pll_modules: design.pll_modules.clone(),
        clock_universe: design.clock_universe(&sdc.created_clocks),
        memory: design.memory.clone(),
    };

    CoreReport { core: core.into(), summary, findings, limits }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdc::parse_sdc;
    use crate::source::{CoreSource, MemSource};

    // --- in-memory-double unit tests: build models directly, assert the lints ---

    fn models(src: &MemSource) -> (SdcModel, DesignModel) {
        let files = src.files();
        (parse_sdc(&files), scan_design(&files))
    }

    #[test]
    fn phantom_group_fires_lint_a() {
        // groups video_pll + sdram_pll, but only core_pll is shipped
        let src = MemSource::new()
            .with(
                "core_constraints.sdc",
                "set_clock_groups -asynchronous \
                 -group {ic|core_pll|inst} -group {ic|video_pll|inst} -group {ic|sdram_pll|inst}",
            )
            .with("rtl/core_pll.v", "module core_pll(); endmodule");
        let (sdc, design) = models(&src);
        let a = lint_phantom(&sdc, &design);
        let subjects: Vec<_> = a.iter().map(|f| f.subject.as_str()).collect();
        assert!(subjects.contains(&"video_pll"));
        assert!(subjects.contains(&"sdram_pll"));
        assert!(!subjects.contains(&"core_pll"), "core_pll is shipped → not phantom");
        assert!(a.iter().all(|f| f.severity == Severity::Warning));
    }

    #[test]
    fn all_resolving_groups_do_not_fire_lint_a() {
        let src = MemSource::new()
            .with(
                "c.sdc",
                "set_clock_groups -asynchronous -group {core_pll} -group {sdram_pll} -group {video_pll}",
            )
            .with("rtl/core_pll.v", "")
            .with("rtl/sdram_pll.v", "")
            .with("rtl/video_pll.v", "");
        let (sdc, design) = models(&src);
        assert!(lint_phantom(&sdc, &design).is_empty());
    }

    #[test]
    fn external_mem_no_timing_fires_lint_b() {
        let src = MemSource::new()
            .with("c.sdc", "set_clock_groups -asynchronous -group {core_pll}")
            .with("rtl/core_pll.v", "")
            .with("rtl/sdram.v", "module sdram(); endmodule");
        let (sdc, design) = models(&src);
        let b = lint_crossing(&sdc, &design);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].severity, Severity::High);
        assert_eq!(b[0].lint, Lint::B);
    }

    #[test]
    fn bram_only_core_does_not_fire_lint_b() {
        // the correctness bar: single-clock BRAM-only core, blanket cut, no timing → NOT flagged
        let src = MemSource::new()
            .with("c.sdc", "set_clock_groups -asynchronous -group {core_pll}")
            .with("rtl/core_pll.v", "");
        let (sdc, design) = models(&src);
        assert!(lint_crossing(&sdc, &design).is_empty());
    }

    #[test]
    fn retimed_core_does_not_fire_lint_b() {
        // external memory present, but the author re-added timing → NOT flagged
        let src = MemSource::new()
            .with(
                "c.sdc",
                "set_clock_groups -asynchronous -group {core_pll}\n\
                 set_multicycle_path -from [get_clocks a] -to [get_clocks b] 2",
            )
            .with("rtl/core_pll.v", "")
            .with("rtl/sdram.v", "");
        let (sdc, design) = models(&src);
        assert!(lint_crossing(&sdc, &design).is_empty());
    }

    #[test]
    fn framework_timing_does_not_suppress_lint_b() {
        // baseline false_path lives only in the Do-not-edit framework file → still fires
        let src = MemSource::new()
            .with("apf_constraints.sdc", "# Do not edit\nset_false_path -from a -to b")
            .with("core.sdc", "set_clock_groups -asynchronous -group {core_pll}")
            .with("rtl/core_pll.v", "")
            .with("rtl/sdram.v", "");
        let (sdc, design) = models(&src);
        assert_eq!(lint_crossing(&sdc, &design).len(), 1, "framework timing must not count as added");
    }

    #[test]
    fn analyze_orders_high_before_warning() {
        // a core that fires BOTH lints: report lists Lint B (high) first
        let src = MemSource::new()
            .with(
                "c.sdc",
                "set_clock_groups -asynchronous -group {core_pll} -group {video_pll}",
            )
            .with("rtl/core_pll.v", "")
            .with("rtl/sdram.v", "");
        let rep = analyze("dual", &src);
        assert!(rep.fired(Lint::A) && rep.fired(Lint::B));
        assert_eq!(rep.findings[0].lint, Lint::B, "high severity first");
    }
}
