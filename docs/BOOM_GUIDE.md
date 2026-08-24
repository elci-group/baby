# The `boom` Sub-Command: Parallel Tool Management

**Version:** 0.4.43+  
**Status:** Production Ready  
**Last Updated:** August 23, 2026

## Executive Summary

The `boom` sub-command provides a unified mechanism for discovering, tracking, and updating multiple tool installations across your system in parallel. It leverages git-based version detection and semantic versioning to maintain a curated set of development tools at their latest compatible versions.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Quick Start](#quick-start)
3. [Configuration Reference](#configuration-reference)
4. [Usage Modes](#usage-modes)
5. [Implementation Details](#implementation-details)
6. [Performance Characteristics](#performance-characteristics)

## Architecture Overview

### Design Principles

**Declarative Configuration**: Tools are declared in `.boom.toml` files, making tool sets reproducible and shareable across teams.

**Parallel Discovery & Detection**: The system discovers installed tools and queries remote repositories concurrently, reducing wall-clock time for large tool sets.

**Semantic Versioning**: Version comparison uses semver semantics, supporting stable, nightly, and bleeding-edge release channels.

**Safe by Default**: All operations require explicit confirmation unless `--yes` flag is used. Dry-run mode shows proposed changes without execution.

### System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    baby boom CLI                             │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────────┐      ┌──────────────┐                     │
│  │   Discovery  │      │     Config   │                     │
│  │  (Filesystem)│      │   Parsing    │                     │
│  └──────────────┘      └──────────────┘                     │
│         │                      │                             │
│         └──────────────────────┘                             │
│                │                                             │
│         ┌──────▼──────────┐                                 │
│         │  Tool Registry  │                                 │
│         │ (in-memory)     │                                 │
│         └──────┬──────────┘                                 │
│                │                                             │
│         ┌──────▼─────────────────┐                          │
│         │   Detection (Parallel)  │                          │
│         │  - Git ls-remote       │                          │
│         │  - Version Extraction  │                          │
│         │  - Version Comparison  │                          │
│         └──────┬─────────────────┘                          │
│                │                                             │
│         ┌──────▼──────────┐                                 │
│         │  Update Report  │                                 │
│         │  (Presentation) │                                 │
│         └──────┬──────────┘                                 │
│                │                                             │
│    ┌───────────┼───────────┐                                │
│    │           │           │                                │
│ ┌──▼──┐  ┌────▼───┐  ┌────▼──────┐                         │
│ │Dry- │  │Confirm │  │Interactive│                         │
│ │Run  │  │Proceed │  │Selection  │                         │
│ └──┬──┘  └────┬───┘  └────┬──────┘                         │
│    │         │            │                                 │
│    └─────────┼────────────┘                                 │
│              │                                               │
│         ┌────▼─────────────────┐                            │
│         │ Execution (Parallel) │                            │
│         │  - Clone repos       │                            │
│         │  - Build (baby)      │                            │
│         │  - Install           │                            │
│         └────┬─────────────────┘                            │
│              │                                               │
│         ┌────▼──────────┐                                   │
│         │ Report Results│                                   │
│         │ Success/Fail  │                                   │
│         └───────────────┘                                   │
└─────────────────────────────────────────────────────────────┘
```

### Core Modules

| Module | Lines | Responsibility |
|--------|-------|-----------------|
| `types.rs` | 157 | Data structures, enums, builders |
| `config.rs` | 77 | TOML parsing, channel resolution |
| `discovery.rs` | 139 | Filesystem scanning, deduplication |
| `detection.rs` | 163 | Git queries, version comparison |
| `execution.rs` | 170 | Dry-run, confirmation, parallel builds |
| `interactive.rs` | 70 | Terminal UI for tool selection |
| **Total** | **776** | Complete feature set |

## Quick Start

### Initialize Configuration

Create a new `.boom.toml` in your project root:

```bash
baby boom --init
```

This generates:
```toml
[boom]
channel = "stable"
scan_dirs = [".", "tools"]

# Discovered tools listed below (uncomment to enable)
# [[tools]]
# name = "example"
# repo = "https://github.com/org/example.git"
```

### Check for Updates

Preview what would be updated:

```bash
baby boom --dry-run
```

Output:
```
Tool            Installed   Available   Action
──────────────────────────────────────────────
kaptaind        0.4.32      0.4.43      Update
baby            0.4.42      0.4.43      Update
birthd          0.4.40      0.4.43      Update
```

### Update All Tools

Auto-confirm and update:

```bash
baby boom --yes
```

### Interactive Mode

Select which tools to update:

```bash
baby boom --interactive
```

## Configuration Reference

### Top-Level `[boom]` Section

```toml
[boom]
# Release channel: stable | nightly | bleeding
# Default: stable
channel = "stable"

# Directories to scan for .baby.toml files (relative paths)
# Default: ["."]
scan_dirs = [".", "tools", "vendor"]

# Number of parallel workers for detection
# Default: 4 (auto-detected from CPU count)
parallelism = 8

# Strip debug symbols from binaries (requires --strip flag)
strip = false

# Backup existing binaries before overwriting
backup = true

# Use sudo for privileged installs
sudo = false

# Install to ~/.local/bin instead of /usr/local/bin
user = false
```

### Tool Declaration `[[tools]]` Section

```toml
[[tools]]
# Required: Tool name (matches binary name)
name = "myapp"

# Required: Git repository URL
repo = "https://github.com/org/myapp.git"

# Optional: Subdirectory containing .baby.toml (if not root)
dir = "cmd/myapp"

# Optional: Path to custom .baby.toml recipe
recipe = ".myapp.toml"

# Optional: Override channel for this tool
# Values: stable | nightly | bleeding
channel = "nightly"
```

### Channel Semantics

**Stable** (default): Releases without prerelease tags
- Versions like: `1.0.0`, `1.2.3`, `2.0.0`

**Nightly**: Prerelease versions
- Versions like: `1.0.0-rc1`, `1.0.0-beta`, `1.0.0-alpha`

**Bleeding**: All versions, newest first
- Includes both stable and prerelease

## Usage Modes

### Dry-Run Mode

```bash
baby boom --dry-run
```

Shows what would happen without making changes. Safe to run in CI/pipelines.

### Confirmation Mode (Default)

```bash
baby boom
```

Displays updates and prompts for confirmation:
```
Found 3 tool(s) with updates. Proceed? [y/N]
```

### Auto-Confirm Mode

```bash
baby boom --yes
```

Automatically confirms all updates. Use in automation/CI only.

### Interactive Mode

```bash
baby boom --interactive
```

Per-tool selection:
```
[1] kaptaind (0.4.32 → 0.4.43)? [y/n/skip]
[2] baby (0.4.42 → 0.4.43)? [y/n/skip]
[3] birthd (0.4.40 → 0.4.43)? [y/n/skip]
```

### Filtered Updates

```bash
baby boom --filter kaptaind,birthd
```

Update only specified tools.

### Custom Parallelism

```bash
baby boom --parallelism 16
```

Use 16 parallel workers instead of default 4.

## Implementation Details

### Discovery Phase

1. Load `.boom.toml` from project root
2. Parse tool declarations
3. Scan configured directories for `.baby.toml` files
4. Extract repository URLs from `.git/config` of discovered projects
5. Deduplicate by (name, repo) pair
6. Tool declarations take precedence over filesystem discovery

**Time Complexity**: O(n) filesystem traversal + O(1) config parsing  
**Space Complexity**: O(n) where n = number of discovered tools

### Detection Phase

For each tool in parallel (up to `parallelism` concurrent):

1. Query remote with `git ls-remote --tags <repo>`
2. Filter tags by channel (stable/nightly/bleeding)
3. Parse semantic versions
4. Select latest version
5. Extract installed version by running `<tool> --version`
6. Compare using semver logic

**Time Complexity**: O(n) tools × O(git_ls_remote_time)  
**Parallelized**: Tokio task per tool, bottleneck is slowest git query

### Execution Phase

For each selected tool in parallel (up to `parallelism` concurrent):

1. Create temporary directory
2. `git clone --depth 1 <repo> <tempdir>`
3. Run `baby --recipe <tempdir>/.baby.toml --user`
4. Collect results (success/failure/duration)
5. Report per-tool status

**Time Complexity**: O(n) tools × O(clone_time + build_time)  
**Parallelized**: Tokio task per tool

### Version Comparison

Uses semantic versioning (MAJOR.MINOR.PATCH[-PRERELEASE]):

```
1.0.0 < 1.0.1 < 1.1.0 < 2.0.0              # Stable ordering
1.0.0-alpha < 1.0.0-beta < 1.0.0-rc1 < 1.0.0   # Prerelease ordering
```

## Performance Characteristics

See [BOOM_BENCHMARKS.md](BOOM_BENCHMARKS.md) for detailed performance analysis.

### Summary

| Operation | Time (10 tools) | Time (50 tools) | Parallelism Factor |
|-----------|-----------------|-----------------|-------------------|
| Discovery | ~50ms | ~200ms | O(n) |
| Detection | ~2.5s | ~8.5s | ~4x speedup |
| Execution | ~60s | ~180s | ~3x speedup |

See benchmarks document for detailed methodology and results.

## Troubleshooting

### Tool Not Discovered

1. Verify `.baby.toml` exists in project root
2. Check `scan_dirs` includes the project directory
3. Run `baby boom --dry-run` to see discovered tools

### Git Remote Query Fails

1. Verify repository URL is accessible
2. Check network connectivity
3. Confirm git is installed and in PATH

### Version Detection Fails

1. Run `<tool> --version` manually to verify format
2. Check if version string contains semantic version
3. Some tools may need custom parsing (patches welcome)

---

**Learn more:** See [BOOM_ARCHITECTURE.md](BOOM_ARCHITECTURE.md) for deep technical details.
