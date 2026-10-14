# Architecture

This page gives the structure of fresnel, [decisions.md](decisions.md) the reasons, and
[CONTRIBUTING.md](../CONTRIBUTING.md) the rules, which the lints and `cargo xtask` check.

## Terms

**Cluster**: one kubeconfig context. **Channel**: one long-lived HTTP/2 connection to a
cluster. **Watch**: one stream of changes for one kind in one cluster. **Store**: the copy in
memory of all objects of one kind in one cluster. **Snapshot**: a view of a store that a reader
holds; it does not change. **Task**: one background activity with a visible state.

## Design rules (they have priority over features)

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
- `fresnel-core`: the domain. Clusters, the watch loop, stores, indexes, the kinds, the quick
  search, the command table, the operations, the settings and the journal. It has no
  dependency on egui. The crates above it reach the connection layer as `fresnel_core::net`.
- `fresnel-ui`: the egui widgets and views. It uses the types of `k8s-openapi`, never of kube.
- `fresnel`: the desktop program. The window, the runtime, the log file and the quit sequence.
- `fresnel-cli`: the command line program for tests and fault diagnosis: `doctor` and `watch`.
- `fresnel-testkit`: test tools on `fresnel-net`; a `fresnel-net` test that uses it is in `tests/`.
- `xtask`: the size check and the macOS bundle. It uses no other crate of the workspace.

## Borders in the code

- Only `cluster/gateway.rs` of `fresnel-core` holds kube clients. Each of its functions applies
  its time limit and the rule for writes. Only two modules use the map types of `imbl`.
- One module wraps each outside UI crate: now `egui_table` in `widgets/table.rs` only.
- Each library crate has one error type with a class: auth, forbidden, conflict, timeout,
  network, server, invalid. An item is private to its crate unless `lib.rs` lists it.

## Background tasks

- `fresnel_net::task::Tasks::start` starts each task, `start_thread` a worker thread, with a
  name, a restart rule and a body for one run. `Restart::Always` runs it again after each end,
  with a pause that grows; `Restart::Never` is a job. The task stops when its handle drops.
- A task is running, in backoff or ended (`Tasks::snapshot`). Each end of a run is a report.

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
- **Hedged connect**: one more attempt after 1.5 connect times; the first one wins. **Upgrade
  client**: exec, attach and port forward, on warm connections with a test.
- **Credential**: one task runs the plugin. After a second HTTP 401, a request proves the token.
- **Signal**: a wake, a network change and "retry now" raise the signal of a cluster. It ends
  each retry delay (the wait of a connect one time in 10 s at most), and each channel tests its
  connection in 3 s. **Outage**: after a minute with no connect, a cluster gets one attempt
  each 5 s, in turns. `Cluster::status` gives a copy of each state, `Cluster::changes` a change.

## Clusters

`fresnel-core` makes a connected cluster from a kubeconfig context. Its name is the `ClusterId`.

- **Kubeconfig**: `kubeconfig::load` reads each file of `KUBECONFIG` of the login shell and then
  `~/.kube/config`. The first file wins a name. A context that the app cannot use stays.
- **Registry**: one entry for each context. **Gateway**: get, list page, watch stream, log stream,
  send and upgrade. A failed write tells its fate, and each read has a size limit.
- **Manifest**: `manifest::fetch` gives the YAML of one object, by default with no `managedFields`.
- **Keys** (`keys`): one Secret or ConfigMap with each value, as text or as size and type. A
  `Draft` saves a merge patch of its changed keys with the version of its read, so no other key
  changes, and HTTP 409 is a conflict. The core follows the object of the open key view.
- **Operations** (`ops`): a function builds each request of plan section 10, with a unit test
  of its URL and body. An `Order` runs as a job in its own task, with steps and a state, not two
  times at once. A write goes out one time; after an unknown result a step reads the object. A
  cancel, or a store that follows nothing, ends a wait. **Permissions**: rules or a review decide
  an action; HTTP 403 drops them. **Write result**: `Snapshot::is_pending` (answer and store).
- **App core**: `app::Core` owns the registry, the tasks and the journal. After its start, one
  task reads the settings file, the login shell environment and the kubeconfig files, connects
  each selected cluster and runs each command. A cluster has its stores, indexes and a task.
- **Files**: `state.json` keeps the selection and the table layouts of the last run. The core
  reads `config.toml` (its version, the key of each command) and never writes it; its first 20
  problems go to the journal with their line and column. **Journal**: 10,000 entries.

## Stores, indexes, search and commands

`store::ClusterStores` holds one store for each kind of one cluster, for all namespaces.

- **Kinds**: `kinds` has the one list of the kinds; a `Kind` is a typed identity. The file of
  each kind implements `Describe`: sidebar group, tier, columns with the text of `kubectl get`,
  search text, health value and actions. A tier B store has the metadata only, tier C none.
- **Watch loop**: one task for each store. It reads a list in pages, 4 lists of a cluster at
  a time, in the sequence of the kinds. Then it follows a watch with bookmarks, which the server
  ends after 60 to 90 s. The next watch starts after the last version. HTTP 410 gives a list.
- **Failures**: a pause that grows to 3 s, which the signal of the cluster ends. A store sends
  no request while its channel is not live: the channel counts the loss of a connection.
