//! Keeps the committed C header in sync with the exports.

use std::env;
use std::fs;
use std::path::PathBuf;

#[test]
fn committed_header_matches_the_exports() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml")).unwrap();
    let mut generated = Vec::new();
    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen generates the header")
        .write(&mut generated);
    let generated = String::from_utf8(generated).unwrap();

    let path = crate_dir.join("include/perldantic.h");
    if env::var_os("PERLDANTIC_UPDATE_HEADER").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &generated).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "ffi/include/perldantic.h is stale; rerun with PERLDANTIC_UPDATE_HEADER=1"
    );
}
