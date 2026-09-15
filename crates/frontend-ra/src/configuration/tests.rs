use super::*;
use ra_ap_syntax::Edition;

fn legacy_cfg(node: &SyntaxNode) -> Vec<String> {
    node.ancestors()
        .flat_map(|ancestor| {
            ancestor
                .children()
                .filter_map(ast::Attr::cast)
                .collect::<Vec<_>>()
        })
        .filter(|attr| {
            attr.simple_name()
                .is_some_and(|name| matches!(name.as_str(), "cfg" | "cfg_attr"))
        })
        .map(|attr| attr.to_string())
        .collect()
}

fn assert_matches_legacy(text: &str, settings: &Settings) -> FileConfigurations {
    let parsed = ra_ap_syntax::SourceFile::parse(text, Edition::Edition2024);
    let root = parsed.syntax_node();
    let cache = FileConfigurations::new(&root, settings);
    for node in root.descendants() {
        assert_eq!(
            cache.node_status(&node),
            settings.node_status(&node),
            "cfg status at {:?} {:?}",
            node.kind(),
            node.text_range()
        );
        assert_eq!(
            cache.has_attribute_macro(&node),
            settings.has_attribute_macro(&node),
            "macro availability at {:?} {:?}",
            node.kind(),
            node.text_range()
        );
        assert_eq!(cache.cfg(&node), legacy_cfg(&node));
    }
    assert_eq!(cache.node_visits, root.descendants().count());
    assert_eq!(cache.child_visits + 1, cache.node_visits);
    cache
}

#[test]
fn cached_inheritance_matches_legacy_for_cfg_attr_and_macro_combinations() {
    let text = r#"
#![cfg_attr(feature = "selected", allow(dead_code))]
#[cfg(unknown_outer)]
#[cfg_attr(feature = "selected", cfg(enabled), wrapper)]
mod outer {
    #![cfg_attr(enabled, cfg(not(test)))]
    #[cfg_attr(unknown_inner, cfg(enabled), another_wrapper)]
    #[cfg(feature = "selected")]
    fn event() {
        #[cfg(false)]
        { target(); }
        #[cfg_attr(feature = "absent", wrapper)]
        let local = || { leaf(); };
        #[cfg_attr(enabled, cfg(false), wrapper)]
        fn nested() { nested_target(); }
        #[unsafe(no_mangle)]
        fn safe_attribute() { target(); }
    }
}
#[cfg(false)]
#[custom_attribute]
fn inactive_macro() { target(); }
#[derive(Clone)]
struct Kept;
"#;
    for unknown_features in [false, true] {
        for enabled in [false, true] {
            let mut settings = Settings {
                features: BTreeSet::from(["selected".into()]),
                unknown_features,
                ..Default::default()
            };
            if enabled {
                settings.cfg.insert("enabled".into(), None);
            }
            assert_matches_legacy(text, &settings);
        }
    }
}

#[test]
fn cached_cfg_strings_preserve_nearest_first_and_sibling_attribute_order() {
    let parsed = ra_ap_syntax::SourceFile::parse(
        "#![cfg(true)] #[cfg(all())] mod outer { #[cfg(not(false))] #[cfg_attr(true, allow(dead_code))] fn event() {} }",
        Edition::Edition2024,
    );
    let root = parsed.syntax_node();
    let cache = FileConfigurations::new(&root, &Settings::default());
    let function = root.descendants().find_map(ast::Fn::cast).unwrap();
    assert_eq!(
        cache.cfg(function.syntax()),
        vec![
            "#[cfg(not(false))]",
            "#[cfg_attr(true, allow(dead_code))]",
            "#[cfg(all())]",
            "#![cfg(true)]",
        ]
    );
}

#[test]
fn incomplete_attributes_and_nested_unknowns_match_legacy() {
    for text in [
        "#[cfg()] fn event() { #[cfg_attr()] let local = 1; }",
        "#[cfg_attr(true, cfg(), unsafe())] fn event() {}",
        "#[cfg(unknown)] mod outer { #[cfg(false)] fn event() { target(); } }",
        "#[cfg_attr(false, wrapper)] mod outer { fn event() { target(); } }",
        "#[cfg_attr(unknown, allow(dead_code))] fn event() { target(); }",
    ] {
        assert_matches_legacy(text, &Settings::default());
    }
}

#[test]
fn flat_statement_and_sibling_item_scans_are_linear_and_default_cache_is_sparse() {
    let mut previous = None;
    for count in [32, 64, 128] {
        for body in [
            format!("fn event() {{ {} }}", "target();".repeat(count)),
            (0..count)
                .map(|index| format!("fn event_{index}() {{ target(); }}"))
                .collect::<String>(),
        ] {
            let cache = assert_matches_legacy(&body, &Settings::default());
            assert!(cache.nodes.is_empty());
            assert!(cache.cfg.is_empty());
            assert_eq!(cache.node_visits, cache.child_visits + 1);
        }
        let text = format!(
            "#[cfg(true)] fn event() {{ {} }}",
            "target();".repeat(count)
        );
        let cache = assert_matches_legacy(&text, &Settings::default());
        if let Some(visits) = previous {
            assert!(cache.child_visits <= visits * 2);
        }
        previous = Some(cache.child_visits);
    }
}

#[test]
fn deeply_nested_scopes_share_attribute_chains_without_repeated_child_scans() {
    let depth = 64;
    let text = format!(
        "{}fn event() {{ leaf(); }}{}",
        "#[cfg(true)] mod nested {".repeat(depth),
        "}".repeat(depth)
    );
    let cache = assert_matches_legacy(&text, &Settings::default());
    assert_eq!(cache.cfg.len(), depth);
    assert_eq!(
        cache
            .cfg
            .iter()
            .map(|entry| entry.attributes.len())
            .sum::<usize>(),
        depth
    );
}

#[test]
fn separate_file_and_configuration_caches_do_not_reuse_stale_status() {
    let text = "#[cfg(feature = \"selected\")] fn event() { target(); }";
    let selected = Settings {
        features: BTreeSet::from(["selected".into()]),
        ..Default::default()
    };
    for settings in [&selected, &Settings::default(), &selected] {
        assert_matches_legacy(text, settings);
    }
}
