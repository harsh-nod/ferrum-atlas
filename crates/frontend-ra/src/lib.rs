//! Pinned rust-analyzer adapter. No upstream IDs cross this boundary.
mod database;
mod extract;

use anyhow::{Result, ensure};
use atlas_model::{BuildContext, FactBatch, SourceSnapshot};

pub const PRODUCER: &str = "rust-analyzer/0.0.349;atlas-adapter/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisLevel {
    Syntax,
    Semantic,
}

/// Analyze only the captured bytes. The adapter never loads or executes project tools.
pub fn analyze(
    source: SourceSnapshot,
    context: BuildContext,
    level: AnalysisLevel,
) -> Result<FactBatch> {
    ensure!(
        context.trust == "read_only",
        "analysis requires a read_only context"
    );
    ensure!(
        source.files.len() <= 20_000,
        "analysis file budget exceeded"
    );
    extract::analyze(source, context, level)
}
