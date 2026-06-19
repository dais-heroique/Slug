//! Generate the C header (`include/slug_ui.h`) from the `#[no_mangle]` exports
//! with cbindgen, mirroring AccessKit's C binding build.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = crate_dir.join("include").join("slug_ui.h");

    let config = cbindgen::Config::from_root_or_default(&crate_dir);
    match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(bindings) => {
            let _ = std::fs::create_dir_all(crate_dir.join("include"));
            bindings.write_to_file(&out);
        }
        // Never fail the build just because header generation hiccuped.
        Err(e) => println!("cargo:warning=cbindgen: {e}"),
    }
}
