fn main() {
    #[cfg(feature = "ffi")]
    uniffi::generate_scaffolding("src/cindermark.udl")
        .expect("failed to generate UniFFI scaffolding");
}
