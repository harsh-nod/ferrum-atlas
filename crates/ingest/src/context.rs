use super::CaptureOptions;
use anyhow::{Context, Result, bail, ensure};
use atlas_model::{
    BuildContext, ContextId, Coverage, CrateInput, SourceSnapshot, Status, UnknownReason, digest,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

fn add_gap(coverage: &mut Coverage, reason: UnknownReason, limitation: String) {
    coverage.status = Status::Partial;
    if let Some(count) = coverage
        .reasons
        .iter_mut()
        .find(|item| item.reason == reason)
    {
        count.count += 1;
    } else {
        coverage
            .reasons
            .push(atlas_model::ReasonCount { reason, count: 1 });
    }
    coverage.limitations.push(limitation);
}

pub(super) fn build_context(
    source: &SourceSnapshot,
    options: &CaptureOptions,
) -> Result<BuildContext> {
    let manifest_digest = digest("manifests", &source.manifests);
    if let Some(mut context) = options.explicit_context.clone() {
        ensure!(
            context.trust == "read_only",
            "only read_only analysis is supported"
        );
        validate_crates(source, &context.crates)?;
        context.manifest_digest = manifest_digest;
        context.features.sort();
        context.features.dedup();
        context.crates.sort_by(|a, b| a.root_file.cmp(&b.root_file));
        context.id = context_id(&context);
        return Ok(context);
    }
    let manifests = source
        .manifests
        .iter()
        .filter(|(path, _)| path.ends_with("Cargo.toml"))
        .map(|(path, text)| {
            Ok((
                path.clone(),
                toml::from_str::<toml::Value>(text).with_context(|| format!("parse {path}"))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut coverage = Coverage::partial(
        UnknownReason::MissingDependency,
        "Manifest-only discovery does not load a sysroot, run Cargo feature resolution, build scripts, or procedural macros.",
    );
    for warning in &source.warnings {
        add_gap(
            &mut coverage,
            UnknownReason::UnsupportedConstruct,
            warning.clone(),
        );
    }
    let paths = source
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut crates = Vec::<CrateInput>::new();
    let mut package_libraries = BTreeMap::<String, String>::new();
    let mut crate_manifests = BTreeMap::<String, String>::new();
    for (manifest_path, manifest) in &manifests {
        let Some(package) = manifest.get("package") else {
            continue;
        };
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        let directory = Path::new(manifest_path).parent().unwrap_or(Path::new(""));
        let edition = package
            .get("edition")
            .and_then(toml::Value::as_str)
            .or_else(|| {
                if !package.get("edition")?.get("workspace")?.as_bool()? {
                    return None;
                }
                manifests
                    .get("Cargo.toml")?
                    .get("workspace")?
                    .get("package")?
                    .get("edition")?
                    .as_str()
            })
            .unwrap_or("2015");
        let lib_path = manifest
            .get("lib")
            .and_then(|lib| lib.get("path"))
            .and_then(toml::Value::as_str)
            .unwrap_or("src/lib.rs");
        let lib_path = normalize_relative(directory, lib_path)?;
        if paths.contains(lib_path.as_str()) {
            let lib_name = manifest
                .get("lib")
                .and_then(|lib| lib.get("name"))
                .and_then(toml::Value::as_str)
                .unwrap_or(name);
            crates.push(CrateInput {
                name: lib_name.replace('-', "_"),
                root_file: lib_path.clone(),
                edition: edition.into(),
                dependencies: BTreeMap::new(),
            });
            package_libraries.insert(manifest_path.clone(), lib_path.clone());
            crate_manifests.insert(lib_path, manifest_path.clone());
        }
        let mut bins = BTreeMap::new();
        let main_path = normalize_relative(directory, "src/main.rs")?;
        if package.get("autobins").and_then(toml::Value::as_bool) != Some(false)
            && paths.contains(main_path.as_str())
        {
            bins.insert(main_path, name.to_owned());
        }
        if let Some(explicit_bins) = manifest.get("bin").and_then(toml::Value::as_array) {
            for bin in explicit_bins {
                if let Some(path) = bin.get("path").and_then(toml::Value::as_str) {
                    bins.insert(
                        normalize_relative(directory, path)?,
                        bin.get("name")
                            .and_then(toml::Value::as_str)
                            .unwrap_or(name)
                            .to_owned(),
                    );
                }
            }
        }
        for (root, bin_name) in bins {
            if paths.contains(root.as_str()) {
                crates.push(CrateInput {
                    name: bin_name.replace('-', "_"),
                    root_file: root.clone(),
                    edition: edition.into(),
                    dependencies: BTreeMap::new(),
                });
                crate_manifests.insert(root, manifest_path.clone());
            } else {
                add_gap(
                    &mut coverage,
                    UnknownReason::MissingDependency,
                    format!("missing crate root: {root}"),
                );
            }
        }
        if manifest.get("build-dependencies").is_some()
            || package.get("build").is_some()
            || paths.contains(normalize_relative(directory, "build.rs")?.as_str())
        {
            add_gap(
                &mut coverage,
                UnknownReason::MacroUnavailable,
                format!("build-script inputs unavailable: {manifest_path}"),
            );
        }
    }
    for krate in &mut crates {
        let manifest_path = &crate_manifests[&krate.root_file];
        let manifest = &manifests[manifest_path];
        let directory = Path::new(manifest_path).parent().unwrap_or(Path::new(""));
        if let Some(lib_root) = package_libraries
            .get(manifest_path)
            .filter(|root| **root != krate.root_file)
        {
            let lib_name = manifest
                .get("lib")
                .and_then(|lib| lib.get("name"))
                .and_then(toml::Value::as_str)
                .or_else(|| manifest.get("package")?.get("name")?.as_str())
                .unwrap_or(&krate.name)
                .replace('-', "_");
            krate.dependencies.insert(lib_name, lib_root.clone());
        }
        if let Some(dependencies) = manifest.get("dependencies").and_then(toml::Value::as_table) {
            for (alias, declared) in dependencies {
                let inherited =
                    declared.get("workspace").and_then(toml::Value::as_bool) == Some(true);
                let dependency = if inherited {
                    manifests
                        .get("Cargo.toml")
                        .and_then(|root| root.get("workspace"))
                        .and_then(|ws| ws.get("dependencies"))
                        .and_then(|deps| deps.get(alias))
                        .unwrap_or(declared)
                } else {
                    declared
                };
                if declared.get("optional").and_then(toml::Value::as_bool) == Some(true)
                    || dependency.get("optional").and_then(toml::Value::as_bool) == Some(true)
                {
                    add_gap(
                        &mut coverage,
                        UnknownReason::CfgUnknown,
                        format!("optional dependency requires Cargo feature resolution: {alias}"),
                    );
                    continue;
                }
                let dep_directory = if inherited { Path::new("") } else { directory };
                let target_manifest = dependency
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .and_then(|path| {
                        normalize_relative(dep_directory, &format!("{path}/Cargo.toml")).ok()
                    });
                if let Some(target_root) = target_manifest
                    .as_ref()
                    .and_then(|path| package_libraries.get(path))
                {
                    krate
                        .dependencies
                        .insert(alias.replace('-', "_"), target_root.clone());
                } else {
                    add_gap(
                        &mut coverage,
                        UnknownReason::MissingDependency,
                        format!("uncaptured dependency {alias} in {manifest_path}"),
                    );
                }
            }
        }
        if manifest.get("target").is_some() {
            add_gap(
                &mut coverage,
                UnknownReason::CfgUnknown,
                format!("target-specific dependencies require explicit context: {manifest_path}"),
            );
        }
    }
    if crates.is_empty() {
        add_gap(
            &mut coverage,
            UnknownReason::MissingDependency,
            "No Cargo crate roots found; loose Rust files receive syntax analysis only.".into(),
        );
    }
    crates.sort_by(|a, b| a.root_file.cmp(&b.root_file));
    let mut features = options.features.clone();
    features.sort();
    features.dedup();
    let mut context = BuildContext {
        id: ContextId::default(),
        name: options.profile.clone(),
        target: options.target.clone(),
        features,
        default_features: options.default_features,
        cfg: options.cfg.clone(),
        crates,
        manifest_digest,
        trust: "read_only".into(),
        coverage,
    };
    validate_crates(source, &context.crates)?;
    context.id = context_id(&context);
    Ok(context)
}

fn context_id(context: &BuildContext) -> ContextId {
    ContextId(digest(
        "context",
        &(
            &context.name,
            &context.target,
            &context.features,
            context.default_features,
            &context.cfg,
            &context.crates,
            &context.manifest_digest,
            &context.trust,
        ),
    ))
}

fn validate_crates(source: &SourceSnapshot, crates: &[CrateInput]) -> Result<()> {
    let files = source
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut roots = BTreeSet::new();
    for krate in crates {
        ensure!(
            files.contains(krate.root_file.as_str()),
            "crate root not captured: {}",
            krate.root_file
        );
        ensure!(
            roots.insert(&krate.root_file),
            "duplicate crate root: {}",
            krate.root_file
        );
        ensure!(
            matches!(krate.edition.as_str(), "2015" | "2018" | "2021" | "2024"),
            "unsupported Rust edition: {}",
            krate.edition
        );
    }
    for krate in crates {
        for target in krate.dependencies.values() {
            ensure!(
                roots.contains(target),
                "dependency root not in context: {target}"
            );
        }
    }
    Ok(())
}

fn normalize_relative(base: &Path, value: &str) -> Result<String> {
    let mut path = PathBuf::new();
    for component in base.join(value).components() {
        match component {
            Component::Normal(name) => path.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(path.pop(), "path escapes captured source root: {value}");
            }
            _ => bail!("absolute source path is not allowed: {value}"),
        }
    }
    path.into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("non-UTF-8 context path"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture;
    use std::fs;
    #[test]
    fn workspace_edition_opt_in_and_member_optional_dependency_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers=['member','helper']\n[workspace.package]\nedition='2024'\n[workspace.dependencies]\nhelper={path='helper'}\n").unwrap();
        for name in ["member", "helper"] {
            fs::create_dir_all(dir.path().join(name).join("src")).unwrap();
            fs::write(
                dir.path().join(name).join("src/lib.rs"),
                "pub fn entry() {}\n",
            )
            .unwrap();
        }
        fs::write(dir.path().join("member/Cargo.toml"), "[package]\nname='member'\nversion='0.1.0'\n[dependencies]\nhelper={workspace=true,optional=true}\n").unwrap();
        fs::write(
            dir.path().join("helper/Cargo.toml"),
            "[package]\nname='helper'\nversion='0.1.0'\nedition.workspace=true\n",
        )
        .unwrap();
        let (_, context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
        let member = context
            .crates
            .iter()
            .find(|krate| krate.name == "member")
            .unwrap();
        let helper = context
            .crates
            .iter()
            .find(|krate| krate.name == "helper")
            .unwrap();
        assert_eq!(member.edition, "2015");
        assert_eq!(helper.edition, "2024");
        assert!(!member.dependencies.contains_key("helper"));
        assert!(
            context
                .coverage
                .reasons
                .iter()
                .any(|reason| reason.reason == UnknownReason::CfgUnknown)
        );
    }

    #[test]
    fn explicit_context_validates_roots_and_canonicalizes_order() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("left.rs"), "pub fn left() {}\n").unwrap();
        fs::write(dir.path().join("right.rs"), "pub fn right() {}\n").unwrap();
        let (_, mut context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
        context.crates = ["left", "right"]
            .map(|name| CrateInput {
                name: name.into(),
                root_file: format!("{name}.rs"),
                edition: "2021".into(),
                dependencies: BTreeMap::new(),
            })
            .to_vec();
        context.features = vec!["fast".into(), "slow".into(), "fast".into()];
        let first = capture(
            dir.path(),
            &CaptureOptions {
                explicit_context: Some(context.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        context.crates.reverse();
        context.features.reverse();
        let second = capture(
            dir.path(),
            &CaptureOptions {
                explicit_context: Some(context.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.1.id, second.1.id);
        context.crates[0]
            .dependencies
            .insert("missing".into(), "missing.rs".into());
        assert!(
            capture(
                dir.path(),
                &CaptureOptions {
                    explicit_context: Some(context),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn path_dependencies_preserve_aliases() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("helper/src")).unwrap();
        fs::write(
            dir.path().join("helper/Cargo.toml"),
            "[package]\nname='helper'\nversion='0.1.0'\nedition='2021'\n",
        )
        .unwrap();
        fs::write(dir.path().join("helper/src/lib.rs"), "pub fn helper() {}\n").unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nalias={package='helper',path='helper'}\nexternal='1'\n").unwrap();
        let (_, context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
        let root = context
            .crates
            .iter()
            .find(|krate| krate.root_file == "src/lib.rs")
            .unwrap();
        assert_eq!(root.dependencies["alias"], "helper/src/lib.rs");
        assert!(!root.dependencies.contains_key("external"));
        assert!(
            context
                .coverage
                .limitations
                .iter()
                .any(|text| text.contains("external"))
        );
        let mut invalid = context.clone();
        invalid.trust = "execute".into();
        assert!(
            capture(
                dir.path(),
                &CaptureOptions {
                    explicit_context: Some(invalid),
                    ..Default::default()
                }
            )
            .is_err()
        );
        invalid = context;
        invalid.crates[0].root_file = "../outside.rs".into();
        assert!(
            capture(
                dir.path(),
                &CaptureOptions {
                    explicit_context: Some(invalid),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
