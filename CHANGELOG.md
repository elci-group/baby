# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Structured `BabyError` type with `ErrorKind` classification.
- Unit tests for `error`, `config`, and `lib` modules.
- Integration tests for `baby`, `birthctl`, and `birthd` CLIs.
- Custom `logger` module replacing `env_logger` with `std`-only timestamp formatting.
- `README.md`, `CONTRIBUTING.md`, and this `CHANGELOG.md`.
- Generated man pages under `man/`.
- Brandi genome (`.brandi/`) and Skillastic skill (`.skillastic/skills/baby-dev.md`).

### Changed

- Replaced shell-outs to `mkdir`, `cp`, and `install` with `std::fs` operations.
- Replaced `chrono` with a custom UTC timestamp formatter.
- `birthd` now uses the shared logger and writes to both stderr and its log file.

### Removed

- Dependency on `chrono`.
- Dependency on `env_logger`.

## [0.2.0] — 2026-08-07

### Added

- Kaptaind monitoring configuration (`kaptaind.toml`, `.kaptainignore`).
- `.gitignore` for build artefacts and generated reports.

### Changed

- Auto-fixed `clippy` lints (`collapsible_if`, `derivable_impls`).
- Applied `cargo fmt` across the workspace.

## [0.1.0] — 2026-08-07

### Added

- Initial release: `baby`, `birthctl`, and `birthd` binaries.
- `InstallConfig` and `ProjectConfig` types.
- Filesystem watching via `notify`.
- Signal-based IPC for daemon control.
- XDG path helpers and PID-file management.
