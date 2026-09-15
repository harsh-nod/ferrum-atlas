use crate::configuration::Configurations;
use anyhow::{Context, Result, anyhow};
use atlas_model::{BuildContext, SourceSnapshot};
use ra_ap_base_db::{
    CrateGraphBuilder, CrateName, CrateOrigin, CrateWorkspaceData, DependencyBuilder, Env,
    SourceRoot,
};
use ra_ap_ide_db::{ChangeWithProcMacros, RootDatabase};
use ra_ap_intern::Symbol;
use ra_ap_span::Edition;
use ra_ap_vfs::{AbsPathBuf, FileId, VfsPath, file_set::FileSet};
use std::collections::BTreeMap;
use triomphe::Arc;

pub(super) fn load(
    source: &SourceSnapshot,
    context: &BuildContext,
    configurations: &Configurations,
) -> Result<RootDatabase> {
    let mut change = ChangeWithProcMacros::default();
    let mut files = FileSet::default();
    let mut ids = BTreeMap::new();
    for (index, file) in source.files.iter().enumerate() {
        let id = FileId::from_raw(index as u32);
        ids.insert(file.path.as_str(), id);
        files.insert(id, VfsPath::new_virtual_path(format!("/{}", file.path)));
        change.change_file(id, Some(file.text.clone()));
    }
    change.set_roots(vec![SourceRoot::new_local(files)]);
    let mut graph = CrateGraphBuilder::default();
    let mut crates = BTreeMap::new();
    for krate in &context.crates {
        let root = *ids
            .get(krate.root_file.as_str())
            .context("crate root missing from source")?;
        let edition = krate
            .edition
            .parse::<Edition>()
            .map_err(|err| anyhow!("invalid edition: {err}"))?;
        let cfg = configurations.roots[&krate.root_file].options();
        let name = CrateName::normalize_dashes(&krate.name);
        let id = graph.add_crate_root(
            root,
            edition,
            Some(name.into()),
            None,
            cfg,
            None,
            Env::default(),
            CrateOrigin::Local {
                repo: None,
                name: Some(Symbol::intern(&krate.name)),
            },
            vec![],
            false,
            Arc::new(AbsPathBuf::assert("/".into())),
            Arc::new(CrateWorkspaceData {
                target: Err("target layout was not captured".into()),
                toolchain: None,
            }),
        );
        crates.insert(krate.root_file.as_str(), id);
    }
    for krate in &context.crates {
        for (alias, root) in &krate.dependencies {
            let target = *crates
                .get(root.as_str())
                .context("dependency root missing from context")?;
            graph
                .add_dep(
                    crates[krate.root_file.as_str()],
                    DependencyBuilder::new(CrateName::normalize_dashes(alias), target),
                )
                .map_err(|err| anyhow!("invalid crate dependency graph: {err}"))?;
        }
    }
    change.set_crate_graph(graph);
    let mut db = RootDatabase::new(Some(128));
    db.apply_change(change);
    Ok(db)
}
