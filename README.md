# fresnel

fresnel is a native Kubernetes desktop client in Rust with egui. It shows many clusters in one
window. Its first aim is stability: no stalled connection, no hidden error, and no old data
that looks current.

## Status

Milestones M0 to M4 are complete; the work is in M6 (metrics), M7 (other kinds) and M9 (logs).
The repository has CI, the test kit, `fresnel-net`, `fresnel-core` (kubeconfig, gateway, app
core, journal, state and settings files, 41 kinds, watch loop, stores, indexes, manifest, quick
search, command table, keys of Secrets and ConfigMaps, operations as jobs, permissions, metrics),
`doctor`, `watch` and the window: a live table of each kind, the panel with the workload views,
their actions and the keys of a Secret or a ConfigMap, the New Secret dialog, the Operations tab.

## The app

```sh
cargo xtask install                   # builds Fresnel.app and puts it into /Applications
cargo xtask install --to <folder>     # the same into some other folder
```

The earlier bundle stays as `Fresnel.app.prev`. At its first start, the app selects the
current context of the kubeconfig files. The top bar has a switch for each cluster. The app
keeps the selection and the widths and columns of each table in `state.json`.

- A table shows each selected cluster. A row with an open ring is from a cluster that is
  not live: its values are old, and a note above the table gives the reason.
- Cmd+K opens the quick search of each kind of each selected cluster, Cmd+Shift+N a new Secret.
  Cmd+[ and Cmd+] go back and forward. A right click lists the actions of a row; a change asks.
- Settings folder: `~/Library/Application Support/fresnel`. A debug build uses `fresnel-debug`.
  Its `config.toml` has `version = 1` and `[shortcuts]`: `back = "Alt+Left"`, `filter = ""`
  (`commands::TABLE` has each name). The journal tells each problem with its line and column.
- Log folder: `~/Library/Logs/fresnel`, with `fresnel.log` and a report file for each panic.
- Only one process runs. `cargo run --package fresnel` starts a debug build next to the app.

## Build and test

Install [rustup](https://rustup.rs). The file `rust-toolchain.toml` selects the compiler. On
Linux, the UI tests need the package `mesa-vulkan-drivers`.

```sh
cargo build
cargo test --workspace                # unit tests and UI tests, with no cluster
FRESNEL_SNAPSHOT_DIR=<folder> cargo test --package fresnel-ui   # also PNG files of the views
cargo test --workspace -- --ignored   # cluster tests, with k3s in Docker
cargo test --release --package fresnel-ui --test views -- frame_time search::time
cargo xtask check-sizes               # size limits of documents and source files
cargo xtask check-borders             # no kube in fresnel-net, no egui in fresnel-core
```

## The commands of fresnel-cli

Each command runs the code of the app and changes nothing in a cluster. With no context, a
command lists the contexts and connects to nothing.

```sh
cargo run --package fresnel-cli -- doctor --context <name>   # measures one cluster
cargo run --package fresnel-cli -- doctor --all --idle-close 150
cargo run --package fresnel-cli -- watch --context <name>    # the stores of one cluster
```

- `doctor` sends `GET /version` only, so it reads no object. It writes `fresnel-doctor.json`:
  the times of the login shell, the credential plugin and the connects, HTTP/2 with its
  stream limit and, with `--idle-close`, when the server closes an idle connection.
- `watch` reads the objects of each kind, as the app does: of a Secret, only the metadata.
  Each 5 seconds, it prints the state and the object count of each store.

## Layout

`crates/` has `fresnel-net` (connections, the credential plugin, the task start function),
`fresnel-core` (the domain), `fresnel-ui` (the views), the programs `fresnel` and `fresnel-cli`,
and `fresnel-testkit` (the fault proxy, k3s in Docker, a fake API server). `xtask` has the
repository tasks: `cargo xtask <command>`.

## Documents

- [docs/architecture.md](docs/architecture.md): the design rules, the crates and their borders.
- [docs/decisions.md](docs/decisions.md): each decision with its reason.
- [CONTRIBUTING.md](CONTRIBUTING.md): the rules for a change.
