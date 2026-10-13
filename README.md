# fresnel

fresnel is a native Kubernetes desktop client in Rust with egui. It shows many clusters in one
window. Its first aim is stability: no stalled connection, no hidden error, and no old data
that looks current.

## Status

Milestone M0 (connect) is complete, and the work is in milestone M1 (live data). The
repository has the workspace, the lints, CI, the function that starts each background task,
the test kit, the network layer `fresnel-net`, the first parts of `fresnel-core` (kubeconfig
loader, cluster registry, gateway, app core, journal, state file, the kinds, the watch loop
and the stores), the commands `doctor` and `watch`, and the first window: the clusters of
the kubeconfig files with their live connection states. The window shows no objects yet.

## The app

```sh
cargo xtask install                   # builds Fresnel.app and puts it into /Applications
cargo xtask install --to <folder>     # the same into some other folder
```

The earlier bundle stays as `Fresnel.app.prev`. At its first start, the app selects the
current context of the kubeconfig files. The top bar has a switch for each cluster. The app
saves the selection in `state.json` of its settings folder and starts with it again.

- Settings folder: `~/Library/Application Support/fresnel`. A debug build uses `fresnel-debug`.
- Log folder: `~/Library/Logs/fresnel`, with `fresnel.log` and a report file for each panic.
- Only one process of the app runs. `cargo run --package fresnel` starts a debug build next
  to the installed app.

## Build and test

Install [rustup](https://rustup.rs). The file `rust-toolchain.toml` selects the compiler.

```sh
cargo build
cargo test --workspace                # unit tests and UI tests, with no cluster
FRESNEL_SNAPSHOT_DIR=<folder> cargo test --package fresnel-ui   # also PNG files of the views
cargo test --workspace -- --ignored   # cluster tests, with k3s in Docker
cargo xtask check-sizes               # size limits of documents and source files
cargo xtask check-borders             # no kube in fresnel-net, no egui in fresnel-core
```

On Linux, the UI tests need the package `mesa-vulkan-drivers`.

## The commands of fresnel-cli

Each command runs the code of the app and changes nothing in a cluster. With no context, a
command lists the contexts and connects to nothing.

```sh
cargo run --package fresnel-cli -- doctor --context <name>   # measures one cluster
cargo run --package fresnel-cli -- doctor --all --idle-close 150
cargo run --package fresnel-cli -- watch --context <name>    # the stores of one cluster
```

- `doctor` sends `GET /version` only, so it reads no object. It reports the time of the login
  shell and of the credential plugin, the connect times with and with no hedge, HTTP/2 with
  its stream limit and, with `--idle-close`, when the server closes an idle connection. It
  writes `fresnel-doctor.json`.
- `watch` reads the objects of each kind, as the app does: of a Secret, only the metadata.
  Each 5 seconds, it prints the state and the object count of each store.

## Layout

- `crates/fresnel-net`: connections, the credential plugin, and the function that starts each
  background task.
- `crates/fresnel-core`: the domain. Clusters, the app core, the journal, stores and operations.
- `crates/fresnel-ui`: the egui widgets and views.
- `crates/fresnel`: the desktop program.
- `crates/fresnel-cli`: the command line program for tests and fault diagnosis.
- `crates/fresnel-testkit`: the fault proxy, the helper for k3s in Docker and a fake API server.
- `xtask`: repository tasks. Run one with `cargo xtask <command>`.

## Documents

- [docs/architecture.md](docs/architecture.md): the design rules, the crates and their borders.
- [docs/decisions.md](docs/decisions.md): each decision with its reason.
- [CONTRIBUTING.md](CONTRIBUTING.md): the rules for a change.
