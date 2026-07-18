use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

struct CatalogEntry {
    executable: String,
    directory: String,
    output_name: String,
    rust_symbol: String,
}

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    let guest_root = manifest_dir.join("../../guests");
    let catalog_path = guest_root.join("catalog.tsv");
    println!("cargo:rerun-if-changed={}", catalog_path.display());
    let catalog_source = fs::read_to_string(&catalog_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", catalog_path.display()));
    let catalog = parse_catalog(&catalog_source);

    for entry in &catalog {
        compile_guest(
            &guest_root,
            &output_dir,
            &entry.directory,
            &entry.output_name,
        );
    }
    for (directory, output_name) in [
        ("smoke-write", "smoke_write.wasm"),
        ("pwd", "pwd.wasm"),
        ("write-file", "write_file.wasm"),
        ("cat", "cat.wasm"),
        ("guest-pipeline", "guest_pipeline.wasm"),
        ("mini-shell", "mini_shell.wasm"),
    ] {
        compile_guest(&guest_root, &output_dir, directory, output_name);
    }

    let mut generated = String::from("const SIGNED_CATALOG: &[SignedCatalogEntry] = &[\n");
    for entry in catalog {
        generated.push_str(&format!(
            "    SignedCatalogEntry {{ executable: {:?}, wasm: {} }},\n",
            entry.executable, entry.rust_symbol
        ));
    }
    generated.push_str("];\n");
    fs::write(output_dir.join("signed_catalog.rs"), generated)
        .expect("write generated signed catalog");
}

fn compile_guest(guest_root: &Path, output_dir: &Path, directory: &str, output_name: &str) {
    let guest_source = guest_root.join(directory).join(format!("{directory}.wat"));
    println!("cargo:rerun-if-changed={}", guest_source.display());
    let wasm = wat::parse_file(&guest_source)
        .unwrap_or_else(|error| panic!("compile {}: {error}", guest_source.display()));
    fs::write(output_dir.join(output_name), wasm)
        .unwrap_or_else(|error| panic!("write {output_name}: {error}"));
}

fn parse_catalog(source: &str) -> Vec<CatalogEntry> {
    let mut entries = Vec::new();
    let mut executables = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(
            fields.len(),
            4,
            "catalog line {line_number} must contain four tab-separated fields"
        );
        let [executable, directory, output_name, rust_symbol] =
            <[&str; 4]>::try_from(fields).expect("field count checked");
        assert!(
            valid_executable(executable),
            "catalog line {line_number} has an invalid absolute executable path"
        );
        assert!(
            valid_token(directory, false, true),
            "catalog line {line_number} has an invalid guest directory"
        );
        assert!(
            output_name.ends_with(".wasm") && valid_token(output_name, true, false),
            "catalog line {line_number} has an invalid output name"
        );
        assert!(
            valid_symbol(rust_symbol),
            "catalog line {line_number} has an invalid Rust byte symbol"
        );
        assert!(
            executables.insert(executable),
            "catalog line {line_number} duplicates executable {executable}"
        );
        assert!(
            directories.insert(directory),
            "catalog line {line_number} duplicates guest directory {directory}"
        );
        assert!(
            outputs.insert(output_name),
            "catalog line {line_number} duplicates output {output_name}"
        );
        assert!(
            symbols.insert(rust_symbol),
            "catalog line {line_number} duplicates Rust symbol {rust_symbol}"
        );
        entries.push(CatalogEntry {
            executable: executable.to_string(),
            directory: directory.to_string(),
            output_name: output_name.to_string(),
            rust_symbol: rust_symbol.to_string(),
        });
    }
    assert!(
        !entries.is_empty(),
        "signed executable catalog cannot be empty"
    );
    entries
}

fn valid_executable(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 256
        && value[1..].split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn valid_token(value: &str, allow_dot: bool, allow_hyphen: bool) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'_'
                || (allow_dot && byte == b'.')
                || (allow_hyphen && byte == b'-')
        })
}

fn valid_symbol(value: &str) -> bool {
    value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_uppercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}
