//! Immutable reviewer declarations, separate from inferred transition candidates.
use crate::ScopedDirectory;
use anyhow::{Context, Result, ensure};
use atlas_analysis::{StateMachineInference, StateTransitionReview, validate_state_review};
use atlas_model::{ContextId, DefinitionId, SnapshotId, digest};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_REVIEWS: usize = 100;
const MAX_BYTES: u64 = 16 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReview {
    version: u32,
    snapshot_id: SnapshotId,
    context_id: ContextId,
    definition_id: DefinitionId,
    review: StateTransitionReview,
}

fn scope(root: &Path, snapshot: &SnapshotId, context: &ContextId) -> PathBuf {
    root.join(digest("state-review-scope", &(snapshot, context)).replace(':', "-"))
}

pub fn list(
    root: &Path,
    snapshot: &SnapshotId,
    context: &ContextId,
    inference: &StateMachineInference,
) -> Result<Vec<StateTransitionReview>> {
    list_with_stop(root, snapshot, context, inference, &|| false)
}

pub fn list_with_stop(
    root: &Path,
    snapshot: &SnapshotId,
    context: &ContextId,
    inference: &StateMachineInference,
    stopped: &dyn Fn() -> bool,
) -> Result<Vec<StateTransitionReview>> {
    ensure!(!stopped(), "state review read cancelled");
    let Some(folder) = ScopedDirectory::open(&scope(root, snapshot, context), false)? else {
        return Ok(vec![]);
    };
    read_reviews(&folder, snapshot, context, inference, stopped)
}

fn read_reviews(
    folder: &ScopedDirectory,
    snapshot: &SnapshotId,
    context: &ContextId,
    inference: &StateMachineInference,
    stopped: &dyn Fn() -> bool,
) -> Result<Vec<StateTransitionReview>> {
    let mut reviews = vec![];
    for name in folder.json_names_with_stop(MAX_REVIEWS, stopped)? {
        ensure!(!stopped(), "state review read cancelled");
        let stored: StoredReview =
            serde_json::from_slice(&folder.read_with_stop(&name, MAX_BYTES, stopped)?)?;
        let expected = format!("{}.json", digest("state-review", &stored).replace(':', "-"));
        ensure!(
            name == Path::new(&expected),
            "state review checksum mismatch"
        );
        ensure!(
            stored.version == 1 && stored.snapshot_id == *snapshot && stored.context_id == *context,
            "state review scope mismatch"
        );
        if stored.definition_id != inference.definition_id
            || stored.review.input_digest != inference.input_digest
        {
            continue;
        }
        validate_state_review(inference, &stored.review)?;
        reviews.push(stored.review);
    }
    ensure!(!stopped(), "state review read cancelled");
    Ok(reviews)
}

pub fn record(
    root: &Path,
    snapshot: &SnapshotId,
    context: &ContextId,
    inference: &StateMachineInference,
    review: StateTransitionReview,
) -> Result<StateTransitionReview> {
    record_with_stop(root, snapshot, context, inference, review, &|| false)
}

pub fn record_with_stop(
    root: &Path,
    snapshot: &SnapshotId,
    context: &ContextId,
    inference: &StateMachineInference,
    review: StateTransitionReview,
    stopped: &dyn Fn() -> bool,
) -> Result<StateTransitionReview> {
    ensure!(!stopped(), "state review write cancelled");
    validate_state_review(inference, &review)?;
    let stored = StoredReview {
        version: 1,
        snapshot_id: snapshot.clone(),
        context_id: context.clone(),
        definition_id: inference.definition_id.clone(),
        review,
    };
    let bytes = serde_json::to_vec(&stored)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "state review byte budget exceeded"
    );
    let name = PathBuf::from(format!(
        "{}.json",
        digest("state-review", &stored).replace(':', "-")
    ));
    let folder = ScopedDirectory::open(&scope(root, snapshot, context), true)?
        .context("state review scope unavailable")?;
    let _lock = folder.import_lock_with_stop(stopped)?;
    read_reviews(&folder, snapshot, context, inference, stopped)?;
    if folder.exists(&name)? {
        ensure!(
            folder.read_with_stop(&name, MAX_BYTES, stopped)? == bytes,
            "state review collision"
        );
        return Ok(stored.review);
    }
    ensure!(
        folder.json_names_with_stop(MAX_REVIEWS, stopped)?.len() < MAX_REVIEWS,
        "snapshot state review limit reached"
    );
    ensure!(
        folder.publish_with_stop(&name, &bytes, stopped)?,
        "state review publication collision"
    );
    Ok(stored.review)
}