- **States**: waiting (for a channel, a credential, a list slot or its watch), list in
  progress, live, attempt, pause, throttled, forbidden, not supported, internal error (a
  panic of the task, with a report and a restart) and stopped. The objects stay in each one.
- **Snapshot**: the task publishes each 30 ms at most, and the generation grows only when an
  object changes. An object has no `managedFields`, a Secret no value. Events have a limit.
  `published::Publisher` gives each value to its readers as a copy of one pointer.
- **On demand**: `ClusterStores::demand` gives a view the full objects of any kind. The store of
  a tier B or C kind starts with its first holder and stops 60 s after the last one.
- **Indexes**: `index::ClusterIndexes` has one task for each index, which applies the
  differences of two snapshots: the owner index (also in a loop, and the pods of a Service),
  the reference index (the pods that use an object) and the Event index of each object.
- **Basis**: each answer of an index tells if it is current, old (a store is not live), not
  loaded (a store had no complete list) or not followed (its task failed or stopped).
- **Quick search**: `search::run` reads the snapshot of each store of each selected cluster and
  sends no request: the full objects of tier A and of a shown kind, else the metadata of tier B.
  A fuzzy match on the name, a text match on the other fields; cluster and kind are filters.
  A hit has the mark of a store that is not live, and each store that it did not read is a note.
- **Command table**: `commands::TABLE` gives each command its identifier, label, default key,
  kinds and confirm rule. `Keys` is the key of each command after the settings.

## Contract between the UI and the core

1. **State.** `Core::view` gives a copy of each state: the clusters with their connections; of
   each connected cluster the Pods, the indexes, the snapshot of each store, the full objects of
   each kind that `ShowKinds` names, the key view and the permissions; the settings; the last
   manifest read, the last create of a Secret, the jobs and the journal. No lock, no handle.
2. **Commands.** A view returns a `Command`, and the task of the core runs it.
3. **Workers.** A view computes its rows on a worker thread: see "Window".
4. **Repaint.** A new snapshot or index asks for a repaint. One notifier limits the repaints
   to 60 in one second. The core does not know egui: the window gives it the wake.
5. **Memory.** A thread frees each old snapshot and the rows. A view on a hidden page has none.

## Window

`fresnel_ui::Shell` draws one frame from a `CoreView`: the top bar, the sidebar with the kinds
and their counts, the page of the open kind, the dock (Journal, Connections) and the status
bar. It holds the page of one kind and names that kind to the core with `ShowKinds`.

- **Table**: `widgets/table.rs` wraps `egui_table`. A kind gives a `Model`: columns, rows,
  their sequence and cells. The cursor and the selection hold objects (cluster, name, UID).
  The table owns the widths, the columns that show and a handle on each edge.
- **Worker** (`widgets/worker.rs`): one thread for each table and for the quick search. Its
  mailbox has one request and one result. New data does not stop a computation; a new query
  does. After a panic, the thread starts again with the request that waits; the view tells why.
- **Object page** (`views/objects`): the objects of one kind in each selected cluster, followed
  by the differences of two snapshots, and the detail panel: metadata, conditions, related
  objects as links, Events and the Manifest tab. An object that left its store shows as deleted.
- **Freshness**: a row of a store that is not live has a mark; a note above the table tells why.
- **Commands**: the shell reads the key of each command before the widgets and runs it; Back
  and Forward take a step for each press. The menu of a row lists the actions of its kind.
- **Quick search**: a dialog with the last text selected; the cursor holds a row by identity, and
  Enter waits for the hits of the text. A hit opens its object, with a note while its store lists,
  or "Show it" when a filter hides its row. The top bar has one namespace filter for each page.
- **Keys** (`views/keys`): the panel of a Secret or a ConfigMap names its object (`ShowKeys`) and
  edits a `Draft`: a hidden value of a Secret until "Show", add, remove, the diff in a dialog
  before a save, a choice for each conflict. The command `new-secret` opens the New Secret dialog.
- **Program**: the folders, the lock of the process, the log file, the panic hook, the runtime
  and the core, which starts after the window is open. The quit stops each task of the core.

## Tests

`fresnel-testkit` runs in the test process, and each part stops at its drop: `FaultProxy` holds,
freezes, delays, resets or silences connections; `K3s` is k3s in Docker; `FakeApiServer` has
TLS, HTTP/2, HTTP/1.1, watch streams, the HTTP 101 upgrade and faults; `Home` is a home folder.

- **Unit tests** run with no network and with paused time: `fresnel-net` on an HTTP/2 server in
  memory, the watch loop on a script, the indexes, health values and columns on two k3s scenes.
- **Socket tests** use the fake API server and the fault proxy on 127.0.0.1, with no Docker.
- **UI tests**: `egui_kittest` with no window, on a fake core; a table test holds its worker at a
  gate, and two tests use the real core. The time limits hold in a release build.
- **Program tests** start `fresnel-cli` and `fresnel` (no window) as from Finder, in a test home.
- **Cluster tests** use k3s in Docker, mostly through the fault proxy (`#[ignore]`). After a fault,
  a store test asserts a recovery time and "store equal to the API". The tests of the indexes,
  the kinds, the keys and the operations (on two clusters) compare each result with kubectl.
- CI runs each check on macOS and Linux for each push, and the cluster tests on Linux.
