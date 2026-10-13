# Architecture

This page gives the structure of fresnel. [decisions.md](decisions.md) gives the reasons.

## Terms

- **Cluster**: one kubeconfig context.
- **Channel**: one long-lived HTTP/2 connection to a cluster.
- **Watch**: one stream of changes for one kind in one cluster.
- **Store**: the copy in memory of all objects of one kind in one cluster.
- **Snapshot**: a view of a store that a reader holds. It does not change.
- **Task**: one background activity with a visible state.
- **Journal**: the list of events, warnings and errors that the app shows.

## Design rules

These rules have priority over features.

1. Each task has an explicit state, and the UI shows it. A task that ends or panics gets a
   final state, a journal entry and a restart.
2. Each request has a time limit. A data stream of the user has no limit on its progress.
3. The app opens few TCP connections. Each new connection uses the hedged connect.
4. Each view shows how fresh its data is.
5. The app keeps old data visible during an error. No empty list stands for an error.
6. The app does not repeat a write when its result is not known. It reads the object.
7. The app starts no helper process. The only exception is the credential plugin.
8. Each object carries the identity of its cluster. No "current cluster" global exists.
9. The lints refuse an ignored error. One function starts and watches each background task.
10. Tests with injected faults prove the recovery times. CI runs them.
11. One clear pattern exists for each repeated job. Documents stay short.
12. Each buffer has a size limit. A buffer that drops data shows a counter.

## Crates

An arrow is a permitted dependency, and there is no other one. A test can use `fresnel-testkit`.

```text
fresnel (program) ------> fresnel-ui ------> fresnel-core ------> fresnel-net
fresnel-cli (program) ---------------------> fresnel-core
fresnel-testkit ------------------------------------------------> fresnel-net
xtask
```

- `fresnel-net`: connections, the credential plugin and the time limits. It has no dependency
  on kube. It is the lowest crate, thus it also holds the task start function.
- `fresnel-core`: the domain. Clusters, the watch loop, stores, indexes, the kinds and the
  journal; later also search and operations. It has no dependency on egui. The crates above
  it reach the connection layer through the name `fresnel_core::net`.
- `fresnel-ui`: the egui widgets and views. It uses the object types of `k8s-openapi` and
  never the types of kube.
- `fresnel`: the desktop program. The window, the runtime, the log file and the quit sequence.
- `fresnel-cli`: the command line program for tests and fault diagnosis. `doctor` measures the
  connection to a cluster and reads no object. `watch` shows the state of each store.
- `fresnel-testkit`: the tools of the tests. It can use `fresnel-net`. Thus a test of
  `fresnel-net` that needs the test kit is a file in `tests/`, not a unit test.
- `xtask`: the size check and the macOS bundle. It uses no other crate of the workspace.

## Borders in the code

- Only one module of `fresnel-core` holds kube clients: `cluster/gateway.rs`. Each function of
  this module applies its time limit and the rule for writes. Only `store/items.rs` and
  `index/links.rs` use the map types of `imbl`.
- One module wraps each outside UI crate: `egui_table` in `widgets/table.rs`, `egui_tiles` in
  `shell/dock.rs`, `egui_plot` in `widgets/chart.rs`. An egui upgrade then changes few files.
- Each library crate has one error type with a class: auth, forbidden, conflict, timeout,
  network, server, invalid. An item is private to its crate unless `lib.rs` lists it.

## Background tasks

`fresnel_net::task::Tasks::start` is the one function that starts a background task on the
runtime. `Tasks::start_thread` starts one on a thread of its own, for a worker.

- The caller gives a name, a restart rule and a body. The body builds one run of the task.
  `Restart::Always` runs it again after each end, with a pause that grows to a limit.
- Each task has a state: running, backoff or ended. `Tasks::snapshot` gives a copy of all. A
  run that ends gives one report to the journal: its end, and the pause before the next run.
- The caller owns a `TaskHandle`. The task stops when the handle drops. A thread ends by
  itself then: its body gets a `Stop`, and its input ends after the handle.

## Network layer

`fresnel_net::Cluster` owns the connections of one cluster and runs its credential plugin. A
request carries a `RequestKind`, which selects the connection, the time limits and gzip.

```text
request -> time limit -> throttle (HTTP 429) -> token and HTTP 401 rule -> lane of the kind
lane: watches | requests | logs  (HTTP/2 channels)      upgrade (HTTP/1.1, warm set)
new connection: resolver -> hedged connect (TCP and TLS) -> one protocol by ALPN
```

- **Channel**: one HTTP/2 connection with its own task. The task connects at the start and
  after each loss, sends a ping each 10 s, and sends a light request each 30 s when no stream
  is open. With a plugin and no token, it sends nothing. Live means: a request got an answer.
- **Group**: the channels of one purpose. At 70 percent of the stream limit of the server,
  one more connection opens, to a maximum of 3. Then a request waits, with a visible state.
- **Hedged connect**: one more attempt starts after 1.5 connect times. The first one wins.
- **Credential**: one task runs the plugin, one run at a time. A request waits 2 s at most.
  After a second HTTP 401, one small request proves the token.
- **Upgrade client**: exec, attach and port forward, on warm connections with a test.
- **Signal**: a wake, a network change and "retry now" raise the one signal of a cluster. It
  ends each retry delay, and each channel tests its connection with a limit of 3 s.
- **State**: `Cluster::status` gives a copy of each state. `Cluster::changes` tells a change.

## Clusters

`fresnel-core` makes a connected cluster from a kubeconfig context. Its name is the `ClusterId`.

- **Kubeconfig**: `kubeconfig::load` reads each file of `KUBECONFIG` of the login shell and then
  `~/.kube/config`. For a name in two files, the first file wins. A context that the app
  cannot use stays, with its error. At a reload, a file that does not load keeps its clusters.
