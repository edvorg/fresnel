# Architecture

[decisions.md](decisions.md) gives the reasons, [CONTRIBUTING.md](../CONTRIBUTING.md) the rules.

Terms. **Cluster**: one kubeconfig context. **Channel**: one HTTP/2 connection to a cluster.
**Watch**: one stream of changes of one kind in one cluster. **Store**: the objects of one kind of
a cluster in memory. **Snapshot**: a fixed view of a store. **Task**: a background job with a state.

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
- `fresnel-core`: the domain: clusters, watch loop, stores, indexes, kinds, search, commands, jobs,
  metrics, logs, terminals, forwards, settings, journal. No egui; others name `fresnel_core::net`.
- `fresnel-ui`: the egui widgets and views. It uses the types of `k8s-openapi`, never of kube.
- `fresnel`: the desktop program. The folders, the lock of the process, the log file, the panic
  hook, the runtime and the core, which starts after the window opens; the quit stops its tasks.
- `fresnel-cli`: the command line program for tests and fault diagnosis: `doctor`, `watch`, `soak`.
- `fresnel-testkit`: test tools on `fresnel-net`, and the soak harness `fresnel-soak`, a program.
- `xtask` (no other crate): checks, soak, the macOS bundle, the Windows program, README pictures.

## Borders in the code

- Only `cluster/gateway.rs` of `fresnel-core` holds kube clients. Each of its functions applies
  its time limit and the rule for writes. Only four modules use `imbl`, only `terminal` alacritty.
- One module wraps each outside UI crate: `widgets/table.rs` and `widgets/chart.rs`.
- Each library crate has one error type with a class: auth, forbidden, conflict, timeout,
  network, server, invalid. An item is private to its crate unless `lib.rs` lists it.
- The `platform` module of a crate holds each difference of macOS and Windows: the environment,
  the process tree of a child, the time asleep, the bind and the reset of a local port, folders,
  the lock, App Nap, notices, fonts and the browser.

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

- **Channel**: one HTTP/2 connection whose task connects at the start and after each loss, pings
  each 10 s and, with no open stream and a token, sends a light request each 30 s. Live: an answer.
  A **group** of a purpose adds one at 70 percent of the stream limit, to 3; then a request waits.
- **Hedged connect**: one more attempt after 1.5 connect times; the first one wins. **Upgrade
  client**: exec, attach and port forward on warm connections with a test, each a **socket** that
  pings after 10 s and breaks with no answer in 5 s, unless its peer reads nothing (`socket`).
  **Local port** (`local`): the listener of a forward binds alone; a reset ends a broken client.
- **Credential**: one task runs the plugin. After a second HTTP 401, a request proves the token.
- **Signal**: a wake, a network change and "retry now" end each retry delay, and each channel and
  socket tests its connection in 3 s. **Outage**: after a minute with no connect, one attempt each
  5 s. `Cluster::status` gives a copy of each state, `changes` a change.

## Clusters

`fresnel-core` makes a connected cluster from a kubeconfig context. Its name is the `ClusterId`.

- **Kubeconfig**: `kubeconfig::load` reads each file of `KUBECONFIG`, then `.kube/config` of the
  home folder and the files of the settings. The first file wins a name.
- **Registry**: one entry for each context. **Gateway**: get, list page, watch stream, log stream,
  send and upgrade. A failed write tells its fate, and each read has a size limit. **Streams**
  (`stream`): exec, attach and the tunnel of a port forward, each on a socket of `upgrade`.
- **Manifest**: `manifest` reads the YAML of an object, and checks, saves and merges an editor.
- **Keys** (`keys`): one Secret or ConfigMap with each value, as text or as size and type. A
  `Draft` saves a merge patch of its changed keys with the version of its read, so no other key
  changes, and HTTP 409 is a conflict. The core follows the object of the open key view.
- **Logs** (`logs`): a session for each view of `ShowLogs` runs a source for each container, whose
  new stream starts at the second of its latest line with a marker; `SaveLog` writes a new file.
