# Architecture

[decisions.md](decisions.md) gives the reasons, [CONTRIBUTING.md](../CONTRIBUTING.md) the rules.

## Terms

**Cluster**: one kubeconfig context. **Channel**: one HTTP/2 connection to a cluster. **Watch**:
one stream of changes of one kind in one cluster. **Store**: the objects of one kind of a cluster
in memory. **Snapshot**: a fixed view of a store. **Task**: a background job with a visible state.

## Design rules (they have priority over features)

1. A task has a state in the UI. An end or a panic gives a final state, an entry and a restart.
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
```

- `fresnel-net` (no kube): connections, the credential plugin, the time limits, the task start.
- `fresnel-core`: the domain: clusters, the watch loop, stores, indexes, kinds, search, commands,
  operations, metrics, logs, settings, journal. No egui; the crates above name `fresnel_core::net`.
- `fresnel-ui`: the egui widgets and views. It uses the types of `k8s-openapi`, never of kube.
- `fresnel`: the desktop program. The folders, the lock of the process, the log file, the panic
  hook, the runtime and the core, which starts after the window opens; the quit stops its tasks.
- `fresnel-cli`: the command line program for tests and fault diagnosis: `doctor`, `watch`, `soak`.
- `fresnel-testkit`: test tools on `fresnel-net`, and the soak harness `fresnel-soak`, a program.
- `xtask` (no other crate): checks, soak, the macOS bundle, the Windows program, README pictures.

## Borders in the code

- Only `cluster/gateway.rs` of `fresnel-core` holds kube clients. Each of its functions applies
  its time limit and the rule for writes. Only four modules use `imbl`.
- One module wraps each outside UI crate: `widgets/table.rs` and `widgets/chart.rs`.
- Each library crate has one error type with a class: auth, forbidden, conflict, timeout,
  network, server, invalid. An item is private to its crate unless `lib.rs` lists it.
- The `platform` module of a crate holds each difference of macOS and Windows: the environment,
  the process tree of a child, the time asleep, the folders, the lock, App Nap, the notifications.

## Background tasks

`fresnel_net::task::Tasks::start` starts each task, `start_thread` a worker thread, with a name, a
restart rule and a body for one run; the task stops when its handle drops. A task is running, in
backoff or ended (`Tasks::snapshot`), and each end of a run is a report.

## Network layer

`fresnel_net::Cluster` owns the connections of one cluster and runs its credential plugin. A
request carries a `RequestKind`, which selects the connection, the time limits and gzip.

```text
request -> time limit -> throttle (429, Retry-After) -> token and HTTP 401 rule -> lane of the kind
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
- **Signal**: a wake, a network change and "retry now" end each retry delay of a cluster, and
  each channel tests its connection in 3 s. **Outage**: after a minute with no connect, a cluster
  gets one attempt each 5 s. `Cluster::status` gives a copy of each state, `changes` a change.

## Clusters

`fresnel-core` makes a connected cluster from a kubeconfig context. Its name is the `ClusterId`.

- **Kubeconfig**: `kubeconfig::load` reads each file of `KUBECONFIG`, then `.kube/config` of the
  home folder and the files of the settings. The first file wins a name.
- **Registry**: one entry for each context. **Gateway**: get, list page, watch stream, log stream,
  send and upgrade. A failed write tells its fate, and each read has a size limit.
- **Manifest**: `manifest` reads the YAML of an object, and checks, saves and merges an editor.
- **Keys** (`keys`): one Secret or ConfigMap with each value, as text or as size and type. A
  `Draft` saves a merge patch of its changed keys with the version of its read, so no other key
  changes, and HTTP 409 is a conflict. The core follows the object of the open key view.
- **Logs** (`logs`): a session for each view of `ShowLogs` runs a source for each container, whose
  new stream starts at the second of its latest line with a marker; `SaveLog` writes a new file.