- **Registry**: one entry for each context, with the connections of each connected cluster.
- **Gateway**: the requests to one cluster, with one function for each request kind: get, list
  page, watch stream, log stream, send and upgrade. The kind gives the connection and the time
  limits. A failed write tells its fate: not sent, refused or not known. Each read has a size limit.
- **App core**: `app::Core` owns the registry, the tasks and the journal. It does no work
  before its start. Then one task reads the login shell environment and the kubeconfig files,
  connects each selected cluster and runs each command of the UI. One task for each cluster
  puts its state changes into the journal. The core does not start the stores yet.
- **State file** and **journal**: `state.json` holds the selection of the last run. The
  journal holds its last 10,000 entries in memory, and a sink takes each entry.

## Stores and indexes

`store::ClusterStores` holds one store for each kind of one cluster, for all namespaces.

- **Kinds**: `kinds` has the one list of the kinds. A `Kind` is a typed identity. A tier A
  kind has a store with full objects, a tier B kind with the metadata only: no store holds a
  value of a Secret. Lease and Endpoints (tier C) have no store. One file for each tier A
  kind gives its health value, and `PodStatus` gives the row of `kubectl get pods`.
- **Watch loop**: one task for each store. It reads a list in pages, 4 lists of a cluster at
  a time: the first lists in the sequence of the kinds, each later list in its turn. Then it
  follows a watch with bookmarks, which the server ends after 60 to 90 s. The next watch starts
  after the last version, and the store stays live. Only HTTP 410 gives a new list.
- **Failures**: a failure gives a pause that grows to 3 s, and the signal of the cluster ends
  each pause. A store sends no request while its channel is not live, and it is not live
  itself then: the channel counts the loss of a connection, not each watch.
- **States**: waiting (for a channel, a credential, a list slot or the start of the watch),
  list in progress, live, attempt, pause, throttled, forbidden, not supported, internal error
  and stopped. The objects stay in each state. A panic of the task gives the state "internal
  error", a report and a restart.
- **Snapshot**: the task applies each event at once and publishes each 30 ms at most. The
  generation grows only when an object changes. An object goes in with no `managedFields`
  and no copy of `kubectl apply`. The store of the Events has a maximum count.
- **Write result**: `Snapshot::is_pending` compares the answer of a write with a snapshot. It
  is true until the store has that version of the object. No store keeps a mark for a write.
- **Published state**: `published::Publisher` gives each value of a state to its readers as
  a copy of one pointer. One worker thread frees a value that a reader held.
- **Indexes**: `index::ClusterIndexes` has one task for each index, which applies the
  differences of two snapshots of its stores. The owner index gives the objects and the pods
  below an owner, also in an owner loop, and the pods behind a Service. The reference index
  gives the pods that use a ConfigMap, a Secret, a ServiceAccount or a Node. The Event index
  gives the Events of an object.
- **Basis**: each answer of an index tells if it is current, old (a store is not live), not
  loaded (a store had no complete list) or not followed (its task failed or stopped).

## Contract between the UI and the core

1. **State.** `Core::view` gives a copy of each state, and a `published::Reader` the snapshot
   of a store. The UI never gets a lock that can stop a task. A view holds no handle to the core.
2. **Commands.** A view returns a `Command`, and the task of the core runs it.
3. **Workers.** The UI computes rows on a worker thread. A result carries the identity of its
   request, and the UI drops a result that does not fit the current state.
4. **Repaint.** A change of a state asks for a repaint. One notifier limits the repaints to 60
   in one second. The core does not know egui: the window gives it the wake function.
5. **Memory.** The UI thread never frees a large old snapshot: see "Published state" above.

## Desktop program

- **Start**: the folders, the lock of the process, the log file, the panic hook, the runtime
  and the core. The core starts after the window is open.
- **Quit**: the core stops its tasks, closes each connection and ends a plugin that runs. Then
  the runtime ends and the log file reaches the disk. `cargo xtask install` builds the bundle.

## Test kit

`fresnel-testkit` has four parts. Each part runs in the test process and stops at its drop.

- `FaultProxy`: a TCP proxy that holds, freezes, delays, resets or silences connections.
- `K3s`: k3s in Docker, with the address of its kube-apiserver, `kubectl` and a restart.
- `FakeApiServer`: TLS, HTTP/2, HTTP/1.1, watch streams, the HTTP 101 upgrade, and faults.
- `Home`: a home folder with a shell profile, for a start with the environment of Finder.

## Tests

- **Unit tests** run with no network and with paused time. A time limit of `fresnel-net` runs
  on an HTTP/2 server in memory, the watch loop on a source with a script, and the indexes,
  the Pod status and the health values on `fixtures/k3s`: a copy of a scene on k3s.
- **Socket tests** use the fake API server and the fault proxy on 127.0.0.1, with no Docker.
- **UI tests** drive the views through `egui_kittest` with no window, on a fake core.
- **Program tests** start `fresnel-cli`, and `fresnel` with no window, as a child process
  with the environment of Finder and the home folder of a test.
- **Cluster tests** use k3s in Docker through the fault proxy, with the attribute `#[ignore]`.
  After each fault, a test of the stores asserts a recovery time and "store equal to the API".
  A test of the indexes compares them and the Pod status with the API and with kubectl.
- CI runs each check on macOS and Linux for each push, and the cluster tests on Linux.

## Guards

The lint table of `Cargo.toml`, `clippy.toml`, `cargo xtask check-sizes` and `cargo xtask
check-borders` hold these rules. [CONTRIBUTING.md](../CONTRIBUTING.md) gives each one.
