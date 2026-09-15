use anyhow::{Context, Result};
use atlas_model::{BuildContext, CfgStatus, SourceSnapshot};
use ra_ap_cfg::CfgOptions;
use ra_ap_intern::Symbol;
use ra_ap_syntax::{AstNode, AstToken, SyntaxNode, WalkEvent, ast};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

#[derive(Clone, Default)]
pub(super) struct Settings {
    cfg: BTreeMap<String, Option<String>>,
    features: BTreeSet<String>,
    unknown_features: bool,
}

pub(super) struct Configurations {
    pub roots: BTreeMap<String, Settings>,
    pub fallback: Settings,
}

#[derive(Clone, Copy)]
struct InheritedSettings {
    status: CfgStatus,
    macro_unavailable: bool,
    cfg: Option<usize>,
}

impl Default for InheritedSettings {
    fn default() -> Self {
        Self {
            status: CfgStatus::Active,
            macro_unavailable: false,
            cfg: None,
        }
    }
}

struct CfgAttributes {
    attributes: Vec<String>,
    parent: Option<usize>,
}

/// File-local inherited attributes, computed without rescanning ancestor siblings.
pub(super) struct FileConfigurations {
    nodes: HashMap<SyntaxNode, InheritedSettings>,
    cfg: Vec<CfgAttributes>,
    #[cfg(test)]
    node_visits: usize,
    #[cfg(test)]
    child_visits: usize,
}

impl FileConfigurations {
    pub fn new(root: &SyntaxNode, settings: &Settings) -> Self {
        let mut cache = Self {
            nodes: HashMap::new(),
            cfg: Vec::new(),
            #[cfg(test)]
            node_visits: 0,
            #[cfg(test)]
            child_visits: 0,
        };
        let mut stack = vec![InheritedSettings::default()];
        for event in root.preorder() {
            match event {
                WalkEvent::Enter(node) => {
                    #[cfg(test)]
                    {
                        cache.node_visits += 1;
                    }
                    let mut inherited = *stack.last().unwrap();
                    let mut attributes = Vec::new();
                    for child in node.children() {
                        #[cfg(test)]
                        {
                            cache.child_visits += 1;
                        }
                        let Some(attr) = ast::Attr::cast(child) else {
                            continue;
                        };
                        if let Some(meta) = attr.meta() {
                            inherited.status = combine(
                                [inherited.status, settings.meta_status(meta.clone())].into_iter(),
                                false,
                            );
                            inherited.macro_unavailable |= settings.attribute_unavailable(meta);
                        }
                        if attr
                            .simple_name()
                            .is_some_and(|name| matches!(name.as_str(), "cfg" | "cfg_attr"))
                        {
                            attributes.push(attr.to_string());
                        }
                    }
                    if !attributes.is_empty() {
                        cache.cfg.push(CfgAttributes {
                            attributes,
                            parent: inherited.cfg,
                        });
                        inherited.cfg = Some(cache.cfg.len() - 1);
                    }
                    // Most source nodes have no effective attributes; avoid retaining
                    // a map entry for the default state on those files.
                    if inherited.status != CfgStatus::Active
                        || inherited.macro_unavailable
                        || inherited.cfg.is_some()
                    {
                        cache.nodes.insert(node, inherited);
                    }
                    stack.push(inherited);
                }
                WalkEvent::Leave(_) => {
                    stack.pop();
                }
            }
        }
        cache
    }

    pub fn node_status(&self, node: &SyntaxNode) -> CfgStatus {
        self.nodes.get(node).copied().unwrap_or_default().status
    }

    pub fn has_attribute_macro(&self, node: &SyntaxNode) -> bool {
        self.nodes
            .get(node)
            .copied()
            .unwrap_or_default()
            .macro_unavailable
    }

    pub fn cfg(&self, node: &SyntaxNode) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = self.nodes.get(node).and_then(|settings| settings.cfg);
        while let Some(index) = current {
            let entry = &self.cfg[index];
            result.extend(entry.attributes.iter().cloned());
            current = entry.parent;
        }
        result
    }
}

