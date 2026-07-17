use std::{env, fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let guest_source = manifest_dir.join("../../guests/smoke-write/smoke-write.wat");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("out dir")).join("smoke_write.wasm");

    println!("cargo:rerun-if-changed={}", guest_source.display());

    let wasm = wat::parse_file(&guest_source).expect("compile smoke-write guest WAT");
    fs::write(output, wasm).expect("write compiled smoke-write guest");
}
