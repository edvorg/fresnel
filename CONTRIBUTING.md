# Contributing

Read [docs/architecture.md](docs/architecture.md) first: the design rules and the crate borders.

## Checks before each push

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-sizes && cargo xtask check-borders
```

CI runs these commands on macOS and Linux, with the time tests in a release build (job `time`).

## Code

- A module has one task, and a source file 400 lines or fewer. An item is private to its crate
  unless `lib.rs` exports it: use `pub(crate)` or `pub(super)`.
- A doc comment gives the reason for the code, in short, and an `#[allow]` or an `#[expect]`
  its `reason`. No dead code and no `todo!`.
- Library code returns an error: no `unwrap`, no `expect`. In `fresnel-ui`, read a slice or
  a text with `get`, not with an index.
- Do not ignore a result. `let _ =` and `.ok();` do not compile. No lint finds `_ = result;`,
  `drop(result)` or a variable that only holds `is_err()`. They hide the error in the same way.
- A view is a function of a copy of the state (`CoreView`). It returns a `Command` and holds
  no handle to the core. Work that can block runs in the task of the core, never in a frame.
- A widget gets the color of a cluster with the identity of the cluster, not with a position.
- Do not take a lock in the head of `if let` or `match`: the guard lives to the end of the
  body. Copy the value in a statement of its own. A lint refuses the other form.

## Background tasks

Start each background task with `Tasks::start`: a name, a restart rule and a body that builds
one run. A direct `tokio::spawn` or `std::thread::spawn` does not compile.

- Keep the handle: the task stops when it drops. Use `Restart::Always` for a task that must
  stay alive, and `Restart::Never` for a job. The body runs again after an error or a panic,
  so it builds each run from a clean state.
- A task can own a `JoinSet` for its short child tasks, if it reads the result of each one.
- A task reads no file itself. A read can wait for a slow disk, and a thread of the runtime
  that waits holds other tasks. Run the read with `spawn_blocking` and wait for its result.
- A worker thread starts with `Tasks::start_thread`. Nothing can stop a thread from outside:
  its body returns when its input ends, or when its `Stop` is set. The owner drops the handle
  before that input, so that end is not a fault. `std::thread::scope` joins each thread.

## Requests

- A request to `fresnel_net::Cluster` carries a `RequestKind` as its extension. The kind sets
  the connection and the time limits. A request with no kind is refused.
- Code above `fresnel-net` sends a request through one of the six functions of `Gateway`.
  Build the request with `kube::core::Request`. No other module holds a kube client.
- An error text and a journal text have no output of a child process (that is `detail()`), no
  value of an object and no part of a kubeconfig file. For bad JSON or YAML, give the position.

## A new kind

A kind is a typed identity, `kinds::Kind`, with one description: `kinds/<kind>.rs` of
`fresnel-core` implements `Describe` for its `k8s-openapi` type (model: `kinds/ingress.rs`).

1. Add the line `Name: module::Type;` to `kinds/list.rs`. Its place is the priority of the
   first list of a cluster and the place in its sidebar group. An `ID` of an other kind fails.
2. Implement `Describe`. `TITLE` is the plural of the sidebar and `SIDEBAR` the group of plan
   section 11.3. `TIER`: `A` for a kind of each view, `C` for objects that change all the
   time, `B` for each other one: it has a store of metadata and a store of full objects on
   demand. `COLUMNS` is the union of the list columns of Headlamp and Lens (`research/01` and
   `02`) besides Name, Namespace, Labels and Age, which each kind has. A column reads its
   `Value` from the object alone, a text has the form of `kubectl get`, and `.hidden()` marks
   a column that the two tools hide. `ACTIONS` adds the actions of plan section 10 to
   `Action::EACH_KIND`; a new action gets its entry in `commands::TABLE`. `health` has the rule
   of the kind or `by_metadata`, `search` gives the texts of the quick search besides the
   metadata (images, nodes, addresses), and `trim` removes each field that no store may keep.
3. Add an object of the kind to `tests/k3s_stores/kinds/scene.yaml`, and each column that
   kubectl shows too to `SAME` in `tests/common/columns.rs`. The cluster test `kinds` with
   `FRESNEL_WRITE_FIXTURE=1` compares the columns with the server and writes `fixtures/kinds`,
   and a unit test compares them on that copy. Each branch of `health` gets a unit test on
   `fixture::of_kinds`.
4. The page reads `ClusterData::snapshot` after `ShowKinds`. Own sections: `views/<kind>.rs`.

## Stores and indexes

- A reader of a store gets a snapshot with `Reader::read` and holds it for one frame or one
  computation. It never gets a lock. Give each new published state a `published::Publisher`.
