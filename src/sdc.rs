//! The SDC model and its parser (the first half of the seam).
//!
//! [`parse_sdc`] turns a core's `.sdc` files into a typed [`SdcModel`]: the clock
//! groups (with their `-asynchronous`/`-exclusive` kind and the `*_pll` clock
//! tokens each group names), the created clocks, and the datapath timing
//! constraints, each tagged with whether it lives in the "Do not edit" framework
//! file. The lints are pure functions of this model plus the [`DesignModel`], so
//! they are unit-tested by building models directly.
//!
//! This ports the survey's SDC-side heuristics: a token/regex scan of constraint
//! text, **not** an elaborated netlist. It reads presence, not coverage.
//!
//! [`DesignModel`]: crate::design::DesignModel

use crate::source::SourceFile;
use regex::Regex;
use serde::Serialize;

/// Intel/Altera PLL *primitive* names that appear inside every fully-qualified
/// group path (e.g. `...|cyclonev_pll:...`). They are not phantom clock domains and
/// are excluded from `*_pll` extraction — same exclusion the survey used.
pub const PLL_PRIMITIVES: &[&str] = &["cyclonev_pll", "altera_pll", "altera_pll_reconfig"];

/// The "Do not edit" template constraint file. Datapath timing found *here* is the
/// framework baseline, not author-added work, so it does not count toward Lint B's
/// "no added timing" test.
const FRAMEWORK_BASENAME: &str = "apf_constraints.sdc";

/// How a `set_clock_groups` relates its groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// `-asynchronous`: STA ignores all crossings between the groups (the blanket cut).
    Asynchronous,
    /// `-exclusive`: the groups are never active simultaneously.
    Exclusive,
    /// A `set_clock_groups` with neither flag.
    Other,
}

/// One `-group { ... }` inside a `set_clock_groups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupMember {
    /// The raw brace contents (trimmed, capped for evidence).
    pub raw: String,
    /// Distinct `*_pll` tokens named in this group (Altera primitives excluded).
    pub pll_tokens: Vec<String>,
}

/// A parsed `set_clock_groups` statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClockGroup {
    /// Source file (relative path) — used in evidence lines.
    pub file: String,
    /// `-asynchronous` / `-exclusive` / neither.
    pub kind: GroupKind,
    /// The `-group {}` members.
    pub members: Vec<GroupMember>,
}

/// A kind of datapath timing constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatapathKind {
    /// `set_multicycle_path`
    Multicycle,
    /// `set_false_path`
    FalsePath,
    /// `set_input_delay`
    InputDelay,
    /// `set_output_delay`
    OutputDelay,
}

impl DatapathKind {
    /// The SDC command that produces this kind.
    pub fn command(self) -> &'static str {
        match self {
            DatapathKind::Multicycle => "set_multicycle_path",
            DatapathKind::FalsePath => "set_false_path",
            DatapathKind::InputDelay => "set_input_delay",
            DatapathKind::OutputDelay => "set_output_delay",
        }
    }
}

/// A datapath-timing constraint occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatapathConstraint {
    /// Which command.
    pub kind: DatapathKind,
    /// Source file.
    pub file: String,
    /// True if the file is the "Do not edit" framework template (baseline, not
    /// author-added).
    pub framework: bool,
}

/// The parsed timing constraints of a core.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SdcModel {
    /// Every `set_clock_groups` found.
    pub clock_groups: Vec<ClockGroup>,
    /// `create_clock`/`create_generated_clock` names (the design's declared clocks).
    pub created_clocks: Vec<String>,
    /// Datapath-timing constraints found.
    pub datapath: Vec<DatapathConstraint>,
}

impl SdcModel {
    /// True if any group is a blanket `-asynchronous` cut.
    pub fn blanket_async(&self) -> bool {
        self.clock_groups.iter().any(|g| g.kind == GroupKind::Asynchronous)
    }

    /// Distinct `*_pll` tokens named inside `-asynchronous` groups, sorted.
    pub fn async_group_pll_tokens(&self) -> Vec<String> {
        let mut set: Vec<String> = Vec::new();
        for g in &self.clock_groups {
            if g.kind != GroupKind::Asynchronous {
                continue;
            }
            for m in &g.members {
                for t in &m.pll_tokens {
                    if !set.contains(t) {
                        set.push(t.clone());
                    }
                }
            }
        }
        set.sort();
        set
    }

    /// The source file of the first `-asynchronous` group, for Lint B evidence.
    pub fn first_async_file(&self) -> Option<String> {
        self.clock_groups
            .iter()
            .find(|g| g.kind == GroupKind::Asynchronous)
            .map(|g| g.file.clone())
    }

    /// The first evidence line (file + group excerpt) naming `token`, if any.
    pub fn evidence_for(&self, token: &str) -> Option<String> {
        for g in &self.clock_groups {
            for m in &g.members {
                if m.pll_tokens.iter().any(|t| t == token) {
                    let excerpt: String = m.raw.chars().take(70).collect();
                    return Some(format!("{}: -group {{{excerpt}...}}", g.file));
                }
            }
        }
        None
    }

