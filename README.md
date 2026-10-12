# fresnel

fresnel is a native Kubernetes desktop client in Rust with egui. It shows many clusters in one
window. Its first aim is stability: no stalled connection, no hidden error, and no old data
that looks current.

## Status

The work is in milestone M0 (connect). The repository has the workspace, the lints, CI and the
function that starts each background task. The network layer and the first window come next.

## Build and test

Install [rustup](https://rustup.rs). The file `rust-toolchain.toml` selects the compiler.

```sh
cargo build
cargo test --workspace                # unit tests and UI tests, with no cluster
cargo test --workspace -- --ignored   # cluster tests, with k3s in Docker
cargo xtask check-sizes               # size limits of documents and source files
```

On Linux, the UI tests need the package `mesa-vulkan-drivers`.

## Layout

- `crates/fresnel-net`: connections, and the function that starts each background task.
- `crates/fresnel-core`: the domain. Clusters, stores, search and operations.
- `crates/fresnel-ui`: the egui widgets and views.
- `crates/fresnel`: the desktop program.
- `crates/fresnel-cli`: the command line program for tests and fault diagnosis.
- `crates/fresnel-testkit`: the fault proxy, the helper for k3s in Docker and a fake API server.
- `xtask`: repository tasks. Run one with `cargo xtask <command>`.

## Documents

- [docs/architecture.md](docs/architecture.md): the design rules, the crates and their borders.
- [docs/decisions.md](docs/decisions.md): each decision with its reason.
- [CONTRIBUTING.md](CONTRIBUTING.md): the rules for a change.