- **Operations** (`ops`): a function builds each request of plan section 10, with a unit test of
  its URL and body. An `Order` is a job in its own task, with steps and a state, not two at once;
  the jobs of a text run in turn. A write goes out one time; an unknown result gets a read. A
  cancel or a store that follows nothing ends a wait. A drain has a task for each pod. **Write
  result**: `Snapshot::is_pending`. **Permissions**: rules or a review, dropped by HTTP 403.
- **App core**: `app::Core` owns the registry, the tasks and the journal. One task reads the
  settings, the environment and the kubeconfig files, connects each selected cluster and runs each
  command; one gives the notices of a failed job and of a lost cluster to `set_notifier`.
- **Files**: `state.json` keeps the selection and the table layouts; `config.toml` the settings of
  `config::table`, with bounds. A look reads both again after a change. **Journal**: 10,000 entries.

## Stores, indexes, search and commands

`store::ClusterStores` holds one store for each kind of one cluster, for all namespaces.

- **Kinds**: `kinds` has the one list of the kinds; a `Kind` is a typed identity. The file of
  each kind implements `Describe`: sidebar group, tier, columns with the text of `kubectl get`,
  search text, health value and actions. A tier B store has the metadata only, tier C none.
- **Watch loop**: one task for each store. It reads a list in pages, 4 lists of a cluster at a
  time, then follows a watch with bookmarks, which the server ends after 60 to 90 s; the next one
  starts after the last version, and HTTP 410 gives a list. A failure gives a pause up to 3 s,
  which the signal ends. No request goes while the channel is not live: it counts the loss.
- **States**: waiting (for a channel, a credential, a list slot or its watch), list in progress,
  live, attempt, pause, throttled, forbidden, not supported, internal error and stopped.
- **Snapshot**: published each 30 ms at most; its generation grows only when an object changes.
  No `managedFields`, no value of a Secret, a limit of Events. `Publisher` copies one pointer.
- **On demand**: `ClusterStores::demand` gives a view the full objects of any kind. The store of
  a tier B or C kind starts with its first holder and stops 60 s after the last one.
- **Indexes**: `index::ClusterIndexes` has one task for each index, which applies the differences
  of two snapshots: the owner index (also in a loop, and the pods of a Service), the reference
  index (the pods that use an object) and the Event index of each object. Each answer is current,
  old (a store is not live), not loaded (no complete list) or not followed (the task stopped).
- **Metrics**: one `metrics::Sampler` for each cluster reads PodMetrics and NodeMetrics each 15 s
  (one read each, no retry; each 5 minutes with no metrics API, and the APIService after a 503);
  each list is a poll store with a state. A series of a container, pod, workload, node, namespace
  or cluster keeps 1 h of samples and 24 h of minutes in a memory limit.
- **Quick search**: `search::run` reads the snapshot of each store of each selected cluster and
  sends no request: the full objects of tier A and of a shown kind, else the metadata of tier B.
  A fuzzy match on the name, a text match on the other fields; cluster and kind are filters.
  A hit has the mark of a store that is not live, and each store that it did not read is a note.
- **Command table**: `commands::TABLE` gives each command its identifier, label, default key,
  kinds and confirm rule. `Keys` is the key of each command after the settings.

## Contract between the UI and the core

1. **State.** `Core::view` gives a copy of each state, with no lock: the clusters and their
   connections; of each connected cluster the Pods, the indexes, the snapshots, the full objects
   of the kinds of `ShowKinds`, the key view, the permissions, the metrics, the server version, the
   logs and saves; the settings, the manifest, the editors and saves, a Secret create, jobs, journal.
2. **Commands.** A view returns a `Command`, and the task of the core runs it. **Workers**: below.
3. **Repaint.** A new snapshot, index, poll or log view asks for a repaint. One notifier limits
   them to 60 in one second, a setting. The core does not know egui: the window gives the wake.
