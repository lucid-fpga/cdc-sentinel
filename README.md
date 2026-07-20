# cdc-sentinel

Topology-aware clock-domain-crossing lints for FPGA cores. Analogue-Pocket cores
inherit a blanket `set_clock_groups -asynchronous` from their build template — a
single line that tells static timing analysis to **ignore every clock crossing in
the design**. That is correct for a core whose logic lives in one clock domain and
wrong — silently — for a core that genuinely crosses into an external-memory clock
with no datapath timing added back. cdc-sentinel reads a core's `.sdc` plus a
lightweight structural scan of its RTL and flags the two failure modes a survey of
31 public cores found hiding behind that cut.

It **detects and explains only** — it never rewrites your constraints. Each finding
names the offending clock or SDC line, says why it is a problem, and points at the
constraint the eventual fixer would write.

## Status

Early development, desk-tested only. The two lints, the SDC model + parser, and the
RTL structural scan are implemented and unit-tested against **in-memory source
doubles**, and validated end-to-end against a committed fixture corpus that
reproduces the classifications of the survey the lints were derived from. The scan
is a **heuristic text analysis, not an elaborated netlist** (see Design notes): it
reports confidence, not proof, and records its false-positive/negative limits. The
public API is not stable.

## What it lints

- **Lint A — dead / phantom clock-group member.** A clock named in a
  `set_clock_groups` that resolves to no instantiated PLL output and no
  `create_clock` in the design. The mechanical fingerprint of a multi-PLL group
  template inherited un-pruned onto a core that never instantiated those PLLs — a
  dead constraint that lies about the design's clock topology.
- **Lint B — unconstrained real crossing.** A core that ships an external-memory
  controller (SDRAM / PSRAM / DDR) — so a genuine core-to-memory clock crossing
  exists — left under a blanket `-asynchronous` group with **no** added
  `set_multicycle_path` / `set_false_path` / `set_input_delay` / `set_output_delay`
  on any path. **Crossing-aware:** a BRAM-only single-clock core with the same
  blanket cut and no added timing is **not** flagged — it has no crossing to
  constrain, so the cut is correct there. A noisy linter is a failed linter.

## Design notes

- **Heuristic, and honest about it.** Both lints run over a regex/token scan of SDC
  and RTL text, not a synthesized netlist. Lint A's "phantom" is therefore
  *inferred* — a `*_pll` token in a group with no matching shipped module — and can
  over-report a legitimately-instantiated-but-differently-named PLL; Lint B detects
  the *presence* of added timing, not whether a constraint actually covers the
  specific crossing. Findings carry a confidence and the analysis records its
  limits rather than claiming certainty it does not have.
- **The lints consume models, not files.** Parsing produces a typed `SdcModel` (the
  clock groups, created clocks and datapath constraints) and a `DesignModel` (the
  clock-name universe and external-memory topology); the lint engine is a pure
  function of those two models. File access sits behind a small `CoreSource` seam
  with a filesystem backend and an in-memory double, so the whole pipeline is proven
  with no real core tree in the loop.
- **Recon-shaped output.** Findings serialize to structured JSON built to be
  ingested as a downstream tool's "CDC hotspots" — severity, the offending
  clock/line, the reason, and a pointer to the fix a later version would write.

## Fixing (`--fix`) — with an honesty guardrail

`--fix` turns detection into correction, but the two lints have very different fix
confidences, and cdc-sentinel refuses to blur them:

- **Lint A (phantom) is safely auto-fixed.** Removing a `-group {}` that names only a
  clock the design never instantiates deletes a no-op line and changes nothing real.
  `--fix` emits the corrected SDC — **byte-identical except the dead member is gone**.
- **Lint B (real crossing) is NOT auto-fixed.** The correct multicycle/false-path
  value depends on the actual timing relationship, which a heuristic scan does not
  know. cdc-sentinel **never emits a specific value as if correct** — it appends a
  clearly-marked **UNVERIFIED** guided suggestion: a *commented* constraint template
  with `<CORE_CLK>` / `<MEM_CLK>` / `<N>` placeholders and a "set the value and
  confirm in STA" banner. A confidently-wrong constraint is the one output we refuse.

The user's SDC is **never overwritten** — corrections go to a new `*.fixed.sdc` file
unless you pass `--in-place`.

`--emit-template-patch` produces the single upstream fix: a proposed patch against the
openFPGA core-template's `core_constraints.sdc` (the root every core inherits the
blanket cut from) that replaces it with a documented scaffold + a per-core TODO, plus
a proposed PR body. cdc-sentinel **does not open the PR** — that is for a maintainer
to review and submit.

## Testing

```
cargo test                        # unit tests + the fixture-corpus lint & fix validation
cargo run -- <dir>                # lint a core directory (--json for machine-readable)
cargo run -- --fix <dir>          # emit corrected SDC + UNVERIFIED crossing suggestions
cargo run -- --emit-template-patch  # the upstream root fix (patch + PR body)
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in this crate by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.
