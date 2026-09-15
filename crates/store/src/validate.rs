use crate::{Error, Result};
use atlas_model::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

pub fn validate(batch: &FactBatch) -> Result<()> {
    if batch.schema_version != SCHEMA_VERSION {
        return Err(Error::UnsupportedVersion(batch.schema_version));
    }
    require(!batch.producer.is_empty(), "producer is required")?;
    require(
        !batch.context.id.0.is_empty() && !batch.source.id.0.is_empty(),
        "source and context identities are required",
    )?;
    require(
        !batch.source.repository_id.0.is_empty(),
        "repository identity is required",
    )?;
    let mut files = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for file in &batch.source.files {
        require(
            !file.id.0.is_empty() && files.insert(&file.id, file).is_none(),
            "duplicate or empty file identity",
        )?;
        require(
            !file.path.is_empty()
                && Path::new(&file.path)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "source path must be repository-relative",
        )?;
        require(paths.insert(&file.path), "duplicate source path")?;
        require(
            file.text.len() <= 64 * 1024 * 1024,
            "source exceeds local 64 MiB limit",
        )?;
        require(
            file.content_hash == digest("content", &file.text),
            "source content checksum mismatch",
        )?;
    }
    let check_span = |span: &Span| -> Result<()> {
        let file = files
            .get(&span.file_id)
            .ok_or_else(|| Error::Invalid("span references missing source".into()))?;
        require(
            span.start <= span.end && span.end as usize <= file.text.len(),
            "source span is out of bounds",
        )?;
        require(
            file.text.is_char_boundary(span.start as usize)
                && file.text.is_char_boundary(span.end as usize),
            "source span splits UTF-8 character",
        )
    };
    let mut definitions = BTreeMap::new();
    for definition in &batch.definitions {
        require(
            !definition.id.0.is_empty() && definitions.insert(&definition.id, definition).is_none(),
            "duplicate or empty definition identity",
        )?;
        require(
            definition.context_id == batch.context.id,
            "definition context mismatch",
        )?;
        require(
            definition.file_id == definition.span.file_id,
            "definition file mismatch",
        )?;
        check_span(&definition.span)?;
        if let Some(body) = &definition.body_span {
            check_span(body)?;
            require(
                body.file_id == definition.file_id
                    && body.start >= definition.span.start
                    && body.end <= definition.span.end,
                "body must be inside its definition",
            )?;
        }
    }
    for definition in &batch.definitions {
        let mut ancestors = BTreeSet::new();
        let mut parent = definition.parent_id.as_ref();
        while let Some(id) = parent {
            require(
                id != &definition.id && ancestors.insert(id),
                "cyclic definition ownership",
            )?;
            let owner = definitions
                .get(id)
                .ok_or_else(|| Error::Invalid("definition references missing owner".into()))?;
            parent = owner.parent_id.as_ref();
        }
    }
    let mut evidence = BTreeSet::new();
    for item in &batch.evidence {
        require(
            !item.id.0.is_empty() && evidence.insert(&item.id),
            "duplicate or empty evidence identity",
        )?;
        require(
            !item.producer.is_empty() && !item.basis.is_empty(),
            "evidence requires a producer and basis",
        )?;
    }
    let mut relations = BTreeSet::new();
    for relation in &batch.relations {
        require(
            !relation.id.0.is_empty() && relations.insert(&relation.id),
            "duplicate or empty relation identity",
        )?;
        require(
            definitions.contains_key(&relation.source),
            "relation references missing source definition",
        )?;
        if let Target::Resolved { id } = &relation.target {
            require(
                definitions.contains_key(id),
                "resolved relation references missing target",
            )?;
        }
        require(
            evidence.contains(&relation.evidence_id),
            "relation references missing evidence",
        )?;
        check_span(&relation.span)?;
    }
    let mut flows = BTreeSet::new();
    for flow in &batch.flows {
        require(
            flows.insert((&flow.definition_id, &flow.phase)),
            "duplicate function flow",
        )?;
        require(
            definitions.contains_key(&flow.definition_id),
            "flow references missing definition",
        )?;
        for point in &flow.points {
            check_span(&point.span)?;
        }
    }
    Ok(())
}

pub(crate) fn normalize(batch: &FactBatch) -> FactBatch {
    let mut batch = batch.clone();
    batch.source.files.sort_by(|a, b| a.id.cmp(&b.id));
    batch.source.warnings.sort();
    batch.definitions.sort_by(|a, b| a.id.cmp(&b.id));
    batch.relations.sort_by(|a, b| a.id.cmp(&b.id));
    batch.evidence.sort_by(|a, b| a.id.cmp(&b.id));
    batch
        .flows
        .sort_by(|a, b| (&a.definition_id, &a.phase).cmp(&(&b.definition_id, &b.phase)));
    batch
}
