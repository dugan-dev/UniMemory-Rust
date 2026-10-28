//! Locate an existing prefixed jemalloc SDK for the MSVC target.
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=UNIMEMORY_JEMALLOC_DIR");
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
    if env::var_os("CARGO_FEATURE_JEMALLOC").is_none()
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("UNIMEMORY_JEMALLOC_DIR") {
        candidates.push(PathBuf::from(path));
    } else if let Some(root) = env::var_os("VCPKG_ROOT") {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture");
        let triplet = match arch.as_str() {
            "x86_64" => "x64-windows",
            "aarch64" => "arm64-windows",
            _ => panic!("Set UNIMEMORY_JEMALLOC_DIR for this MSVC architecture"),
        };
        let installed = PathBuf::from(root).join("installed");
        candidates.push(installed.join(format!("{triplet}-unimemory")));
        candidates.push(installed.join(triplet));
    }
    let prefix = candidates.into_iter().find(|p| p.join("lib/jemalloc.lib").is_file() || p.join("lib/jemalloc_s.lib").is_file())
        .expect("jemalloc on MSVC needs a je_-prefixed native SDK: set UNIMEMORY_JEMALLOC_DIR to its installation prefix (lib/jemalloc.lib); add its bin directory to PATH when running");
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    let static_library = prefix.join("lib/jemalloc_s.lib");
    if static_library.is_file() {
        println!("cargo:rustc-link-lib=static=jemalloc_s");
    } else {
        println!("cargo:rustc-link-lib=jemalloc");
    }
    let linked_library = if static_library.is_file() {
        static_library
    } else {
        prefix.join("lib/jemalloc.lib")
    };
    println!("cargo:rerun-if-changed={}", linked_library.display());
}
