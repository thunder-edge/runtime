pub const RUNTIME_BASE_STARTUP_SNAPSHOT: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/runtime_base.snapshot.bin"));

include!(concat!(
    env!("OUT_DIR"),
    "/runtime_base_residual_sources.rs"
));
