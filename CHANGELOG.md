# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `baby boom` now runs end-to-end (discover → detect → confirm/select →
  execute → report); it previously only logged a "under development"
  stub.
- Live animated progress for `baby boom`: a three-zone grid
  (Requirements | Compiled | Errors) redrawn in place on a TTY, built on
  `form3`'s table/animation primitives, with packages moving out of
  Requirements as they settle. Off a TTY (piped output, CI), the same
  transitions are logged one line per event instead.
- Deterministic `ErrorKind` variants for common `boom` build/install
  failures (git auth, DNS/network unreachable, repo not found, disk
  full, missing build toolchain, install permission denied), each with
  specific remediation text, classified from command stderr.
- Structured `BabyError` type with `ErrorKind` classification.
- Unit tests for `error`, `config`, and `lib` modules.
- Integration tests for `baby`, `birthctl`, and `birthd` CLIs.
- Custom `logger` module replacing `env_logger` with `std`-only timestamp formatting.
- `README.md`, `CONTRIBUTING.md`, and this `CHANGELOG.md`.
- Generated man pages under `man/`.
- Brandi genome (`.brandi/`) and Skillastic skill (`.skillastic/skills/baby-dev.md`).

### Changed

- `boom::execute_updates` now runs tools concurrently, bounded by
  `parallelism`, instead of one at a time (`parallelism` was previously
  accepted but ignored).
- Replaced shell-outs to `mkdir`, `cp`, and `install` with `std::fs` operations.
- Replaced `chrono` with a custom UTC timestamp formatter.
- `birthd` now uses the shared logger and writes to both stderr and its log file.

### Fixed

- `boom::detect_updates` no longer silently drops most tools when
  `parallelism` is smaller than the tool count — it used to `await` and
  discard every completed batch except the last.

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
