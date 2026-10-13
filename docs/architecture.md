# Architecture

This page gives the structure of fresnel. [decisions.md](decisions.md) gives the reasons.

## Terms

- **Cluster**: one kubeconfig context.
- **Channel**: one long-lived HTTP/2 connection to a cluster.
- **Watch**: one stream of changes for one kind in one cluster.
- **Store**: the copy in memory of all objects of one kind in one cluster.
- **Task**: one background activity with a visible state.
- **Journal**: the list of events, warnings and errors that the app shows.

## Design rules

These rules have priority over features.

1. Each task has an explicit state, and the UI shows it. A task that ends or panics gets a
   final state, a journal entry and a restart.
2. Each request has a time limit. A data stream of the user has no limit on its progress.
3. The app opens few TCP connections. Each new connection uses the hedged connect.
4. Each view shows how fresh its data is.
5. The app keeps old data visible during an error. It never shows an empty list in place of
   an error.
6. The app does not repeat a write when its result is not known. It reads the object and
   shows the result.
7. The app starts no `kubectl` process and no helper process. The only exception is the
   credential plugin of the kubeconfig.
8. Each object carries the identity of its cluster. No "current cluster" global exists.
9. The compiler and the lints refuse an ignored error. One function starts all background
   tasks and watches them.
10. Tests with injected faults prove the recovery times. CI runs them.
11. One clear pattern exists for each repeated job. Documents stay short.
12. Each buffer has a size limit. A buffer that drops data shows a counter.

## Crates

An arrow shows a permitted dependency. Each crate can also use `fresnel-testkit` in its tests.
No other dependency between the crates is permitted.

```text
fresnel (program) ------> fresnel-ui ------> fresnel-core ------> fresnel-net
fresnel-cli (program) ---------------------> fresnel-core
fresnel-testkit ------------------------------------------------> fresnel-net
xtask
```

- `fresnel-net`: connections. The hedged connector, the resolver, channels, upgrade
  connections, the credential plugin, time limits and connection states. It has no dependency
  on kube. It is the lowest crate, thus it also holds the task start function.
- `fresnel-core`: the domain. Clusters, the watch loop, stores, indexes, search, operations,
  forwards, metrics, settings and the journal. It has no dependency on egui. The crates above
  it reach the connection layer through the name `fresnel_core::net`.
- `fresnel-ui`: the egui widgets and views. It uses the object types of `k8s-openapi` and
  never the types of kube.
- `fresnel`: the desktop program. The window, the tokio runtime and the log setup.
- `fresnel-cli`: the command line program for tests and fault diagnosis. Its command `doctor`
  measures the connection to a cluster. It sends `GET /version` only, so it reads no object.
- `fresnel-testkit`: the fault proxy, the helper for k3s in Docker and a fake API server.
  Only tests use it. The test kit can use `fresnel-net`. Thus a test of `fresnel-net` that
  needs the test kit is a file in `tests/`, not a unit test.
- `xtask`: repository tasks. It uses no other crate of the workspace.

## Borders in the code

- Only one module of `fresnel-core` holds kube clients: `cluster/gateway.rs`. Each function of
  this module applies its time limit and the rule for writes. The kubeconfig loader has its
  own types for the file format.
- One module wraps each outside UI crate: `egui_table` in `widgets/table.rs`, `egui_tiles` in
  `shell/dock.rs`, `egui_plot` in `widgets/chart.rs`. An egui upgrade then changes few files.
- Each library crate has one error type with a class: auth, forbidden, conflict, timeout,
  network, server, invalid.
- An item is private to its crate by default. `lib.rs` lists the public types.

## Background tasks

`fresnel_net::task::Tasks::start` is the one function that starts a background task.

- The caller gives a name, a restart rule and a body. The body builds one run of the task.
- Each task has a state: running, backoff, or ended. `Tasks::snapshot` gives a copy of all
  states to the UI.
- A run that ends gives one report: the end (`Ok`, an error or a panic) and the pause before
  the next run. The journal takes each report through the trait `Reporter`.
- `Restart::Always` runs the task again after each end, with a pause that grows to a limit.
  `Restart::Never` is for a job that runs one time.
- The caller owns a `TaskHandle`. The task stops when the handle drops.
- Code of the caller cannot stop the supervisor: it catches a panic of the body and a panic
  of the reporter.
- A fault of the supervisor itself, for example a runtime with no timer, ends the task with a
  final state and a report.

## Network layer

`fresnel_net::Cluster` is one value for each cluster. It owns the connections, runs the
credential plugin and gives a tower service. A request carries a `RequestKind`; the kind
selects the connection, the time limits and gzip.

