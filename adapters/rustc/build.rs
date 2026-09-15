use std::{env, process::Command};

fn main() {
    let rustc = env::var_os("RUSTC").expect("Cargo supplies RUSTC");
    let output = Command::new(&rustc)
        .arg("-vV")
        .output()
        .expect("rustc identity");
    assert!(output.status.success());
    let identity = String::from_utf8(output.stdout).unwrap();
    for (field, variable) in [
        ("commit-hash", "ATLAS_RUSTC_COMMIT"),
        ("commit-date", "ATLAS_RUSTC_DATE"),
        ("host", "ATLAS_RUSTC_HOST"),
        ("release", "ATLAS_RUSTC_RELEASE"),
        ("LLVM version", "ATLAS_RUSTC_LLVM"),
    ] {
        let value = identity
            .lines()
            .find_map(|line| {
                line.strip_prefix(field)
                    .and_then(|tail| tail.strip_prefix(": "))
            })
            .expect("rustc identity field");
        if field == "commit-hash" {
            assert_eq!(
                value, "55e86c996809902e8bbad512cfb4d2c18be446d9",
                "unsupported compiler commit"
            );
        }
        println!("cargo:rustc-env={variable}={value}");
    }
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let sysroot = String::from_utf8(output.stdout).unwrap();
    let sysroot = sysroot.trim();
    println!("cargo:rustc-env=ATLAS_RUSTC_SYSROOT={sysroot}");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{sysroot}/lib");
    println!("cargo:rerun-if-changed=build.rs");
}
