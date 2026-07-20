//! The design model and its RTL structural scan (the second half of the seam).
//!
//! [`scan_design`] reads a core's RTL and derives two things the lints need:
//!
//! 1. the **clock-name universe** — which PLL modules the core actually ships /
//!    declares, so Lint A can tell a real clock from a phantom group member; and
//! 2. the **memory topology** — whether the core ships an external-memory
//!    controller (SDRAM/PSRAM/DDR), so Lint B can gate on a genuine cross-domain
//!    crossing existing.
//!
//! This ports the survey's PLL-module heuristic and its memory-controller
//! heuristic. Both are filename/text scans, not an elaborated netlist:
//!
//! - **Under-report:** a PLL built by a `.qip`/`.ip` we cannot see, or a controller
//!   in an un-scanned file, is missed.
//! - **Over-report:** a `*_pll` module declaration that is present but unused would
//!   still be counted as shipped.

use crate::source::SourceFile;
use regex::Regex;
use serde::Serialize;

/// The PLL *module* names a Pocket core instantiates (the Gateman / `mf_pllbase`
/// family). Matched as path/text substrings — so `mf_audio_pll` also yields the
/// `audio_pll` token, exactly as the survey's extractor did.
pub const PLL_MODULES: &[&str] =
    &["core_pll", "sdram_pll", "video_pll", "mf_audio_pll", "audio_pll", "mf_pllbase"];

/// A class of external-memory controller shipped by the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemKind {
    /// Multi-port SDRAM controller (`sdram_4w*`) — the arcade pattern.
    SdramMultiport,
    /// A plain SDRAM controller.
    Sdram,
    /// Pseudo-static RAM / cellular RAM (`psram`/`cellram`/`cram`).
    Psram,
    /// A DDR controller file. On the Pocket these are usually an unused MiSTer
    /// vestige, so DDR **alone** does not gate Lint B — recorded, not trusted.
    Ddr,
}

impl MemKind {
    /// Does this controller imply a real core↔memory clock crossing that Lint B
    /// should require timing for? SDRAM/PSRAM yes; a DDR vestige no.
    pub fn is_crossing(self) -> bool {
        matches!(self, MemKind::SdramMultiport | MemKind::Sdram | MemKind::Psram)
    }
}

/// A detected external-memory controller and the file that evidences it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemController {
    /// The controller class.
    pub kind: MemKind,
    /// The RTL file (relative path) it was found in.
    pub evidence: String,
}

/// The structural facts the RTL scan derives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DesignModel {
    /// Distinct PLL-module tokens the core ships or declares (sorted).
    pub pll_modules: Vec<String>,
    /// External-memory controllers detected (first evidence per kind).
    pub memory: Vec<MemController>,
    /// How many RTL files the scan read (a coverage indicator for the limits note).
    pub rtl_files_scanned: usize,
}

impl DesignModel {
    /// True if the core ships a controller implying a real memory clock crossing
    /// (SDRAM/PSRAM — not a bare DDR vestige). This is Lint B's topology gate.
    pub fn has_external_crossing(&self) -> bool {
        self.memory.iter().any(|m| m.kind.is_crossing())
    }

    /// The crossing that gates Lint B, for the finding's evidence, if any.
    pub fn crossing_evidence(&self) -> Option<&MemController> {
        self.memory.iter().find(|m| m.kind.is_crossing())
    }

    /// The full clock-name universe a group member must resolve into: the shipped
    /// PLL tokens plus any `create_clock` names from the SDC. Lowercased.
    pub fn clock_universe(&self, created_clocks: &[String]) -> Vec<String> {
        let mut u: Vec<String> = self.pll_modules.clone();
        for c in created_clocks {
            let lc = c.to_ascii_lowercase();
            if !u.contains(&lc) {
                u.push(lc);
            }
        }
        u
    }
}

