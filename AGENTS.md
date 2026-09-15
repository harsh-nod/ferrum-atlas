# Ferrum Atlas

Implement the source specification in `rust_analyzer_and_visualizer.md` in measured stages.
Never execute an analyzed repository's tools, build scripts, wrappers, or macros in read-only mode.
Use explicit evidence and limitations; lexical matches are not semantic resolution.
Do not copy private source or benchmark corpora into this public repository.

The integration lead owns root manifests, lockfiles, model contracts, server, CLI, and integration.
Work in assigned isolated worktrees. Request shared-contract changes from the lead.
Each agent owns tests for its work; another agent reviews it after integration.
Commit focused changes. The lead integrates and pushes public progress.
Run `cargo fmt --all --check`, targeted tests, then workspace tests and Clippy at integration.
The viewer uses generated model types, React, CodeMirror, Cytoscape and ELK worker layout.

Do not claim large-scale qualification or semantic completeness from small fixtures.
No Qwen workers unless explicitly requested by the user.
