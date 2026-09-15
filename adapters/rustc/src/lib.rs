#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

mod capture;
#[path = "../../../crates/model/src/compiler.rs"]
pub mod compiler;
mod extract;

use anyhow::{Context, Result, bail, ensure};
use compiler::*;
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf, sync::Arc};

pub const PHASE: &str = "runtime_optimized";
pub const TARGET: &str = "x86_64-unknown-linux-gnu";
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct Options {
    pub root: PathBuf,
    pub source: PathBuf,
    pub output: PathBuf,
    pub crate_name: String,
    pub edition: String,
    pub panic_strategy: String,
    pub cfg: Vec<String>,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mut trusted = false;
        let mut root = None;
        let mut source = None;
        let mut output = None;
        let mut crate_name = "atlas_input".to_owned();
        let mut edition = "2021".to_owned();
        let mut panic_strategy = "unwind".to_owned();
        let mut cfg = Vec::new();
        while let Some(flag) = args.next() {
            if flag == "--trusted-local" {
                trusted = true;
                continue;
            }
            let value = args.next().context("option requires a value")?;
            match flag.as_str() {
                "--root" => root = Some(PathBuf::from(value)),
                "--source" => source = Some(PathBuf::from(value)),
                "--output" => output = Some(PathBuf::from(value)),
                "--crate-name" => crate_name = value,
                "--edition" => edition = value,
                "--panic" => panic_strategy = value,
                "--cfg" if cfg.len() < 256 => cfg.push(value),
                _ => bail!("unsupported option: {flag}"),
            }
        }
        ensure!(
            trusted,
            "compiler extraction requires explicit --trusted-local"
        );
        ensure!(
            matches!(edition.as_str(), "2015" | "2018" | "2021" | "2024"),
            "unsupported edition"
        );
        ensure!(
            matches!(panic_strategy.as_str(), "unwind" | "abort"),
            "unsupported panic strategy"
        );
        ensure!(
            !crate_name.is_empty()
                && crate_name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "invalid crate name"
        );
        ensure!(
            crate_name.len() <= 128 && cfg.iter().all(|value| value.len() <= 4096),
            "option budget exceeded"
        );
        let root = fs::canonicalize(root.context("--root is required")?)?;
        ensure!(root.is_dir(), "source root is not a directory");
        let source = fs::canonicalize(root.join(source.context("--source is required")?))?;
        ensure!(
            source.starts_with(&root) && source.is_file(),
            "source must be a regular file inside --root"
        );
        let output = output.context("--output is required")?;
        let output = if output.is_absolute() {
            output
        } else {
            std::env::current_dir()?.join(output)
        };
        ensure!(!output.exists(), "output already exists");
        cfg.sort();
        cfg.dedup();
        Ok(Self {
            root,
            source,
            output,
            crate_name,
            edition,
            panic_strategy,
            cfg,
        })
    }
}

