fn main() {
    // Only generate UniFFI scaffolding when the `ffi` feature is enabled.
    // Pure-Rust consumers (the default) skip this entirely, so they don't run
    // the bindings codegen. Cargo exposes enabled features to build scripts as
    // CARGO_FEATURE_<NAME> environment variables.
    if std::env::var_os("CARGO_FEATURE_FFI").is_some() {
        uniffi::generate_scaffolding("src/cindermark.udl").ok();
    }
}