```text
request -> time limit -> throttle (HTTP 429) -> token and HTTP 401 rule -> lane of the kind
lane: watches | requests | logs  (HTTP/2 channels)      upgrade (HTTP/1.1, warm set)
new connection: resolver -> hedged connect (TCP and TLS) -> one protocol by ALPN
```

- **Channel**: one HTTP/2 connection with its own task. The task connects at the start and
  after each loss, sends a ping each 10 s, and sends a light request each 30 s when no stream
  is open. The state is live only after a request got an answer.
- **Group**: the channels of one purpose. At 70 percent of the stream limit of the server,
  one more connection opens, to a maximum of 3. Then a request waits, and the state shows it.
- **Hedged connect**: one more attempt starts after 1.5 connect times, and the first attempt
  that completes TCP and TLS wins.
- **Credential**: one task runs the plugin, one run at a time. A request waits 2 s at most.
  After a second HTTP 401, one small request proves the token.
- **Upgrade client**: exec, attach and port forward. A warm connection gets a test before use.
- **Signal**: a wake, a network change and "retry now" raise the one signal of a cluster. It
  ends each retry delay, and each channel tests its connection with a limit of 3 s.
- **State**: `Cluster::status` gives a copy of each state. `Cluster::changes` tells when to
  read again. The text of the plugin is only in `detail()`, never in `Display` or `Debug`.

## Clusters

`fresnel-core` makes a connected cluster from a kubeconfig context.

- **Kubeconfig**: `kubeconfig::load` reads each file of `KUBECONFIG` of the login shell and then
  `~/.kube/config`, and lists each context. For a name in two files, the first file wins. Each
  problem of a file is in the result. A context that the app cannot use stays in the list
  with its error.
- **Registry**: one entry for each context. It owns the connections of each connected cluster
  and gives a copy of each state. A `ClusterId` is the name of the context.
- **Gateway**: the requests to one cluster, with one function for each request kind: get, list
  page, watch stream, log stream, send and upgrade. The kind gives the connection and the
  time limits. A failed write tells its fate: not sent, refused or not known.
- **Probe**: `fresnel_net::probe` measures single steps of a connect with the connect code of
  the app. The `doctor` command uses it.

## Contract between the UI and the core

1. **State.** One type publishes each state. The UI gets a copy, never a lock that can stop a
   task.
2. **Commands.** The UI starts an operation as a job. Job states: pending, done, failed, not
   sent, result not known.
3. **Workers.** The UI computes sorted and filtered rows on a worker thread. Each result
   carries the identity of its request. The UI drops a result that does not fit the current
   state.
4. **Repaint.** A store requests a repaint when it changes. One notifier limits these repaints.
5. **Memory.** The UI thread never frees a large old snapshot.

## Test kit

`fresnel-testkit` has three parts. Each part runs in the test process and stops at its drop.

- `FaultProxy`: a TCP proxy before a server. Its modes are pass, hold of new connections,
  freeze, latency and a reset each period. Two calls reset all connections and make the open
  connections silent.
- `K3s`: k3s in Docker with a fixed address in its own Docker network. It gives a kubeconfig
  file and the address of the kube-apiserver, and it can restart the cluster.
- `FakeApiServer`: a local server with TLS, HTTP/2, HTTP/1.1, watch streams and the HTTP 101
  upgrade. Its scripted faults are no answer, HTTP 401, HTTP 429, GOAWAY and a low stream limit.

A fault test of the app puts a `FaultProxy` before `K3s` or before `FakeApiServer`. The proxy
and the fake server have counters and a `wait_for` call, so that a test waits for a signal and
not for a fixed time. A task of the kit that ends is a fault of the test equipment: the kit
prints one line for it, and `task_failures` gives the list.

## Tests

- **Unit tests** run with no network. A test of a timer uses paused time. The tests of
  `fresnel-net` for a time limit use an HTTP/2 server on pipes in memory, so each time is exact.
- **Socket tests** use the fake API server and the fault proxy on 127.0.0.1. They need no
  Docker and run with the unit tests. Real sockets need real time.
- **UI tests** drive the views through `egui_kittest` with no window.
- **Program tests** start `fresnel-cli` as a child process with the environment of a program
  from Finder. The home folder is a folder of the test, so the login shell reads its profile.
- **Cluster tests** use k3s in Docker through the fault proxy. They have the attribute
  `#[ignore]`, and `cargo test --workspace -- --ignored` runs them.
- CI runs the format check, the lints, the unit tests, the UI tests and the size check on
  macOS and Linux for each push. A second job on Linux runs the cluster tests.

## Guards

- `Cargo.toml` holds the one lint table and the one version table of the workspace.
- `clippy.toml` refuses a direct `tokio::spawn` and a direct `std::thread::spawn`.
- `fresnel-ui` refuses index access without a check.
- `cargo xtask check-sizes` checks the limits of the documents and the source files.