    /// Datapath kinds added by the author (i.e. not in the framework file), sorted
    /// and de-duplicated. Empty ⇒ "no added timing" (Lint B's necessary condition).
    pub fn added_timing(&self) -> Vec<DatapathKind> {
        let mut v: Vec<DatapathKind> =
            self.datapath.iter().filter(|d| !d.framework).map(|d| d.kind).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// True if `path`/`text` is the "Do not edit" framework template file.
fn is_framework(path: &str, text: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if base == FRAMEWORK_BASENAME {
        return true;
    }
    let head: String = text.chars().take(400).collect();
    head.to_ascii_lowercase().contains("do not edit")
}

/// Drop full-line `#` comments (first non-space char is `#`) and blank-fold, then
/// splice Tcl `\` line continuations so each statement is one logical line.
fn logical_lines(text: &str) -> Vec<String> {
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        kept.push(line);
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in kept {
        if let Some(stripped) = line.strip_suffix('\\') {
            cur.push_str(stripped);
            cur.push(' ');
        } else {
            cur.push_str(line);
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Extract the distinct non-primitive `*_pll` tokens from a group's brace body,
/// preserving first-seen order.
fn pll_tokens(pll_re: &Regex, body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for cap in pll_re.captures_iter(body) {
        let tok = cap[1].to_ascii_lowercase();
        if PLL_PRIMITIVES.contains(&tok.as_str()) {
            continue;
        }
        if !out.contains(&tok) {
            out.push(tok);
        }
    }
    out
}

/// Parse a core's SDC files into an [`SdcModel`]. Non-SDC files are ignored.
pub fn parse_sdc(files: &[SourceFile]) -> SdcModel {
    let pll_re = Regex::new(r"(?i)\b([a-z0-9]*_pll)\b").unwrap();
    let group_re = Regex::new(r"(?s)-group\s*\{([^}]*)\}").unwrap();
    let name_re = Regex::new(r"-name\s+([A-Za-z_][\w$]*)").unwrap();

    let mut model = SdcModel::default();

    for f in files.iter().filter(|f| f.is_sdc()) {
        let framework = is_framework(&f.path, &f.text);
        for stmt in logical_lines(&f.text) {
            let low = stmt.to_ascii_lowercase();

            if low.contains("set_clock_groups") {
                let kind = if low.contains("-asynchronous") {
                    GroupKind::Asynchronous
                } else if low.contains("-exclusive") {
                    GroupKind::Exclusive
                } else {
                    GroupKind::Other
                };
                let members = group_re
                    .captures_iter(&stmt)
                    .map(|c| {
                        let raw = c[1].trim().to_string();
                        let pll_tokens = pll_tokens(&pll_re, &raw);
                        GroupMember { raw, pll_tokens }
                    })
                    .collect();
                model.clock_groups.push(ClockGroup { file: f.path.clone(), kind, members });
            }

            if low.contains("create_clock") || low.contains("create_generated_clock") {
                if let Some(c) = name_re.captures(&stmt) {
                    let name = c[1].to_string();
                    if !model.created_clocks.contains(&name) {
                        model.created_clocks.push(name);
                    }
                }
            }

            for kind in [
                DatapathKind::Multicycle,
                DatapathKind::FalsePath,
                DatapathKind::InputDelay,
                DatapathKind::OutputDelay,
            ] {
                if low.contains(kind.command()) {
                    model.datapath.push(DatapathConstraint {
                        kind,
                        file: f.path.clone(),
                        framework,
                    });
                }
            }
        }
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use crate::source::CoreSource;

    fn parse(files: &MemSource) -> SdcModel {
        parse_sdc(&files.files())
    }

    #[test]
    fn parses_blanket_async_and_pll_tokens() {
        let src = MemSource::new().with(
            "core_constraints.sdc",
            "set_clock_groups -asynchronous \\\n \
             -group {ic|core_pll|core_pll_inst|altera_pll_i clk} \\\n \
             -group {ic|video_pll|video_pll_inst|cyclonev_pll clk}\n",
        );
        let m = parse(&src);
        assert!(m.blanket_async());
        assert_eq!(m.clock_groups.len(), 1);
        assert_eq!(m.clock_groups[0].kind, GroupKind::Asynchronous);
        // cyclonev_pll excluded; core_pll + video_pll kept
        assert_eq!(m.async_group_pll_tokens(), vec!["core_pll", "video_pll"]);
    }

    #[test]
    fn excludes_altera_primitives() {
        let src = MemSource::new()
            .with("c.sdc", "set_clock_groups -asynchronous -group {altera_pll_reconfig|x cyclonev_pll}");
        let m = parse(&src);
        assert!(m.async_group_pll_tokens().is_empty());
    }

    #[test]
    fn framework_timing_is_not_author_added() {
        let src = MemSource::new()
            .with(
                "apf_constraints.sdc",
                "# Do not edit\nset_false_path -from [get_clocks a] -to [get_clocks b]",
            )
            .with("core.sdc", "set_multicycle_path -from [get_clocks c] -to [get_clocks c] 2");
        let m = parse(&src);
        // framework false_path present but excluded; only the author multicycle counts
        assert_eq!(m.added_timing(), vec![DatapathKind::Multicycle]);
    }

    #[test]
    fn header_do_not_edit_marks_framework() {
        let src = MemSource::new().with(
            "sys_constr.sdc",
            "## AUTO-GENERATED — Do not edit this file\nset_input_delay -clock c 2.0 [get_ports x]",
        );
        let m = parse(&src);
        assert!(m.added_timing().is_empty(), "do-not-edit header should mark framework");
    }

    #[test]
    fn commented_constraint_is_ignored() {
        let src = MemSource::new()
            .with("c.sdc", "# set_clock_groups -asynchronous -group {core_pll}\ncreate_clock -name clk_74a -period 13.4 [get_ports clk]");
        let m = parse(&src);
        assert!(!m.blanket_async());
        assert_eq!(m.created_clocks, vec!["clk_74a"]);
    }
}
