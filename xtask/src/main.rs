use anyhow::{Result, bail};
fn main() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let path = root.join("web/src/api/types.ts");
    let expected = atlas_model::typescript();
    match std::env::args().nth(1).as_deref() {
        Some("types") => {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, expected)?;
        }
        Some("check-types") => {
            if std::fs::read_to_string(path)? != expected {
                bail!("transport types are stale; run cargo run -p xtask -- types");
            }
        }
        _ => bail!("usage: cargo run -p xtask -- <types|check-types>"),
    }
    Ok(())
}
