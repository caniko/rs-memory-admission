# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.7] - 2026-05-22

### Fixed

- Repair weighted-gate rustdoc links for the async admission API so release docs build cleanly.
- Align release tooling with the current crate surface by checking clippy with all features in the flake and publish workflow.
- Add dependency audit and deny-policy configuration to local release hooks and Forgejo publish validation.

### Changed

- Refresh Forgejo release workflow messaging and shell formatting for clearer tag and token validation failures.
- Update release documentation examples to point at crate version `0.1.7`.

[Unreleased]: https://codeberg.org/caniko/rs-memory-admission/compare/0.1.7...HEAD
[0.1.7]: https://codeberg.org/caniko/rs-memory-admission/compare/0.1.6...0.1.7