impl Configurations {
    pub fn new(source: &SourceSnapshot, context: &BuildContext) -> Result<Self> {
        let manifests = source
            .manifests
            .iter()
            .filter(|(path, _)| path.ends_with("Cargo.toml"))
            .map(|(path, text)| {
                Ok((
                    path.as_str(),
                    toml::from_str::<toml::Value>(text).with_context(|| format!("parse {path}"))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut roots = BTreeMap::new();
        let fallback = Settings {
            cfg: context.cfg.clone(),
            features: context.features.iter().cloned().collect(),
            unknown_features: true,
        };
        // Local feature closure is supported. Dependency feature unification and forwarding
        // require Cargo's resolver, so affected predicates remain unknown.
        let dependency_overrides = manifests.values().any(|manifest| {
            has_dependency_overrides(manifest)
                || manifest
                    .get("workspace")
                    .is_some_and(has_dependency_overrides)
                || manifest.get("target").is_some()
        });
        let virtual_workspace_selection = manifests
            .get("Cargo.toml")
            .is_some_and(|manifest| manifest.get("package").is_none())
            && (!context.features.is_empty() || !context.default_features);
        let mut unresolved_forwarding = dependency_overrides || virtual_workspace_selection;
        for krate in &context.crates {
            let owner = Path::new(&krate.root_file)
                .ancestors()
                .skip(1)
                .find_map(|directory| {
                    let manifest_path = directory.join("Cargo.toml");
                    let path = manifest_path.to_str()?;
                    manifests
                        .get_key_value(path)
                        .filter(|(_, manifest)| manifest.get("package").is_some())
                });
            let mut settings = Settings {
                cfg: context.cfg.clone(),
                ..Settings::default()
            };
            let mut pending = Vec::new();
            let primary = owner.is_none_or(|(path, _)| *path == "Cargo.toml");
            if primary {
                pending.extend(context.features.iter().cloned());
            }
            let declared = owner
                .and_then(|(_, manifest)| manifest.get("features"))
                .and_then(toml::Value::as_table);
            if (context.default_features || !primary)
                && declared.is_some_and(|features| features.contains_key("default"))
            {
                pending.push("default".into());
            }
            while let Some(feature) = pending.pop() {
                if feature.contains('/') || feature.starts_with("dep:") {
                    settings.unknown_features = true;
                    continue;
                }
                if !settings.features.insert(feature.clone()) {
                    continue;
                }
                if let Some(enabled) = declared
                    .and_then(|features| features.get(&feature))
                    .and_then(toml::Value::as_array)
                {
                    for enabled in enabled {
                        if let Some(enabled) = enabled.as_str() {
                            pending.push(enabled.into());
                        } else {
                            settings.unknown_features = true;
                        }
                    }
                }
            }
            if let Some(Some(feature)) = context.cfg.get("feature") {
                settings.features.insert(feature.clone());
            }
            unresolved_forwarding |= settings.unknown_features;
            roots.insert(krate.root_file.clone(), settings);
        }
        if unresolved_forwarding {
            for settings in roots.values_mut() {
                settings.unknown_features = true;
            }
        }
        Ok(Self { roots, fallback })
    }
}

fn has_dependency_overrides(manifest: &toml::Value) -> bool {
    manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .is_some_and(|deps| {
            deps.values().any(|dep| {
                dep.get("features").is_some()
                    || dep.get("default-features").and_then(toml::Value::as_bool) == Some(false)
            })
        })
}

impl Settings {
    #[cfg(test)]
    pub fn has_attribute_macro(&self, node: &SyntaxNode) -> bool {
        node.ancestors()
            .flat_map(|node| {
                node.children()
                    .filter_map(ast::Attr::cast)
                    .collect::<Vec<_>>()
            })
            .filter_map(|attr| attr.meta())
            .any(|meta| self.attribute_unavailable(meta))
    }

    fn attribute_unavailable(&self, meta: ast::Meta) -> bool {
        match meta {
            ast::Meta::CfgMeta(_) => false,
            ast::Meta::CfgAttrMeta(meta) => match meta
                .cfg_predicate()
                .map(|predicate| self.predicate(predicate))
                .unwrap_or(CfgStatus::Unknown)
            {
                CfgStatus::Inactive => false,
                CfgStatus::Active => meta.metas().any(|meta| self.attribute_unavailable(meta)),
                CfgStatus::Unknown => true,
            },
            ast::Meta::UnsafeMeta(meta) => meta
                .meta()
                .is_none_or(|meta| self.attribute_unavailable(meta)),
            _ => !meta.simple_name().is_some_and(|name| {
                matches!(
                    name.as_str(),
                    "derive"
                        | "doc"
                        | "allow"
                        | "warn"
                        | "deny"
                        | "forbid"
                        | "expect"
                        | "inline"
                        | "repr"
                        | "test"
                        | "must_use"
                        | "no_mangle"
                        | "export_name"
                        | "link"
                        | "link_name"
                        | "link_section"
                        | "path"
                        | "cold"
                        | "deprecated"
                        | "no_std"
                        | "no_main"
                        | "non_exhaustive"
                        | "track_caller"
                        | "automatically_derived"
                        | "used"
                        | "global_allocator"
                        | "panic_handler"
                )
            }),
        }
    }

    pub fn options(&self) -> CfgOptions {
        let mut options = CfgOptions::default();
        for (key, value) in &self.cfg {
            if let Some(value) = value {
                options.insert_key_value(Symbol::intern(key), Symbol::intern(value));
            } else {
                options.insert_atom(Symbol::intern(key));
            }
        }
        for feature in &self.features {
            options.insert_key_value(Symbol::intern("feature"), Symbol::intern(feature));
        }
        options
    }

    #[cfg(test)]
    pub fn node_status(&self, node: &SyntaxNode) -> CfgStatus {
        combine(
            node.ancestors()
                .flat_map(|node| {
                    node.children()
                        .filter_map(ast::Attr::cast)
                        .collect::<Vec<_>>()
                })
                .filter_map(|attr| attr.meta())
                .map(|meta| self.meta_status(meta)),
            false,
        )
    }

    pub fn attr_status(&self, attr: &ast::Attr) -> CfgStatus {
        attr.meta()
            .map(|meta| self.meta_status(meta))
            .unwrap_or(CfgStatus::Unknown)
    }

    fn meta_status(&self, meta: ast::Meta) -> CfgStatus {
        match meta {
            ast::Meta::CfgMeta(meta) => meta
                .cfg_predicate()
                .map(|predicate| self.predicate(predicate))
                .unwrap_or(CfgStatus::Unknown),
            ast::Meta::CfgAttrMeta(meta) => match meta
                .cfg_predicate()
                .map(|predicate| self.predicate(predicate))
                .unwrap_or(CfgStatus::Unknown)
            {
                CfgStatus::Inactive => CfgStatus::Active,
                CfgStatus::Active => {
                    combine(meta.metas().map(|meta| self.meta_status(meta)), false)
                }
                CfgStatus::Unknown => CfgStatus::Unknown,
            },
            _ => CfgStatus::Active,
        }
    }

    fn predicate(&self, predicate: ast::CfgPredicate) -> CfgStatus {
        match predicate {
            ast::CfgPredicate::CfgAtom(atom) => {
                if atom.true_token().is_some() {
                    return CfgStatus::Active;
                }
                if atom.false_token().is_some() {
                    return CfgStatus::Inactive;
                }
                let Some(key) = atom.ident_token() else {
                    return CfgStatus::Unknown;
                };
                if atom.eq_token().is_some() {
                    let value = atom
                        .string_token()
                        .and_then(ast::String::cast)
                        .and_then(|value| value.value().ok().map(|value| value.into_owned()));
                    let Some(value) = value else {
                        return CfgStatus::Unknown;
                    };
                    if key.text() == "feature" {
                        if self.unknown_features {
                            CfgStatus::Unknown
                        } else if self.features.contains(&value) {
                            CfgStatus::Active
                        } else {
                            CfgStatus::Inactive
                        }
                    } else {
                        self.cfg
                            .get(key.text())
                            .map(|selected| from_bool(selected.as_ref() == Some(&value)))
                            .unwrap_or(CfgStatus::Unknown)
                    }
                } else if self.cfg.contains_key(key.text()) {
                    from_bool(self.cfg[key.text()].is_none())
                } else if key.text() == "test" {
                    CfgStatus::Inactive
                } else {
                    CfgStatus::Unknown
                }
            }
            ast::CfgPredicate::CfgComposite(composite) => {
                let Some(keyword) = composite.keyword() else {
                    return CfgStatus::Unknown;
                };
                let mut children = composite
                    .cfg_predicates()
                    .map(|predicate| self.predicate(predicate));
                match keyword.text() {
                    "all" => combine(children, false),
                    "any" => combine(children, true),
                    "not" => match (children.next(), children.next()) {
                        (Some(CfgStatus::Active), None) => CfgStatus::Inactive,
                        (Some(CfgStatus::Inactive), None) => CfgStatus::Active,
                        _ => CfgStatus::Unknown,
                    },
                    _ => CfgStatus::Unknown,
                }
            }
        }
    }
}

fn from_bool(value: bool) -> CfgStatus {
    if value {
        CfgStatus::Active
    } else {
        CfgStatus::Inactive
    }
}
fn combine(values: impl Iterator<Item = CfgStatus>, any: bool) -> CfgStatus {
    let mut unknown = false;
    for value in values {
        match value {
            CfgStatus::Active if any => return CfgStatus::Active,
            CfgStatus::Inactive if !any => return CfgStatus::Inactive,
            CfgStatus::Unknown => unknown = true,
            _ => {}
        }
    }
    if unknown {
        CfgStatus::Unknown
    } else {
        from_bool(!any)
    }
}

#[cfg(test)]
#[path = "configuration/tests.rs"]
mod tests;