4. **Memory.** A thread frees each old snapshot and the rows. A view on a hidden page has none.

## Window

`fresnel_ui::Shell` draws a frame from a `CoreView`: top bar, sidebar, a page of the sidebar, the
dock (Operations, Journal, Connections, a tab for each log view and editor) and status bar.

- **Table**: `widgets/table.rs` wraps `egui_table`. A kind gives a `Model`: columns, rows,
  their sequence and cells. The cursor and the selection hold objects (cluster, name, UID).
  The table owns the widths, the columns that show and a handle on each edge.
- **Worker** (`widgets/worker.rs`): one thread for each table and for the quick search. Its
  mailbox has one request and one result. New data does not stop a computation; a new query
  does. After a panic, the thread starts again with the request that waits; the view tells why.
- **Object page** (`views/objects`): the objects of one kind in each selected cluster and the
  panel: metadata, the sections of the kind (a folder of `views` for each sidebar group, and
  `sections::also_shown` for the other kinds that they read), conditions, related objects as
  links, Events and the Manifest. An object that left its store shows as deleted.
- **Actions** (`views/act`): the panel, the menus, a key and the quick search ask for an action
  of the command table. A gate closes it with the reason: a later milestone, the state of the
  object, the rules, a read-only cluster. A change asks in a dialog: an order for each object.
- **Quick search**: a dialog; its cursor holds a row by identity. A hit opens its object, with a
  note while its store lists, or "Show it" when a filter hides its row. One namespace filter.
- **Keys** (`views/keys`): the panel of a Secret or a ConfigMap names its object (`ShowKeys`) and
  edits a `Draft` behind the gate of "Edit keys": a hidden value until "Show", add, remove, the
  diff in a dialog before a save, a choice for each conflict. The New Secret dialog is `new-secret`.
- **Metrics** (`views/metrics`): CPU and Memory columns with bars, a tab with `egui_plot` charts.
  **Overviews** (`views/overview`): a worker counts; `cluster/version.rs` reads each server version.
- **Logs**, **Editor** (`views/logs`, `views/editor`): a tab for each view with a search and its own
  rows (`pane`, `text`); a worker filters a log (`ShowLogs`) and checks a text (`ShowEditors`). A
  save shows the diff of its dry run (`SaveEditor`); a YAML tab sends documents (`RunInTurn`).
- **Settings** (`views/settings`): a control for each entry of `config::table`, cluster and key; a
  draft goes to `EditSettings`. `shell/focus` moves the keys between four areas; `?` lists them.

## Tests

`fresnel-testkit` runs in the test process, and each part stops at its drop: `FaultProxy` holds,
freezes, delays, resets or silences connections; `K3s` is k3s in Docker, with agents as nodes;
`FakeApiServer` has TLS, HTTP/2, HTTP/1.1, watch and log streams, the HTTP 101 upgrade and faults.

- **Unit tests** run with no network and with paused time: `fresnel-net` on an HTTP/2 server in
  memory, the watch loop on a script, the indexes, health values and columns on two k3s scenes.
- **Socket tests** use the fake API server and the fault proxy on 127.0.0.1, with no Docker.
- **UI tests**: `egui_kittest` with no window, on a fake core; a table test holds its worker at a
  gate. The program tests `live`, `settings` and `k3s_*` draw the real core. Time: release build.
- **Program tests** start `fresnel-cli` and `fresnel` (no window) as from Finder, in a test home.
- **Cluster tests** use k3s in Docker, mostly through the fault proxy (`#[ignore]`). After a fault,
  a store test asserts a recovery time and "store equal to the API". The tests of the indexes,
  the kinds, the keys, the operations, the core commands, the metrics and the logs match kubectl.
- **Soak**: `fresnel-cli soak` checks each store of the core against the API; `fresnel-soak` adds k3s.
- CI runs each check on macOS, Linux and Windows at each push, and the cluster tests on Linux.
