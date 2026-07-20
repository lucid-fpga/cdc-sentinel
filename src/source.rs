//! The source seam. Both the SDC parser and the RTL scan consume files through a
//! [`CoreSource`], so the whole pipeline runs against either a real core directory
//! ([`FsSource`]) or an in-memory double ([`MemSource`]) with no change to the
//! analysis. This is the testable seam the lints are built on: a unit test hands
//! the parser a `MemSource` of a few lines of synthesized SDC/Verilog and asserts
//! the resulting model, no core tree required.

use crate::error::{Error, Result};
use std::path::Path;

/// One source file: a repo-relative-ish path (used for evidence lines) and its
/// text. Binary/undecodable bytes are lossily decoded so a stray file never aborts
/// a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Path as it should appear in a finding's evidence (relative to the core root
    /// for [`FsSource`]; verbatim for [`MemSource`]).
    pub path: String,
    /// File contents.
    pub text: String,
}

impl SourceFile {
    /// Construct a file, e.g. in a test double.
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        SourceFile { path: path.into(), text: text.into() }
    }

    /// Lowercased basename extension (no dot), or `""`.
    pub fn ext(&self) -> String {
        Path::new(&self.path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// Is this a timing-constraint file cdc-sentinel should parse?
    pub fn is_sdc(&self) -> bool {
        self.ext() == "sdc"
    }

    /// Is this an RTL source file the structural scan should read?
    pub fn is_rtl(&self) -> bool {
        matches!(self.ext().as_str(), "v" | "sv" | "vh" | "svh")
    }
}

/// A provider of a core's source files. The one method keeps the seam trivial to
/// double; consumers filter by [`SourceFile::is_sdc`] / [`SourceFile::is_rtl`].
pub trait CoreSource {
    /// Every source file cdc-sentinel might read, in a stable order.
    fn files(&self) -> Vec<SourceFile>;
}

/// In-memory source — the test double. Holds a fixed set of files; nothing touches
/// the filesystem.
#[derive(Debug, Default, Clone)]
pub struct MemSource {
    files: Vec<SourceFile>,
}

impl MemSource {
    /// An empty double.
    pub fn new() -> Self {
        MemSource::default()
    }

    /// Add a file, builder-style.
    pub fn with(mut self, path: impl Into<String>, text: impl Into<String>) -> Self {
        self.files.push(SourceFile::new(path, text));
        self
    }
}

impl CoreSource for MemSource {
    fn files(&self) -> Vec<SourceFile> {
        self.files.clone()
    }
}

/// Filesystem source — walks a core directory, collecting SDC and RTL files. Skips
/// `target/`, `.git/` and other dotted directories, and files that do not decode as
/// SDC/RTL. Paths in the returned files are relative to `root`.
pub struct FsSource {
    root: std::path::PathBuf,
    files: Vec<SourceFile>,
}

impl FsSource {
    /// Walk `root`, reading every SDC/RTL file under it. Fails only if `root`
    /// itself cannot be read; individual unreadable files are skipped.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut files = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).map_err(|source| Error::Source {
                path: dir.display().to_string(),
                source,
            })?;
            for entry in entries.flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if p.is_dir() {
                    if name == "target" || name.starts_with('.') {
                        continue;
                    }
                    stack.push(p);
                    continue;
                }
                let rel = p
                    .strip_prefix(&root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                let probe = SourceFile { path: rel.clone(), text: String::new() };
                if !(probe.is_sdc() || probe.is_rtl()) {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(&p) {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    files.push(SourceFile { path: rel, text });
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(FsSource { root, files })
    }

    /// The directory this source was opened on.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl CoreSource for FsSource {
    fn files(&self) -> Vec<SourceFile> {
        self.files.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_extensions() {
        assert!(SourceFile::new("a/core_constraints.sdc", "").is_sdc());
        assert!(SourceFile::new("rtl/core_pll.v", "").is_rtl());
        assert!(SourceFile::new("rtl/top.sv", "").is_rtl());
        assert!(!SourceFile::new("readme.md", "").is_rtl());
        assert!(!SourceFile::new("readme.md", "").is_sdc());
    }

    #[test]
    fn mem_source_round_trips() {
        let s = MemSource::new()
            .with("a.sdc", "set_clock_groups -asynchronous")
            .with("b.v", "module core_pll(); endmodule");
        let files = s.files();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.sdc");
        assert!(files[0].is_sdc());
        assert!(files[1].is_rtl());
    }
}
