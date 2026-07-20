//! Topology-aware clock-domain-crossing lints for FPGA cores.
//!
//! Analogue-Pocket cores inherit a blanket `set_clock_groups -asynchronous` from
//! their build template — one line telling static timing analysis to ignore every
//! clock crossing in the design. That is correct for a single-clock-domain core and
//! silently wrong for one that genuinely crosses into an external-memory clock with
//! no datapath timing added back. cdc-sentinel reads a core's `.sdc` plus a
//! lightweight structural scan of its RTL and flags the two failure modes:
//!
//! - **Lint A** — a dead / phantom clock-group member (a clock named in a group
//!   that resolves to no instantiated PLL or `create_clock`).
//! - **Lint B** — an unconstrained real crossing (an external-memory core under a
//!   blanket async cut with no added datapath timing). Crossing-aware: a BRAM-only
//!   single-clock core is never flagged.
//!
//! It **detects and explains only** — it never rewrites constraints.
//!
//! # Design
//!
//! Parsing produces two typed models — an [`sdc::SdcModel`] and a
//! [`design::DesignModel`] — and the lints are pure functions of them, so they are
//! unit-tested by building models directly. File access sits behind the
//! [`source::CoreSource`] seam, with a filesystem backend and an in-memory double.
//!
//! ```
//! use cdc_sentinel::{analyze, source::MemSource, lint::Lint};
//!
//! // an external-memory core under a blanket async cut with no added timing
//! let core = MemSource::new()
//!     .with("core_constraints.sdc", "set_clock_groups -asynchronous -group {core_pll}")
//!     .with("rtl/core_pll.v", "module core_pll(); endmodule")
//!     .with("rtl/sdram.v", "module sdram(); endmodule");
//!
//! let report = analyze("demo", &core);
//! assert!(report.fired(Lint::B));
//! ```
//!
//! # Scope and honesty
//!
//! Both lints run over a heuristic regex/token scan of SDC and RTL text, **not** an
//! elaborated netlist. Findings carry a confidence and the report records its
//! false-positive/negative limits. This ports detection logic that was validated
//! across a survey of 31 public cores; the committed fixture corpus reproduces that
//! survey's classifications.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod design;
pub mod error;
pub mod fix;
pub mod lint;
pub mod patch;
pub mod report;
pub mod sdc;
pub mod source;

pub use design::{scan_design, DesignModel, MemController, MemKind};
pub use error::{Error, Result};
pub use fix::{plan_fix, plan_fix_models, CrossingSuggestion, FixPlan, FixedFile};
pub use lint::{analyze, analyze_models, CoreReport, Finding, Lint, Severity, Summary};
pub use patch::{emit_template_patch, TemplatePatch};
pub use sdc::{parse_sdc, DatapathKind, SdcModel};
pub use source::{CoreSource, FsSource, MemSource, SourceFile};