- **Terminal** (`terminal`): the screen of `alacritty_terminal` behind a fair lock and a guard of
  its parser; a session (`remote`) runs its exec or attach, and ends its shell at a close or a quit.
- **Forwards** (`forward`): a task for each rule with its listener and a tunnel for each client;
  the stores choose the pod, the channels tell the path. A forward keeps its cluster connected.
- **Operations** (`ops`): a function builds each request of plan section 10, with a unit test of
  its URL and body. An `Order` is a job in its own task, with steps and a state, not two at once;
  the jobs of a text run in turn. A write goes out one time; an unknown result gets a read. A
  cancel or a store that follows nothing ends a wait. A drain has a task for each pod. **Write
  result**: `Snapshot::is_pending`. **Permissions**: rules or a review, dropped by HTTP 403.
- **App core**: `app::Core` owns the registry, the tasks and the journal. One task reads the
  settings, the environment and the kubeconfig files, connects each selected cluster and runs each
  command; one gives the notices of a failed job or forward and a lost cluster to `set_notifier`.
- **Files**: `state.json` keeps the selection and the table layouts; `config.toml` the settings of
  `config::table` and the saved forwards. A look reads both after a change. **Journal**: 10,000.

## Stores, indexes, search and commands

`store::ClusterStores` holds one store for each kind of one cluster, for all namespaces.

- **Kinds**: `kinds` has the one list of the kinds; a `Kind` is a typed identity. The file of
  each kind implements `Describe`: sidebar group, tier, columns with the text of `kubectl get`,
  search text, health value and actions. A tier B store has the metadata only, tier C none.
- **Watch loop**: one task for each store. It reads a list in pages, 4 lists of a cluster at a
  time, then a watch with bookmarks, which the server ends after 60 to 90 s; HTTP 410 gives a list.
  A failure gives a pause up to 3 s, which the signal ends. A lost channel counts its loss.
- **States**: waiting (for a channel, a credential, a list slot or its watch), list in progress,
  live, attempt, pause, throttled, forbidden, not supported, internal error and stopped.
- **Snapshot**: published each 30 ms at most; its generation grows only when an object changes.
  No `managedFields`, no value of a Secret, a limit of Events. `Publisher` copies one pointer.
- **On demand**: `ClusterStores::demand` gives a view the full objects of any kind. The store of
  a tier B or C kind starts with its first holder and stops 60 s after the last one.
- **Indexes**: `index::ClusterIndexes` has one task for each index, which applies the differences
  of two snapshots: the owners (also in a loop, and the pods of a Service), the pods that use an
  object, the Events of an object. An answer is current, old, not loaded or not followed.
- **Metrics**: one `metrics::Sampler` for each cluster reads PodMetrics and NodeMetrics each 15 s
  (no retry; each 5 minutes with no metrics API), each list a poll store with a state. A series of
  a container, pod, workload, node, namespace or cluster keeps 1 h and 24 h in a memory limit.
- **Quick search**: `search::run` reads the snapshot of each store of each selected cluster and
  sends no request. A fuzzy match on the name, a text match on the other fields; cluster and kind
  are filters. A hit has the mark of a store that is not live; a store that it did not read, a note.
- **Command table**: `commands::TABLE` gives each command its identifier, label, default key,
  kinds and confirm rule. `Keys` is the key of each command after the settings.

## Contract between the UI and the core

1. **State.** `Core::view` copies each state, with no lock: the clusters, their links and the data
   of each connected one (`ClusterData`: stores, indexes, objects of `ShowKinds`, keys, access,
   metrics, version, logs); settings, manifest, editors, create, jobs, terminals, forwards, journal.
2. **Commands.** A view returns a `Command`, and the task of the core runs it. **Workers**: below.
3. **Repaint.** A new snapshot, index, poll or log view asks for a repaint. One notifier limits
   them to 60 in one second, a setting. The core does not know egui: the window gives the wake.
4. **Memory.** A thread frees each old snapshot and the rows. A view on a hidden page has none.

