use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    for path in [
        "apps",
        "crates",
        "skills",
        "schemas",
        "docs",
        "scripts",
        "examples",
        "fixtures/evaluation",
        "Cargo.toml",
        "Cargo.lock",
        "README.md",
        "CHANGELOG.md",
        ".github",
        ".git/HEAD",
        ".git/index",
        ".git/packed-refs",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    println!("cargo:rerun-if-env-changed=OKF_GIT");
    let git = env::var_os("OKF_GIT").unwrap_or_else(|| "git".into());
    let output = |args: &[&str]| {
        Command::new(&git)
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_owned())
    };
    if let Some(reference) = output(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={}", root.join(reference).display());
    }
    if let Some(reference) = output(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = output(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
    let commit = output(&["rev-parse", "HEAD"])
        .filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let dirty = output(&["status", "--porcelain", "--untracked-files=normal"])
        .map(|s| (!s.is_empty()).to_string())
        .unwrap_or_else(|| "unknown".into());
    println!(
        "cargo:rustc-env=OKF_BUILD_COMMIT={}",
        commit.unwrap_or_else(|| "unknown".into())
    );
    println!("cargo:rustc-env=OKF_BUILD_DIRTY={dirty}");
    println!(
        "cargo:rustc-env=OKF_BUILD_TARGET={}",
        env::var("TARGET").unwrap()
    );
}
