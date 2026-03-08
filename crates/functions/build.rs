use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let snapshot_path = out_dir.join("runtime_base.snapshot.bin");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let manifest_dir_static: &'static str = Box::leak(manifest_dir.into_boxed_str());

    let snapshot_output = runtime_core::extensions::create_runtime_base_snapshot(manifest_dir_static)
        .expect("failed to create runtime base snapshot");

    fs::write(&snapshot_path, &snapshot_output.output).expect("failed to write runtime base snapshot");

    for file in snapshot_output.files_loaded_during_snapshot {
        println!("cargo:rerun-if-changed={}", file.display());
    }
}