/// Scan a core's RTL files into a [`DesignModel`]. Non-RTL files are ignored.
pub fn scan_design(files: &[SourceFile]) -> DesignModel {
    let module_re = Regex::new(r"(?i)\bmodule\s+([a-z0-9_]*_pll)\b").unwrap();

    let mut pll: Vec<String> = Vec::new();
    let mut push_pll = |tok: &str| {
        let t = tok.to_ascii_lowercase();
        if crate::sdc::PLL_PRIMITIVES.contains(&t.as_str()) {
            return;
        }
        if !pll.contains(&t) {
            pll.push(t);
        }
    };

    let mut memory: Vec<MemController> = Vec::new();
    let seen_kind = |mem: &mut Vec<MemController>, kind: MemKind, ev: &str| {
        if !mem.iter().any(|m| m.kind == kind) {
            mem.push(MemController { kind, evidence: ev.to_string() });
        }
    };

    let mut scanned = 0usize;
    for f in files.iter().filter(|f| f.is_rtl()) {
        scanned += 1;
        let path_lc = f.path.to_ascii_lowercase();

        // PLL modules from file paths (skip MiSTer `upstream/` vestige and the
        // `reconfig` helper IP, which is not itself a PLL) — ports the survey scan.
        if !(path_lc.contains("upstream") || path_lc.contains("reconfig")) {
            for tok in PLL_MODULES {
                if path_lc.contains(tok) {
                    push_pll(tok);
                }
            }
        }
        // PLL modules from `module <name>_pll` declarations (catches inline PLLs).
        for cap in module_re.captures_iter(&f.text) {
            push_pll(&cap[1]);
        }

        // Memory-controller topology — ports the survey's filename heuristic.
        let ext = f.ext();
        if ext == "v" || ext == "sv" {
            if path_lc.contains("sdram_4w") {
                seen_kind(&mut memory, MemKind::SdramMultiport, &f.path);
            } else if path_lc.contains("sdram") {
                seen_kind(&mut memory, MemKind::Sdram, &f.path);
            }
            if path_lc.contains("psram") || path_lc.contains("cellram") || path_lc.contains("cram")
            {
                seen_kind(&mut memory, MemKind::Psram, &f.path);
            }
            if path_lc.contains("ddram") || path_lc.contains("ddr3") {
                seen_kind(&mut memory, MemKind::Ddr, &f.path);
            }
        }
    }

    pll.sort();
    DesignModel { pll_modules: pll, memory, rtl_files_scanned: scanned }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{CoreSource, MemSource};

    fn scan(s: &MemSource) -> DesignModel {
        scan_design(&s.files())
    }

    #[test]
    fn audio_pll_file_yields_both_tokens() {
        // mf_audio_pll.v ships → both mf_audio_pll and audio_pll are in the universe
        let d = scan(&MemSource::new().with("rtl/mf_audio_pll.v", "module mf_audio_pll(); endmodule"));
        assert!(d.pll_modules.contains(&"mf_audio_pll".to_string()));
        assert!(d.pll_modules.contains(&"audio_pll".to_string()));
    }

    #[test]
    fn upstream_and_reconfig_excluded() {
        let d = scan(
            &MemSource::new()
                .with("rtl/upstream/core_pll.v", "")
                .with("rtl/sdram_pll_reconfig.v", ""),
        );
        assert!(d.pll_modules.is_empty(), "upstream/reconfig must not count as shipped PLLs");
    }

    #[test]
    fn detects_sdram_crossing_but_not_bare_ddr() {
        let sdram = scan(&MemSource::new().with("rtl/sdram.v", "module sdram(); endmodule"));
        assert!(sdram.has_external_crossing());
        let ddr_only = scan(&MemSource::new().with("rtl/ddram.v", "module ddram(); endmodule"));
        assert!(!ddr_only.has_external_crossing(), "a bare DDR vestige must not gate Lint B");
        assert_eq!(ddr_only.memory[0].kind, MemKind::Ddr);
    }

    #[test]
    fn bram_only_core_has_no_crossing() {
        let d = scan(&MemSource::new().with("rtl/core_pll.v", "module core_pll(); endmodule"));
        assert!(!d.has_external_crossing());
        assert!(d.pll_modules.contains(&"core_pll".to_string()));
    }

    #[test]
    fn multiport_sdram_recognised() {
        let d = scan(&MemSource::new().with("rtl/sdram_4w_ctrl.v", ""));
        assert_eq!(d.memory[0].kind, MemKind::SdramMultiport);
    }
}
