#[test]
fn follows_positive_and_zero_paths() {
    assert_eq!(atlas_pilot::entry(3), 8);
    assert_eq!(atlas_pilot::entry(0), 0);
    assert_eq!(atlas_pilot::callback(3, atlas_pilot::entry), 8);
}
