# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial public release of Cindermark, extracted from the Ember Notes
  editor engine: single-pass block + inline Markdown parser with UTF-16
  offsets, incremental dirty-block re-parsing, and UniFFI Swift bindings.
- `build-apple.sh` for building Apple static libraries + Swift bindings.
- Swift Package (`Package.swift`) with a binary XCFramework target.
- Configurable image-marker URI scheme (off by default; host apps opt in,
  e.g. Ember Notes passes `"ember:"`).

### Changed
- Eliminated redundant full block-list clones from the incremental parse
  FFI paths (up to 3 extra full passes per keystroke removed).
