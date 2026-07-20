//! Rendering [`CoreReport`]s: structured JSON (built to be ingested as a downstream
//! tool's "CDC hotspots") and a human-readable text form.

use crate::lint::{CoreReport, Finding, Lint, Severity};
use serde::Serialize;

/// The crate name emitted in JSON, so a consumer knows the producer.
pub const TOOL: &str = "cdc-sentinel";
/// The crate version emitted in JSON.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
struct JsonEnvelope<'a> {
    tool: &'static str,
    version: &'static str,
    #[serde(flatten)]
    report: &'a CoreReport,
}

/// One core's report as pretty JSON.
pub fn to_json(report: &CoreReport) -> String {
    let env = JsonEnvelope { tool: TOOL, version: VERSION, report };
    serde_json::to_string_pretty(&env).expect("CoreReport serializes")
}

/// A whole corpus run as a pretty JSON array.
pub fn corpus_json(reports: &[CoreReport]) -> String {
    let envs: Vec<JsonEnvelope> = reports
        .iter()
        .map(|report| JsonEnvelope { tool: TOOL, version: VERSION, report })
        .collect();
    serde_json::to_string_pretty(&envs).expect("reports serialize")
}

fn severity_tag(s: Severity) -> &'static str {
    match s {
        Severity::High => "HIGH ",
        Severity::Warning => "WARN ",
    }
}

fn lint_tag(l: Lint) -> &'static str {
    match l {
        Lint::A => "lint-A",
        Lint::B => "lint-B",
    }
}

fn render_finding(out: &mut String, f: &Finding) {
    out.push_str(&format!("  [{}] {} ({})\n", severity_tag(f.severity), lint_tag(f.lint), f.id));
    out.push_str(&format!("    subject : {}\n", f.subject));
    out.push_str(&format!("    where   : {}\n", f.evidence));
    out.push_str(&format!("    why     : {}\n", f.reason));
    out.push_str(&format!("    fix     : {}\n", f.fix_hint));
    out.push_str(&format!("    ({:?})\n", f.confidence));
}

/// One core's report as human-readable text.
pub fn to_human(report: &CoreReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("=== {} ===\n", report.core));
    let s = &report.summary;
    out.push_str(&format!(
        "  blanket_async={}  external_memory={}  added_timing={}\n",
        s.blanket_async,
        s.external_memory,
        if s.added_timing.is_empty() {
            "none".to_string()
        } else {
            s.added_timing.iter().map(|k| k.command()).collect::<Vec<_>>().join(",")
        },
    ));
    out.push_str(&format!(
        "  pll_modules=[{}]\n",
        s.pll_modules.join(", "),
    ));
    if report.findings.is_empty() {
        out.push_str("  no findings\n");
    } else {
        for f in &report.findings {
            render_finding(&mut out, f);
        }
    }
    for l in &report.limits {
        out.push_str(&format!("  limit: {l}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze;
    use crate::source::MemSource;

    #[test]
    fn json_has_tool_and_findings() {
        let src = MemSource::new()
            .with("c.sdc", "set_clock_groups -asynchronous -group {video_pll}")
            .with("rtl/core_pll.v", "")
            .with("rtl/sdram.v", "");
        let rep = analyze("t", &src);
        let js = to_json(&rep);
        assert!(js.contains("\"tool\": \"cdc-sentinel\""));
        assert!(js.contains("cdc-b-unconstrained-crossing"));
        assert!(js.contains("cdc-a-phantom-clock-group"));
        // round-trips as valid JSON
        let v: serde_json::Value = serde_json::from_str(&js).unwrap();
        assert_eq!(v["tool"], "cdc-sentinel");
    }

    #[test]
    fn human_renders_no_findings() {
        let src = MemSource::new()
            .with("c.sdc", "set_clock_groups -asynchronous -group {core_pll}")
            .with("rtl/core_pll.v", "");
        let rep = analyze("bram", &src);
        let h = to_human(&rep);
        assert!(h.contains("no findings"));
    }
}
