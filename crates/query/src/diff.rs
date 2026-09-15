use crate::*;

const DIFF_DEFINITIONS: usize = 10_000;
const DIFF_CHANGES: usize = 1000;

impl QueryEngine {
    pub fn diff(&self, request: &DiffRequest) -> Result<DiffResponse> {
        self.authorize(&request.before)?;
        self.authorize(&request.after)?;
        let before = self.snapshot(&request.before)?;
        let after = self.snapshot(&request.after)?;
        if before.repository_id != after.repository_id {
            return Err(Error::InvalidQuery(
                "diff snapshots must belong to one repository".into(),
            ));
        }
        if before.definition_count as usize > DIFF_DEFINITIONS
            || after.definition_count as usize > DIFF_DEFINITIONS
        {
            return Err(Error::BudgetExhausted);
        }
        let control = QueryControl::new(Duration::from_secs(2));
        let old_reader = self.reader(&before.id, &before.context.id, &control)?;
        let new_reader = self.reader(&after.id, &after.context.id, &control)?;
        let old = old_reader.definitions_bounded(DIFF_DEFINITIONS + 1, 16 * 1024 * 1024)?;
        let new = new_reader.definitions_bounded(DIFF_DEFINITIONS + 1, 16 * 1024 * 1024)?;
        if old.len() > DIFF_DEFINITIONS || new.len() > DIFF_DEFINITIONS {
            return Err(Error::BudgetExhausted);
        }
        let mut truncated = false;
        let mut coverage = before.coverage.clone();
        if after.coverage.status != Status::Complete && coverage.status == Status::Complete {
            coverage.status = Status::Partial;
        }
        coverage.reasons.extend(after.coverage.reasons.clone());
        coverage
            .limitations
            .extend(after.coverage.limitations.clone());
        coverage.limitations.push("Correspondence uses unique path, qualified name and kind; moves and renames are not inferred. Impact lists direct caller candidates, not observed execution.".into());
        let old_files: BTreeMap<_, _> = old_reader
            .files()?
            .into_iter()
            .map(|file| (file.id, file.path))
            .collect();
        let new_files: BTreeMap<_, _> = new_reader
            .files()?
            .into_iter()
            .map(|file| (file.id, file.path))
            .collect();
        let mut old_groups = BTreeMap::<_, Vec<Definition>>::new();
        let mut new_groups = BTreeMap::<_, Vec<Definition>>::new();
        for definition in old {
            old_groups
                .entry(key(&definition, &old_files))
                .or_default()
                .push(definition);
        }
        for definition in new {
            new_groups
                .entry(key(&definition, &new_files))
                .or_default()
                .push(definition);
        }
        let keys: BTreeSet<_> = old_groups
            .keys()
            .chain(new_groups.keys())
            .cloned()
            .collect();
        let mut changes = Vec::new();
        let mut bytes = 8192;
        'groups: for key in keys {
            if control.stopped() {
                truncated = true;
                break;
            }
            let mut old = old_groups.remove(&key).unwrap_or_default();
            let mut new = new_groups.remove(&key).unwrap_or_default();
            let mut group_changes = Vec::new();
            if old.len() == 1 && new.len() == 1 {
                let old = old.pop().expect("one old definition");
                let new = new.pop().expect("one new definition");
                let mut changed_fields = Vec::new();
                if old.signature_hash != new.signature_hash {
                    changed_fields.push("signature".into());
                }
                if old.body_hash != new.body_hash {
                    changed_fields.push("body".into());
                }
                if old.visibility != new.visibility {
                    changed_fields.push("visibility".into());
                }
                if old.cfg != new.cfg {
                    changed_fields.push("cfg".into());
                }
                if old.cfg_status != new.cfg_status {
                    changed_fields.push("cfg_status".into());
                }
                if old.span.start != new.span.start || old.span.end != new.span.end {
                    changed_fields.push("source_span".into());
                }
                if !changed_fields.is_empty() {
                    group_changes.push(DefinitionChange {
                        kind: "modified".into(),
                        before: Some(old),
                        after: Some(new),
                        correspondence: "unique_structural_key".into(),
                        changed_fields,
                    });
                }
            } else {
                let ambiguous = old.len() > 1 || new.len() > 1;
                let correspondence = if ambiguous { "ambiguous" } else { "unmatched" };
                if ambiguous {
                    add_reason(&mut coverage, UnknownReason::UnsupportedConstruct);
                }
                for definition in old {
                    group_changes.push(DefinitionChange {
                        kind: "removed".into(),
                        before: Some(definition),
                        after: None,
                        correspondence: correspondence.into(),
                        changed_fields: vec![],
                    });
                }
                for definition in new {
                    group_changes.push(DefinitionChange {
                        kind: "added".into(),
                        before: None,
                        after: Some(definition),
                        correspondence: correspondence.into(),
                        changed_fields: vec![],
                    });
                }
            }
            for change in group_changes {
                let extra = size(&change)?;
                if changes.len() >= DIFF_CHANGES || bytes + extra > RESPONSE_BYTES {
                    truncated = true;
                    break 'groups;
                }
                bytes += extra;
                changes.push(change);
            }
        }
        let mut impact = BTreeMap::new();
        'impact: for change in &changes {
            let Some(definition) = &change.after else {
                continue;
            };
            if control.stopped() {
                truncated = true;
                break;
            }
            let incoming = new_reader.adjacency(&definition.id, &Direction::Incoming, 201)?;
            if incoming.len() == 201 {
                truncated = true;
            }
            for relation in incoming {
                if impact.contains_key(&relation.source) {
                    continue;
                }
                if let Some(caller) = new_reader.definition(&relation.source)? {
                    let extra = size(&caller)?;
                    if impact.len() >= 200 || bytes + extra > RESPONSE_BYTES {
                        truncated = true;
                        break 'impact;
                    }
                    bytes += extra;
                    impact.insert(caller.id.clone(), caller);
                }
            }
        }
        if truncated || control.stopped() {
            truncated = true;
            budget_coverage(
                &mut coverage,
                "Diff reached a definition, change, caller, time, or response byte budget",
            );
        }
        bounded(DiffResponse {
            before: before.id,
            after: after.id,
            context_changed: before.context.id != after.context.id,
            changes,
            impact_candidates: impact.into_values().collect(),
            coverage,
            truncated,
        })
    }
}

fn key(definition: &Definition, files: &BTreeMap<FileId, String>) -> (String, String, String) {
    (
        files.get(&definition.file_id).cloned().unwrap_or_default(),
        definition.qualified_name.clone(),
        definition.kind.clone(),
    )
}