## Window

`fresnel_ui::Shell` draws a frame from a `CoreView`: top bar, sidebar, a page of the sidebar, the
dock (Operations, Forwards, Journal, Connections, a tab of each log, editor, terminal), status bar.

- **Table**: `widgets/table.rs` wraps `egui_table`. A kind gives a `Model`: columns, rows, sequence,
  cells. Cursor and selection hold objects; the table owns widths, shown columns, edge handles.
- **Worker** (`widgets/worker.rs`): one thread for each table and for the quick search. Its
  mailbox has one request and one result. New data does not stop a computation; a new query
  does. After a panic, the thread starts again with the request that waits; the view tells why.
- **Object page** (`views/objects`): the objects of one kind in each selected cluster and the
  panel: metadata, the sections of the kind (a folder of `views` for each sidebar group), its
  conditions, related objects as links, Events and the Manifest. A deleted object shows as such.
- **Actions** (`views/act`): the panel, the menus, a key and the quick search ask for an action
  of the command table. A gate closes it with the reason: a later milestone, the state of the
  object, the rules, a read-only cluster. A change asks in a dialog: an order for each object.
- **Quick search**: a dialog; its cursor holds a row by identity. A hit opens its object, with a
  note while its store lists, or "Show it" when a filter hides its row. One namespace filter.
- **Keys** (`views/keys`): the panel of a Secret or a ConfigMap names its object (`ShowKeys`) and
  edits a `Draft`: a hidden value until "Show", add, remove, a diff before a save, a conflict.
- **Metrics** (`views/metrics`): CPU and Memory columns with bars, a tab with `egui_plot` charts.
  **Overviews** (`views/overview`): a worker counts; `cluster/version.rs` reads each server version.
- **Logs**, **Editor** (`views/logs`, `views/editor`): a tab for each view with a search and its own
  rows (`pane`, `text`); a worker filters a log (`ShowLogs`) and checks a text (`ShowEditors`). A
  save shows the diff of its dry run (`SaveEditor`); a YAML tab sends documents (`RunInTurn`).
- **Terminal** (`widgets/terminal`): copies the rows of a `Terminal` under its lock and paints the
  copy: style runs, box characters as shapes, the cursor; keys, paste, pointer and size go to the
  terminal. `views/terminal`: a tab for each terminal; Shell, Attach, Debug, Node shell open one.
- **Settings** (`views/settings`): a control for each entry of `config::table`, cluster and key; a
  draft goes to `EditSettings`. `shell/focus` moves the keys between four areas; `?` lists them.
- **Forwards** (`views/forwards`): the tab with the state of each forward and its buttons, the
  dialog of the Forward button of each port, the counts of the status bar, the question at a close.

## Tests

`fresnel-testkit` runs in the test process, and each part stops at its drop: `FaultProxy` holds,
freezes, delays, resets or silences connections; `K3s` is k3s in Docker, with agents as nodes;
`FakeApiServer` has TLS, HTTP/2, HTTP/1.1, watch and log streams, a WebSocket pod and faults.

- **Unit tests** run with no network and with paused time: `fresnel-net` on an HTTP/2 server in
  memory, the watch loop on a script, the indexes, health values and columns on two k3s scenes.
- **Socket tests** use the fake API server and the fault proxy on 127.0.0.1, with no Docker.
- **UI tests**: `egui_kittest` with no window, on a fake core; a table test holds its worker at a
  gate. The program tests `live`, `settings` and `k3s_*` draw the real core. Time: release build.
- **Program tests** start `fresnel-cli` and `fresnel` (no window) as from Finder, in a test home.
- **Cluster tests** (`#[ignore]`) use k3s in Docker, mostly behind the fault proxy: a store test
  asserts a recovery time and "store equal to the API" after a fault, the others match kubectl.
- **Soak**: `fresnel-cli soak` checks the stores against the API and a forward; `fresnel-soak`: k3s.
- CI runs each check on macOS, Linux and Windows at each push, and the cluster tests on Linux.
