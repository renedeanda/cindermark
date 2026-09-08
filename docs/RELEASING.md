# Releasing Cindermark

Releases have separate Rust and Apple distribution steps. Review source changes
through a PR before preparing artifacts. Never publish from a dirty checkout.

## Source checks

Run `cargo fmt --check`, default and `ffi` tests, default and `bindgen` strict
Clippy, documentation tests, and benchmark compilation. Build the WASM feature
for `wasm32-unknown-unknown`. Regenerate Swift bindings into a temporary directory
and compare them with the committed bindings.

Run `cargo package --list` and `cargo publish --dry-run`; inspect the archive for
unrelated files or private data. Test a separate consumer against the packaged
crate. Check the no-default-feature dependency tree, including build dependencies.
Record toolchain, source revision, performance results and untested platforms.

Update the changelog, migration guide and installation documentation together.
Keep published installation examples on an available version until publication.

## Apple release

The manual `release.yml` workflow prepares optimized Apple slices, the XCFramework
and its checksum on `release/vVERSION`. A maintainer opens the metadata PR from
that branch; the workflow does not require Actions permission to create PRs.
Review release metadata and generated bindings before tagging. Verify the final
source differs from the artifact source only in reviewed release metadata.
Record both revisions and the artifact checksum. Download and retain the run's
artifact before its 30-day retention expires.

Publish only after verifying the merged manifest URL and checksum match the
prepared artifact. Test SwiftPM resolution from the actual tag on supported
platforms. Do not move an existing release tag or overwrite an asset to repair
a release; document the failure and use a new version when necessary.

The artifact preparation workflow intentionally does not publish. After the
metadata PR merges, create the version tag at that exact reviewed revision and
attach the prepared XCFramework ZIP and provenance to the GitHub release.
Resolve and build a fresh Swift consumer before describing Apple distribution
as available. Confirm the paid-run cost before dispatching preparation.

## Rust publication

Verify crates.io access and version availability without printing credentials.
From the verified release checkout, run `cargo publish --dry-run`, then
`cargo publish --locked`. Verify a fresh consumer can install that registry
version and that docs.rs completes successfully. Publishing the Apple release
does not publish the Rust crate.

## Handoff

Report Rust publication, Apple publication and WASM artifacts independently.
Include source/artifact provenance, installation checks and outstanding manual
acceptance. A downstream application can use reviewed release-mode source
artifacts without waiting for registry publication.
