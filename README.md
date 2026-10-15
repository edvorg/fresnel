# fresnel

fresnel is a native Kubernetes desktop client in Rust with egui. It shows many clusters in one
window. Its first aim is stability: no stalled connection, no hidden error, and no old data
that looks current.

## Screenshots

<table>
<tr><td width="50%"><a href="docs/screenshots/pods-three-clusters.png"><img src="docs/screenshots/pods-three-clusters.png" alt="The Pods page"></a><br>The pods of three clusters. prod-us has no connection: its rows are old, and a note tells why.</td>
<td width="50%"><a href="docs/screenshots/deployment-metrics.png"><img src="docs/screenshots/deployment-metrics.png" alt="The Metrics tab of a Deployment"></a><br>The CPU and memory of a Deployment and of each of its pods, with the request and the limit.</td></tr>
<tr><td width="50%"><a href="docs/screenshots/deployment-logs.png"><img src="docs/screenshots/deployment-logs.png" alt="The logs of a Deployment"></a><br>The logs of the pods of a Deployment, each pod in its color, with a search.</td>
<td width="50%"><a href="docs/screenshots/quick-search.png"><img src="docs/screenshots/quick-search.png" alt="The quick search"></a><br>Cmd+K: the quick search finds objects of each kind in each cluster.</td></tr>
<tr><td width="50%"><a href="docs/screenshots/rollback.png"><img src="docs/screenshots/rollback.png" alt="The rollback dialog"></a><br>The revisions of a Deployment and the dialog of a rollback.</td>
<td width="50%"><a href="docs/screenshots/secret-save.png"><img src="docs/screenshots/secret-save.png" alt="The save dialog of a Secret"></a><br>The keys of a Secret: a save sends only the changed key, after its diff.</td></tr>
<tr><td width="50%"><a href="docs/screenshots/node-drain.png"><img src="docs/screenshots/node-drain.png" alt="A node drain in the Operations tab"></a><br>A node drain as a job of the Operations tab, with the state of each pod.</td>
<td width="50%"><a href="docs/screenshots/cluster-overview-light.png"><img src="docs/screenshots/cluster-overview-light.png" alt="The cluster overview"></a><br>The cluster overview in the light theme: nodes, pods, usage and warnings.</td></tr>
</table>

## Status

Milestones M0 to M4, M6, M7 and M9 are complete; the work is in M8 (settings, keys, notices).
The repository has CI, the test kit, `fresnel-net`, `fresnel-core` (kubeconfig, gateway, app
core, journal, state and settings files, 41 kinds, watch loop, stores, indexes, manifest, quick
search, command table, keys, operations as jobs, permissions, metrics, logs, notices), `doctor`,
`watch` and the window: a live table and a panel with the sections of each kind in scope, actions,
a node drain, Secret and ConfigMap keys, the Operations tab, metrics, logs, overviews, settings.

## The app

```sh
cargo xtask install                   # builds Fresnel.app and puts it into /Applications
cargo xtask install --to <folder>     # the same into some other folder
```

The earlier bundle stays as `Fresnel.app.prev`. At its first start, the app selects the current
context of the kubeconfig files. It keeps the selection and the table layouts in `state.json`.

- A table shows each selected cluster. A row with an open ring is from a cluster that is
  not live: its values are old, and a note above the table gives the reason.
- Cmd+K opens the quick search, ? lists each key, Cmd+1 to Cmd+4 and F6 move the focus between
  the sidebar, the table, the panel and the dock. Cmd+[ and Cmd+] go back and forward. A right
  click lists the actions of a row; a change asks, and a read-only cluster refuses it.
- Settings folder: `~/Library/Application Support/fresnel`. A debug build uses `fresnel-debug`.
  Cmd+, opens the settings view, which edits `config.toml` and keeps its comments. The app reads
  it again after each change; the journal tells each problem with its line and column.
- A failed operation, and a cluster that stays lost while the window is hidden, give a notification.
- Log folder: `~/Library/Logs/fresnel` (`fresnel.log`, panic reports). Saved logs: `~/Downloads`.
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
cargo xtask screenshots               # the pictures above, from made-up data with no cluster
```

## The commands of fresnel-cli

Each command runs the code of the app and changes nothing in a cluster. With no context, a
command lists the contexts and connects to nothing.

```sh
cargo run --package fresnel-cli -- doctor --context <name>   # one cluster; --all: each one
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