- The watch loop sends its requests through the trait `Source`. A rule of the loop gets a
  unit test with paused time and exact times on the source with a script (`watch/tests`).
- A state of a store tells its reason. No step of the loop hides an error or counts the
  loss of a connection as a failure of its own: the channel counts it.
- An object shows "change pending" after a write while `Snapshot::is_pending` is true for
  the answer of that write (`store::Written`). No code puts a mark into a store.
- An index is an `Indexer` in `index/`. It follows each store with `Tracked::read`, holds a
  relation in `Links`, and each answer gives its `Basis`: "none" never looks like "not loaded".

## Views, tables and commands

- A table is a `Table` with the `Model` of its kind (`widgets/table/model.rs`). A page calls
  `Table::restore` before `show`, and sends the command of `save_layout` after it.
- `Model::rows` and `Model::compare` run on the worker thread of the table, a
  `widgets::worker::Worker`. A sort value comes from the object alone, and `Model::tie` gives
  two objects one sequence. A view that closes empties the mailbox of its worker.
- A cursor and a selection hold an `ObjectId`, never a row number. A hidden view calls `rest`.
- A view marks each row of a store that is not live and tells the state of that store with
  its reason. A list with no row says "none" only after a complete list of each store.
- A command has one entry in `commands::TABLE` (identifier, label, a default key only when the
  app runs it, confirm rule). The keys, the row menus and the quick search read it.
- A setting is a key of `config.toml`, read in `config/read.rs`. An unknown key or a value that
  does not fit is a problem with its line and column, and the other keys stay in use.

## Tests

- A test must be able to fail. Break the code one time and see the failure.
- A test waits for a signal of the code, never for a fixed time. For a timer, use paused time
  (`#[tokio::test(start_paused = true)]`): a time limit of `fresnel-net` gets its exact time.
- A test on real sockets uses real time and waits for a counter of the test kit (`wait_for`).
  Only a check that nothing arrives waits a short fixed time.
- A fault that k3s cannot make comes from `FakeApiServer`, a network fault from a `FaultProxy`.
- A UI test uses `egui_kittest` and opens no window. It builds the `CoreView` of each state
  by hand (`tests/views/fake.rs`) and reads the frame through the accessibility tree. It
  draws frames until the frame shows what it waits for (`until`).
- A race test of a table holds the worker at a gate and changes the data, the filter or the
  sort in that time (`widgets/table/tests/`). A time limit of a view is for a release build.
  The PNG files of the views are for a look by a person: no test compares pixels.
- A test of a program starts it as a child process with `env -i`, the `PATH` of Finder and a
  home folder of the test (`fresnel_testkit::Home`). No test reads the home folder of the user.
- No test opens the window of `fresnel`: it runs `Boot`, or `frame::draw` in a harness. A
  test of `xtask` works in the test folder of the build, never in `/Applications`.
- A test file that needs `expect` starts with an `#![allow(clippy::expect_used, reason = "…")]`.
- A cluster test has `#[ignore = "needs k3s in Docker"]`, and `cargo test --workspace --
  --ignored` runs it. A store test asserts a recovery time and, after each fault,
  `assert_equal_to_the_api` (`tests/k3s_stores/api.rs`).
- `fresnel_testkit::K3s` publishes only the kube-apiserver port of `rancher/k3s:v1.33.5-k3s1`.
  Its drop removes the container: `docker ps --all --filter name=fresnel-test-` shows the rest.
- No test uses a real cluster or the kubeconfig of the user. No object of a real cluster goes
  into the repository: `fixtures/k3s` and `fixtures/kinds` are copies of the scenes of the
  tests `indexes` and `kinds`, which write them again when `FRESNEL_WRITE_FIXTURE` is set.

## Dependencies

- The root `Cargo.toml` holds the one version table, and a crate takes `workspace = true`.
  An exact pin (`=`) has a comment that gives its reason.
- `fresnel-net` does not depend on kube, and `fresnel-core` not on egui, also not through some
  other crate: `cargo xtask check-borders` and `cargo test` check it. `fresnel-ui` uses no
  type of kube, and only `widgets/table.rs` uses `egui_table`.
- A crate above `fresnel-core` names the connection layer `fresnel_core::net`, not `fresnel-net`.

## Documents and commits

- `README.md` has 80 lines or fewer, `docs/architecture.md` 200 and this file 150.
  `docs/decisions.md` has one paragraph of 80 words or fewer for each decision; a change of a
  design choice changes its paragraph.
- Commit in small steps. The message says what changed and why, in plain words. For now,
  changes go directly to `master`. Push only when the five checks pass.
