# Contributing

Read [docs/architecture.md](docs/architecture.md) first. It gives the design rules and the
borders between the crates.

## Checks before each push

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-sizes
```

CI runs the same commands on macOS and Linux. The file `rust-toolchain.toml` pins the compiler.

## Code

- A module has one task. A source file has 400 lines or fewer.
- An item is private to its crate unless `lib.rs` exports it. Inside a crate, use `pub(crate)`
  or `pub(super)`.
- A doc comment gives the reason for the code. Keep each comment short.
- The repository has no dead code and no `todo!`.
- Library code returns an error. It has no `unwrap` and no `expect`.
- Do not ignore a result. `let _ =` and `.ok();` do not compile. No lint finds `_ = result;`,
  `drop(result)` or a variable that only holds `is_err()`. They hide the error in the same way.
- In `fresnel-ui`, read a slice or a text with `get`, not with an index.
- An `#[allow]` or an `#[expect]` gives its `reason`.

## Background tasks

Start each background task with `Tasks::start`. A direct `tokio::spawn` does not compile, and
a direct `std::thread::spawn` does not compile.

```rust
use fresnel_net::task::{Backoff, Restart, TaskName};

let name = TaskName::scoped("watch", cluster_name);
let handle = tasks.start(name, Restart::Always(Backoff::default()), move || {
    let api = api.clone();
    async move { watch(api).await }
});
```

- Keep the handle. The task stops when the handle drops.
- Use `Restart::Always` for a task that must stay alive, and `Restart::Never` for a job.
- The body runs again after an error or a panic. It must build each run from a clean state.
- A task can own a `JoinSet` for its short child tasks, if it reads the result of each one.
- A worker thread needs its own start function in `fresnel_net::task`, with a state and a
  report. `std::thread::scope` is permitted, because it joins each thread.

## Requests

- A request to `fresnel_net::Cluster` carries a `RequestKind` as its extension. The kind sets
  the connection and the time limits. A request with no kind is refused.
- An error text that can go to a log has no text of a child process. Put that text in
  `detail()`, as `PluginError` does.
- Code above `fresnel-net` sends a request through one of the six functions of
  `fresnel_core::cluster::Gateway`. Build the request with `kube::core::Request`. No other
  module holds a kube client.
- An error text has no value of an object and no part of a kubeconfig file. For JSON or YAML
  that does not fit, give the position only.

## Tests

- A test must be able to fail. Break the code one time and see the failure.
- A test waits for a signal of the code, never for a fixed time. For a timer, use paused time:
  `#[tokio::test(start_paused = true)]`.
- A test on real sockets uses real time, because paused time does not wait for a socket. It
  waits for a counter of the test kit with `wait_for`. Only a check that nothing arrives waits
  a short fixed time.
- A fault that k3s cannot make on order comes from `FakeApiServer`. A network fault comes
  from a `FaultProxy` before the server.
- A time limit of `fresnel-net` gets a unit test with paused time on the server in memory
  (`src/testing.rs`), and the test asserts the exact time.
- A UI test uses `egui_kittest` and opens no window.
- A test of `fresnel-cli` starts the program as a child process with `env -i`, the `PATH` of
  Finder and a home folder of the test. No test reads the home folder of the user.
- A test that needs k3s or Docker has the attribute `#[ignore = "needs k3s in Docker"]`.
  `cargo test --workspace -- --ignored` runs these tests. CI runs them on Linux.
- A helper in `tests/` that needs `expect` starts its file with
  `#![allow(clippy::expect_used, reason = "a test stops with a panic")]`.

Rules for a cluster test:

- The test starts its cluster with `fresnel_testkit::K3s::start`. The helper uses the image
  `rancher/k3s:v1.33.5-k3s1` and publishes only the kube-apiserver port, not the supervisor port.
- The drop of the helper removes the container. After a stopped test run,
  `docker ps --all --filter name=fresnel-test-` shows what is left.
- No test uses a real cluster or the kubeconfig of the user.
- Test data comes from the test cluster. No object from a real cluster goes into the repository.

## Dependencies

- The root `Cargo.toml` holds the one version table. A crate takes a dependency with
  `workspace = true`.
- An exact pin (`=`) has a comment that gives its reason.
- `fresnel-net` does not depend on kube. `fresnel-core` does not depend on egui.
  `fresnel-ui` does not use the types of kube.
- A crate above `fresnel-core` names the connection layer as `fresnel_core::net`. Its manifest
  does not list `fresnel-net`.

## Documents

- `README.md` has 80 lines or fewer, `docs/architecture.md` 200 and this file 150.
- `docs/decisions.md` has one paragraph of 80 words or fewer for each decision.
- A change of a design choice also changes its paragraph in `docs/decisions.md`.

## Commits

- Commit in small steps. The message says what changed and why, in plain words.
- For now, changes go directly to `master`. Push only when the four checks pass.