pub fn compiler_identity() -> CompilerIdentity {
    CompilerIdentity {
        adapter: "atlas-rustc".into(),
        adapter_version: env!("CARGO_PKG_VERSION").into(),
        release: env!("ATLAS_RUSTC_RELEASE").into(),
        commit_hash: env!("ATLAS_RUSTC_COMMIT").into(),
        commit_date: env!("ATLAS_RUSTC_DATE").into(),
        host: env!("ATLAS_RUSTC_HOST").into(),
        llvm_version: env!("ATLAS_RUSTC_LLVM").into(),
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn run(options: Options) -> Result<()> {
    ensure!(
        env!("ATLAS_RUSTC_HOST") == TARGET,
        "unsupported compiler host"
    );
    let source = options
        .source
        .strip_prefix(&options.root)?
        .to_str()
        .context("source path is not UTF-8")?
        .to_owned();
    let capture = Arc::new(capture::Capture::new(options.root.clone()));
    let mut args = vec![
        "atlas-rustc".into(),
        source.clone(),
        "--crate-name".into(),
        options.crate_name.clone(),
        "--crate-type=lib".into(),
        format!("--edition={}", options.edition),
        format!("--target={TARGET}"),
        format!("-Cpanic={}", options.panic_strategy),
        "-Copt-level=0".into(),
        "-Coverflow-checks=yes".into(),
        "-Zmir-opt-level=0".into(),
        "--emit=metadata".into(),
        "--sysroot".into(),
        env!("ATLAS_RUSTC_SYSROOT").into(),
        "--error-format=json".into(),
    ];
    for cfg in &options.cfg {
        args.push("--cfg".into());
        args.push(cfg.clone());
    }
    let mut callback = Extractor {
        capture,
        result: None,
    };
    rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(&args, &mut callback))
        .map_err(|_| anyhow::anyhow!("compiler rejected input; no bundle published"))?;
    let mut bodies = callback
        .result
        .context("compiler did not reach extraction")??;
    let mut inputs = CompilerInputs {
        manifest_hash: String::new(),
        files: callback.capture.files(),
        crate_name: options.crate_name,
        crate_root: source,
        edition: options.edition,
        target: TARGET.into(),
        panic_strategy: options.panic_strategy,
        mir_opt_level: 0,
        rustc_args: args,
        environment_policy: "empty".into(),
        trust: "trusted_local".into(),
        compiled_artifact: None,
    };
    let compiler = compiler_identity();
    inputs.manifest_hash = sha256(&serde_json::to_vec(&(&compiler, PHASE, &inputs))?);
    let mut body_paths = std::collections::BTreeSet::new();
    for body in &mut bodies {
        ensure!(
            body_paths.insert(body.def_path.clone()),
            "ambiguous compiler item path"
        );
        body.body_id = format!(
            "mir:{}",
            sha256(&serde_json::to_vec(&(
                &inputs.manifest_hash,
                &body.def_path
            ))?)
        );
    }
    let bundle = CompilerBundle {
        schema_version: COMPILER_SCHEMA_VERSION,
        compiler,
        inputs,
        phase: PHASE.into(),
        bodies,
        limitations: vec![
            "Trusted local execution is not an OS sandbox; do not use with hostile sources.".into(),
            "One explicitly selected library crate and installed pinned sysroot; no Cargo orchestration, dependency acquisition, build-script integration, or user-supplied procedural macro artifacts.".into(),
            "Runtime MIR after compiler lowering at mir-opt-level=0; not source flow, borrow-checker MIR, or generated machine code.".into(),
            "Local accesses are syntactic MIR facts, not reaching definitions or interprocedural dataflow; unknown effects must propagate conservatively.".into(),
            "FunctionDefinition call targets identify generic compiler items, not selected trait implementations or monomorphized runtime instances.".into(),
            "Constants, statics, promoted bodies, generated shims, and external bodies are not enumerated; coroutine lowering can hide original suspension points.".into(),
            "Macro-expanded and imported spans are unavailable rather than guessed source locations.".into(),
        ],
    };
    let parent = options.output.parent().context("output has no parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = BoundedWriter {
            inner: temporary.as_file_mut(),
            remaining: MAX_OUTPUT_BYTES,
        };
        serde_json::to_writer(&mut writer, &bundle)?;
        writer.write_all(b"\n")?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(&options.output)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

struct Extractor {
    capture: Arc<capture::Capture>,
    result: Option<Result<Vec<CompilerBody>>>,
}

impl Callbacks for Extractor {
    fn config(&mut self, config: &mut interface::Config) {
        config.file_loader = Some(Box::new(capture::Loader(self.capture.clone())));
    }

    fn after_analysis<'tcx>(&mut self, _: &interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        self.result = Some(extract::extract(tcx, &self.capture));
        Compilation::Stop
    }
}

struct BoundedWriter<W> {
    inner: W,
    remaining: usize,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other("compiler output budget exceeded"));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_output_fails_without_accepting_over_budget_bytes() {
        let mut output = BoundedWriter {
            inner: Vec::new(),
            remaining: 3,
        };
        output.write_all(b"ab").unwrap();
        assert!(output.write_all(b"cd").is_err());
        assert_eq!(output.inner, b"ab");
        output.write_all(b"c").unwrap();
        assert_eq!(output.remaining, 0);
    }
}
