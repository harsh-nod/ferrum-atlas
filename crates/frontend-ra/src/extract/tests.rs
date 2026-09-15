use super::*;

fn legacy_add_gap(coverage: &mut Coverage, reason: UnknownReason, limitation: &str) {
    coverage.status = Status::Partial;
    if let Some(count) = coverage
        .reasons
        .iter_mut()
        .find(|entry| entry.reason == reason)
    {
        count.count += 1;
    } else {
        coverage.reasons.push(ReasonCount { reason, count: 1 });
    }
    if !coverage.limitations.iter().any(|value| value == limitation) {
        coverage.limitations.push(limitation.into());
    }
}

#[test]
fn coverage_accumulation_preserves_exact_messages_counts_and_insertion_order() {
    let initial = Coverage {
        status: Status::Partial,
        reasons: vec![ReasonCount {
            reason: UnknownReason::CfgUnknown,
            count: 3,
        }],
        limitations: vec!["existing".into(), "existing".into(), "retained".into()],
    };
    let mut expected = initial.clone();
    let mut actual = CoverageAccumulator::new(initial);
    for index in 0..512 {
        for (reason, text) in [
            (UnknownReason::CfgUnknown, "existing".to_string()),
            (
                UnknownReason::MissingDependency,
                format!("Unresolved missing_{index}: MissingDependency"),
            ),
            (
                UnknownReason::MacroUnavailable,
                format!("Unresolved missing_{index}: MissingDependency"),
            ),
        ] {
            legacy_add_gap(&mut expected, reason.clone(), &text);
            actual.add_gap(reason, &text);
        }
    }
    assert_eq!(actual.lookups, 512 * 3);
    assert_eq!(actual.finish(), expected);
}

#[test]
fn distinct_unresolved_labels_require_one_set_lookup_each() {
    for count in [1024, 2048, 4096] {
        let mut actual = CoverageAccumulator::new(Coverage::complete());
        for index in 0..count {
            let label = format!("Unresolved target_{index}: CfgUnknown");
            actual.add_gap(UnknownReason::CfgUnknown, &label);
            actual.add_gap(UnknownReason::CfgUnknown, &label);
        }
        assert_eq!(actual.lookups, count * 2);
        let coverage = actual.finish();
        assert_eq!(coverage.limitations.len(), count);
        assert_eq!(coverage.reasons[0].count, (count * 2) as u32);
        assert_eq!(coverage.status, Status::Partial);
    }
}
