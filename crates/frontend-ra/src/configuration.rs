use anyhow::{Context, Result};
use atlas_model::{BuildContext, CfgStatus, SourceSnapshot};
use ra_ap_cfg::CfgOptions;
use ra_ap_intern::Symbol;
use ra_ap_syntax::{AstNode, AstToken, SyntaxNode, ast};
use std::{
    collections::{BTreeMap, BTreeSet},
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
            manifest
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .is_some_and(|deps| {
                    deps.values().any(|dep| {
                        dep.get("features").is_some()
                            || dep.get("default-features").and_then(toml::Value::as_bool)
                                == Some(false)
                    })
                })
        });
        let mut unresolved_forwarding = dependency_overrides;
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

impl Settings {
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
