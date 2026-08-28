# baby

**baby — Build And Bin Yield** — validate a project's installation recipe,
build it, and install the resulting binary with one command.

> Make building and installing Rust binaries effortless, observable, and repeatable.

## Install

From source with a working Rust toolchain:

```bash
cargo install --path .
```

This installs three binaries:

- `baby` — one-shot build and install.
- `birthd` — filesystem watcher daemon that rebuilds on change.
- `birthctl` — control utility for `birthd`.

## Quick start

Build and install the current project to `/usr/local/bin`:

```bash
baby
```

Install to your user bin directory instead:

```bash
baby --user
```

Build, install, and run the binary:

```bash
baby --run -- --help
```

Set up a watched project:

```bash
birthctl watch --project myapp --path src --path Cargo.toml --install ~/.local/bin
birthd &
```

Now every change to `src/` or `Cargo.toml` triggers a rebuild and install.

## Configuration

### Installation recipes

baby resolves installation metadata in this order:

1. An explicit `--recipe <PATH>`.
2. `.baby.toml` in the current project.
3. A compatibility recipe derived from `[package].name` in `Cargo.toml`.

Directory names are never used as binary names. Non-Cargo repositories must
provide a versioned recipe, so repository discovery cannot silently turn into
the wrong build command. Recipe resolution and validation happen before any
command executes.

```toml
schema = "baby.install/v1"
build_system = "npm" # cargo, npm, python, binary-release, or script
binary = "my-cli"
artifact = "dist/my-cli"
commands = [
  ["npm", "ci"],
  ["npm", "run", "build"],
]
```

`artifact` must be repository-relative and cannot contain `..`. Each command
is an argument array, so it is executed directly without a shell. A
`binary-release` recipe may omit `commands` when the artifact is already
present. Use `baby --check-recipe` to validate and print the resolved recipe
without compiling or installing.

#### Graceful restart hooks

By default, `install_binary` never truncates a binary in place — it stages
the new one alongside the old and `rename`s it into place, so installing
over a currently-running binary never fails with `ETXTBSY`. The running
process keeps serving the old (now unlinked) inode until it's restarted.

`--service` triggers that restart, and by default it's a hard
`systemctl restart <binary>.service`. For a daemon that needs to hand off
gracefully instead of being killed mid-request (leader election, in-flight
connections, etc.), set `restart_command` in the recipe to run your own
handoff tooling instead:

```toml
restart_command = ["widget-cli", "shark", "upgrade", "{binary}"]
```

`{binary}` is replaced with the resolved install path before the command
runs. This is intentionally a hand-off point, not a built-in leader-election
implementation — `baby` doesn't know how any given daemon manages its own
runtime state. kaptaind's own `.baby.toml` uses this to call
`kaptaind-cli shark upgrade`, which spawns the new binary as a standby,
health-checks it, and only then asks the old leader to retire (see
kaptaind's "Shark Stating" docs). Omit `restart_command` to keep the
default hard restart.

### Watch configuration

`birthd` discovers `.birth.toml` files in three places:

1. The current working directory.
2. `$HOME/.config/birth.d/`
3. `/etc/birth.d/`

A `.birth.toml` looks like this:

```toml
project = "myapp"
watch = ["src", "Cargo.toml"]
build = "cargo build --release"
install = "/usr/local/bin"
restart = "myapp.service"  # optional systemd service to restart
debounce_ms = 500
strip = false
backup = false
sudo = false
user = false
```

- `project` — project/binary name.
- `watch` — paths to watch for changes.
- `build` — build command (default: `cargo build --release`).
- `install` — installation directory.
- `restart` — optional systemd service to restart after a successful build.
- `debounce_ms` — quiet period before triggering a rebuild (default: 500).
- `strip` — strip debug symbols before installing.
- `backup` — backup the existing binary before overwriting.
- `sudo` — use `sudo` for privileged operations.
- `user` — install to `~/.local/bin` (ignored if `install` is absolute).

## Commands

### `baby`

```text
baby [OPTIONS] [RUN_ARGS]
```

| Option | Description |
|--------|-------------|
| `--run` | Build, install, then execute the binary. |
| `--strip` | Strip debug symbols before installing. |
| `--backup` | Backup existing binary before overwriting. |
| `--service` | Restart matching systemd service after install. |
| `--sudo` | Use `sudo` for privileged install operations. |
| `--user` | Install to `~/.local/bin`. |
| `--dry-run` | Show what would happen without mutating the filesystem. |
| `--no-clean` | Keep Cargo build artefacts after installation. |
| `--target-dir <DIR>` | Override the Cargo target directory. |
| `--install-dir <DIR>` | Override the installation directory. |
| `--recipe <PATH>` | Use an explicit `baby.install/v1` recipe. |
| `--check-recipe` | Validate and print the recipe without executing it. |

When stderr is an interactive terminal, `baby` shows a short crying-baby
installation animation. On success the baby falls asleep to the right and a
completion summary reports install path, elapsed time, build command count,
installed artifact size, and cleanup status. Redirected output, CI, dry runs,
and `birthd` logs remain non-animated.

### `birthctl`

```text
birthctl <COMMAND>
```

| Command | Description |
|---------|-------------|
| `status` | Show daemon status and watched projects. |
| `reload` | Tell `birthd` to reload its configs. |
| `stop` | Gracefully stop the daemon. |
| `logs` | Show recent daemon logs. |
| `watch` | Create a local `.birth.toml`. |

### `birthd`

```text
birthd
```

Run the build daemon. It stores its PID in `$XDG_RUNTIME_DIR/birthd.pid`
(or `/tmp/birthd.pid`) and writes logs to `$XDG_STATE_HOME/birthd.log`
(or `~/.local/state/birthd.log`).

Signals:

- `SIGHUP` — reload configuration.
- `SIGTERM` / `SIGINT` — graceful shutdown.

## XDG paths

| Purpose | Path |
|---------|------|
| PID file | `$XDG_RUNTIME_DIR/birthd.pid` |
| Log file | `$XDG_STATE_HOME/birthd.log` |
| User configs | `$XDG_CONFIG_HOME/birth.d/` |
| Fallback user install | `$HOME/.local/bin` |

If the XDG variables are unset, `birthd` falls back to common defaults.

## Development

The quality gate for every change:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo audit
brandi lint --fail-under 80
```

This repository is monitored by [kaptaind](https://github.com/elci-group/kaptaind).
Kaptaind auto-commits qualifying changes, but GitHub branch protection on `main`
prevents auto-push, so maintainers push manually.

## License

MIT. See [LICENSE](LICENSE).
