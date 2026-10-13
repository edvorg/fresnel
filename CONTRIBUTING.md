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
- An item is private to its crate unless `lib.rs` exports it: use `pub(crate)` or `pub(super)`.
- A doc comment gives the reason for the code. Keep each comment short.
- The repository has no dead code and no `todo!`.
- Library code returns an error. It has no `unwrap` and no `expect`.
- Do not ignore a result. `let _ =` and `.ok();` do not compile. No lint finds `_ = result;`,
  `drop(result)` or a variable that only holds `is_err()`. They hide the error in the same way.
- In `fresnel-ui`, read a slice or a text with `get`, not with an index.
- A view is a function of a copy of the state (`CoreView`). It returns a `Command` and holds
  no handle to the core. Work that can block runs in the task of the core, never in a frame.
- A widget gets the color of a cluster with the identity of the cluster, not with a position.
- A journal text can go to the log file. It has no output of a child process.
- An `#[allow]` or an `#[expect]` gives its `reason`.
- Do not take a lock in the head of `if let` or `match`: the guard lives to the end of the
  body. Copy the value in a statement of its own. A lint refuses the other form.

## Background tasks

Start each background task with `Tasks::start`. A direct `tokio::spawn` does not compile, and
a direct `std::thread::spawn` does not compile.

```rust
let name = TaskName::scoped("watch", cluster_name);
let handle = tasks.start(name, Restart::Always(Backoff::default()), move || {
    let api = api.clone();
    async move { watch(api).await }
});
```

- Keep the handle: the task stops when it drops. Use `Restart::Always` for a task that must
  stay alive, and `Restart::Never` for a job.
- The body runs again after an error or a panic. It must build each run from a clean state.
- A task can own a `JoinSet` for its short child tasks, if it reads the result of each one.
- A task reads no file itself. A read can wait for a slow disk, and a thread of the runtime
  that waits holds other tasks. Run the read with `spawn_blocking` and wait for its result.
- A worker thread starts with `Tasks::start_thread`. Nothing can stop a thread from outside:
  its body returns when its input ends, or when its `Stop` is set. The owner drops the handle
  before that input, so that end is not a fault. `std::thread::scope` joins each thread.

## Requests

- A request to `fresnel_net::Cluster` carries a `RequestKind` as its extension. The kind sets
  the connection and the time limits. A request with no kind is refused.
- An error text that can go to a log has no text of a child process: that text is `detail()`.
- Code above `fresnel-net` sends a request through one of the six functions of `Gateway`.
  Build the request with `kube::core::Request`. No other module holds a kube client.
- An error text has no value of an object and no part of a kubeconfig file. For JSON or YAML
  that does not fit, give the position only.

## Kinds and stores

- A kind is one line of the list in `kinds/list.rs`: its identity, its tier and its object
  type. The sequence of the list is the priority of the first lists of a cluster.
- A reader of a store gets a snapshot with `Reader::read` and holds it for one frame or one
  computation. It never gets a lock. Give each new published state a `published::Publisher`.
- The watch loop sends its requests through the trait `Source`. A rule of the loop gets a
  unit test with paused time and exact times on the source with a script (`watch/tests`).
- A state of a store tells its reason. No step of the loop hides an error or counts the
  loss of a connection as a failure of its own: the channel counts it.
- An object shows "change pending" after a write while `Snapshot::is_pending` is true for
  the answer of that write (`store::Written`). No code puts a mark into a store.
- A tier A kind gives its health value in `kinds/<kind>.rs`, from the object alone: a value
  has no clock and reads no other object.
- An index is an `Indexer` in `index/`. It follows each store with `Tracked::read`, holds a
  relation in `Links`, and each answer gives its `Basis`: "none" never looks like "not loaded".

## Views and tables

- A table is a `Table` with the `Model` of its kind (`widgets/table/model.rs`).
- `Model::rows` and `Model::compare` run on the worker thread of the table. A sort value comes
  from the object alone, and `Model::tie` gives two objects one sequence.
- A cursor and a selection hold an `ObjectId`, never a row number. A hidden view calls `rest`.
- A view marks each row of a store that is not live and tells the state of that store with
  its reason. A list with no row says "none" only after a complete list of each store.

## Tests

- A test must be able to fail. Break the code one time and see the failure.
- A test waits for a signal of the code, never for a fixed time. For a timer, use paused time:
  `#[tokio::test(start_paused = true)]`.
- A test on real sockets uses real time. It waits for a counter of the test kit with
  `wait_for`. Only a check that nothing arrives waits a short fixed time.
- A fault that k3s cannot make on order comes from `FakeApiServer`. A network fault comes
  from a `FaultProxy` before the server.
- A time limit of `fresnel-net` gets a unit test with paused time that asserts the exact time.
- A UI test uses `egui_kittest` and opens no window. It builds the `CoreView` of each state
  by hand (`tests/views/fake.rs`) and reads the frame through the accessibility tree. It
  draws frames until the frame shows what it waits for (`until`).
- A race test of a table holds the worker at a gate and changes the data, the filter or the
  sort in that time (`widgets/table/tests/`). The limit of a frame time is for a release build.
- The PNG files of the views are for a look by a person. No test compares pixels.
- A test of a program starts it as a child process with `env -i`, the `PATH` of Finder and a
  home folder of the test (`fresnel_testkit::Home`). No test reads the home folder of the user.
- No test opens the window of `fresnel`. A test runs `Boot`, or `frame::draw` in a harness.
- A test of `xtask` works in the test folder of the build, never in `/Applications`.
- A test that needs k3s or Docker has the attribute `#[ignore = "needs k3s in Docker"]`.
  `cargo test --workspace -- --ignored` runs these tests. CI runs them on Linux.
- A cluster test of the stores asserts a recovery time and, after each fault, that each
  store is equal to the API (`assert_equal_to_the_api` in `tests/k3s_stores/api.rs`).
- A test file that needs `expect` starts with an `#![allow(clippy::expect_used, reason = "…")]`.

Rules for a cluster test:

- The test starts its cluster with `fresnel_testkit::K3s::start`, which publishes only the
  kube-apiserver port of the image `rancher/k3s:v1.33.5-k3s1`.
- The drop of the helper removes the container. After a stopped test run,
  `docker ps --all --filter name=fresnel-test-` shows what is left.
- No test uses a real cluster or the kubeconfig of the user.
- Test data comes from the test cluster. No object from a real cluster goes into the repository.
- `fixtures/k3s` is a copy of the scene `tests/k3s_stores/indexes/scene.yaml`. The cluster test
  `the_indexes` writes it again when `FRESNEL_WRITE_FIXTURE` is set. The unit tests and the view
  tests use it; a test changes an object for a state that the scene cannot hold.

## Dependencies

- The root `Cargo.toml` holds the one version table: a crate takes `workspace = true`.
- An exact pin (`=`) has a comment that gives its reason.
- `fresnel-net` does not depend on kube, and `fresnel-core` not on egui, also not through some
  other crate: `cargo xtask check-borders` and `cargo test` check it. `fresnel-ui` uses no
  type of kube, and only `widgets/table.rs` uses `egui_table`.
- A crate above `fresnel-core` names the connection layer `fresnel_core::net`, not `fresnel-net`.

## Documents

- `README.md` has 80 lines or fewer, `docs/architecture.md` 200 and this file 150.
- `docs/decisions.md` has one paragraph of 80 words or fewer for each decision.
- A change of a design choice also changes its paragraph in `docs/decisions.md`.

## Commits

- Commit in small steps. The message says what changed and why, in plain words.
- For now, changes go directly to `master`. Push only when the four checks pass.
