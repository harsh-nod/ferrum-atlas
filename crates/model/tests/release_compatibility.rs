#[test]
fn release_compatibility_metadata_matches_public_schema_constants() {
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../../scripts/release-schema.json")).unwrap();
    assert_eq!(metadata["metadata_version"], 1);
    assert_eq!(metadata["qualification"], "unqualified_prerelease");
    assert_eq!(metadata["deployment"], "local_single_node");
    assert_eq!(
        metadata["model_schema_version"],
        atlas_model::SCHEMA_VERSION
    );
    assert_eq!(
        metadata["store_schema_version_write"],
        atlas_model::SCHEMA_VERSION
    );
    assert_eq!(
        metadata["store_schema_versions_read"],
        serde_json::json!([atlas_model::SCHEMA_VERSION])
    );
    assert_eq!(
        metadata["portable_schema_versions_read"],
        serde_json::json!([atlas_model::SCHEMA_VERSION])
    );
    assert_eq!(
        metadata["compiler_bundle_schema_versions_read"],
        serde_json::json!([atlas_model::COMPILER_SCHEMA_VERSION])
    );
}
