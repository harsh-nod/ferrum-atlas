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
    validate_source(&source, &context)?;
    extract::analyze(source, context, level)
}

fn validate_source(source: &SourceSnapshot, context: &BuildContext) -> Result<()> {
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
    Ok(())
}

/// An expendable warm frontend cache. Persisted facts never serialize this database.
#[derive(Default)]
pub struct AnalyzerSession {
    database: Option<ra_ap_ide_db::RootDatabase>,
    input_key: String,
    text: Vec<String>,
}
impl AnalyzerSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn analyze(
        &mut self,
        source: SourceSnapshot,
        context: BuildContext,
        level: AnalysisLevel,
    ) -> Result<FactBatch> {
        validate_source(&source, &context)?;
        let configurations = configuration::Configurations::new(&source, &context)?;
        // File IDs are positional in this adapter. Changed path sets, manifests, or
        // configurations rebuild the crate graph; only exact-key file text is reused.
        let key = atlas_model::digest(
            "warm-inputs",
            &(
                &source.repository_id,
                &source.manifests,
                &context,
                source
                    .files
                    .iter()
                    .map(|file| &file.path)
                    .collect::<Vec<_>>(),
                PRODUCER,
            ),
        );
        if let (Some(database), true) = (self.database.as_mut(), self.input_key == key) {
            let mut change = ra_ap_ide_db::ChangeWithProcMacros::default();
            for (index, file) in source.files.iter().enumerate() {
                if self.text[index] != file.text {
                    change.change_file(
                        ra_ap_vfs::FileId::from_raw(index as u32),
                        Some(file.text.clone()),
                    );
                }
            }
            database.apply_change(change);
        } else {
            self.database = Some(database::load(&source, &context, &configurations)?);
        }
        self.input_key = key;
        self.text = source.files.iter().map(|file| file.text.clone()).collect();
        let database = self.database.as_ref().unwrap();
        ra_ap_hir::attach_db(database, || {
            extract::extract(source, context, level, database, &configurations)
        })
    }
}
