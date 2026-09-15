use super::*;
use std::cell::Cell;

#[test]
fn cancelled_line_scans_never_report_a_complete_window_or_line_count() {
    let text = "line\r\n".repeat(100_000);
    let source = SourceFile {
        id: FileId("file:long".into()),
        path: "long.rs".into(),
        content_hash: digest("content", &text),
        text,
    };
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 2
    };
    assert!(matches!(
        source_window(
            &SnapshotId("snapshot:test".into()),
            source.clone(),
            1,
            1,
            &stop
        ),
        Err(Error::BudgetExhausted)
    ));
    calls.set(0);
    assert!(matches!(
        source_line_at(source.text.as_bytes(), &stop),
        Err(Error::BudgetExhausted)
    ));
    let window = source_window(
        &SnapshotId("snapshot:test".into()),
        source,
        100_000,
        2,
        &|| false,
    )
    .unwrap();
    assert_eq!(window.total_lines, 100_001);
    assert_eq!(window.text, "line\r\n");
    assert!(window.truncated);
}
