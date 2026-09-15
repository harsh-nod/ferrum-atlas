//! Pinned rust-analyzer adapter. No upstream IDs cross this boundary.
mod configuration;
mod database;
mod extract;

use anyhow::{Result, ensure};
use atlas_model::{BuildContext, FactBatch, SourceSnapshot};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

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
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut bytes = 0usize;
    for file in &source.files {
        ensure!(
            file.text.len() <= 4 * 1024 * 1024,
            "analysis file byte budget exceeded"
        );
        ensure!(
            !file.path.is_empty()
                && Path::new(&file.path)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "source path must be normalized and relative"
        );
        ensure!(
            paths.insert(&file.path) && ids.insert(&file.id),
            "duplicate source path or file identity"
        );
        bytes = bytes.saturating_add(file.text.len());
    }
    for text in source.manifests.values() {
        bytes = bytes.saturating_add(text.len());
    }
    ensure!(
        bytes <= 256 * 1024 * 1024,
        "analysis total byte budget exceeded"
    );
    extract::analyze(source, context, level)
}
