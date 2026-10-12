# Decisions

Each section gives one decision and its reason in one short paragraph.
`cargo xtask check-sizes` checks this form. The first seven decisions are decisions of the user.

## Rust, a native app and egui

The user selected the language Rust, a native app and the GUI toolkit egui at the start of the
project. The app thus has no browser engine and no local web server between the user and the
cluster.

## A private repository with direct changes to master

The repository is private. For now, each change goes directly to `master` with no pull
request. The user decided this for the first phase. CI runs on each push.

## Tests use local test clusters only

Automated tests use k3s in Docker and no other cluster. No object from a real cluster goes
into the repository. A manual verification run can use a real cluster. Such a run changes
nothing and reads no Secrets.

## Logs, the terminal and YAML editing come last

The log viewer, the terminal and the YAML editor are the last milestones. They are own widgets
with much work, and the base app must be stable before them.

## A demonstration ends each milestone

Each milestone ends with an app that the user can start, a short list of steps to try, and the
test results. The user then decides if and how the work continues.

## The license waits

The project has no license until the user publishes it. Thus no manifest has a `license` field.

## Research and prototypes stay outside the repository

The research reports and six tested prototypes are in a folder outside this repository. A
reviewer ran each prototype again and listed its defects. Code moves into the repository part
by part, with each defect of its review corrected.

## An own network layer

`fresnel-net` owns the connections. The standard kube client opened 31 TCP connections for 30
watches, and on a bad network each new connection can get no answer for seconds. The own layer
used 2 connections. It finds a silent connection with a ping, and it starts a second connect
attempt when the first is slow. With 12 percent held connections, 15.5 percent of plain
connects took 1 second or more. No hedged connect did.

## kube behind one module, with an exact pin

Only `cluster/gateway.rs` in `fresnel-core` holds kube clients. `fresnel-net` has no kube
dependency, and `fresnel-ui` uses only the object types of `k8s-openapi`. A kube upgrade thus
needs the review of one module. The kube version has an exact pin, because `fresnel-net`
replaces parts of the kube transport.

## The app runs the credential plugin

The app runs the credential plugin of the kubeconfig itself: one run at a time for each
cluster, with a time limit and a visible state. In the prototype test, the standard client ran
the plugin 3 times, and a plugin that hangs blocked it with no limit. The token stays in
memory only.

## One function starts each background task

In each prototype review, a task that ended or panicked left a state that looked good and data
that did not change. Thus `fresnel_net::task::Tasks::start` is the only way to start a task.
It gives the task a name, a state, a report for the journal and a restart with backoff. A lint
refuses a direct `tokio::spawn` and a direct `std::thread::spawn`.

## State goes to the UI as a copy

The UI reads a copy of each state and never holds a lock that a task needs. A slow frame thus
cannot stop a task. Data that arrives during a frame cannot change what the frame reads. In
the UI prototype, such a change caused two index errors that stopped the UI thread.

## No repeat of a write with an unknown result

After a write with no answer, the app reads the object and shows the result. A second send
could apply the change two times. The app repeats a write only after a definite refusal by the
server, with a visible state and a limit.

## No kubectl and no helper process

The app talks to the API server directly. A helper process adds a start time, a second place
for errors, and a process that can stay alive after the app. The only exception is the
credential plugin of the kubeconfig.

## Each object carries its cluster

No "current cluster" global exists. Each object, request and task carries the identity of its
cluster. Earlier tools showed the data of the first selected cluster only, or sent an action
to the incorrect cluster. With the identity in each object, the code has no default cluster to
use by mistake.

## One wrapper module for each outside UI crate

egui changes its API about each quarter. `egui_table`, `egui_tiles` and `egui_plot` each have
one wrapper module in `fresnel-ui`. An upgrade thus changes few files, and the views do not
see the outside types.

## Lints refuse an ignored error

The lint table denies an ignored result, `unwrap` and `expect`, and, in `fresnel-ui`, index
access without a check. A hidden error was the cause of data that stopped with no message in
the earlier tools. CI makes each warning an error.

## Size limits for files and documents

A source file has 400 lines or fewer, and each document has a limit. `cargo xtask check-sizes`
checks the limits in CI. A short file holds one task. A short document gets read, and a reader
finds the rule that applies.

## Fault tests on a real kube-apiserver

The tests run on k3s in Docker through a fault proxy that makes held connections, silent
connections, resets and latency. The test kit publishes only the kube-apiserver port of k3s
(6444), not the supervisor port (6443). The supervisor serves the same API with other HTTP/2
settings: a stream limit of 250 in place of 100, and no close of an idle connection. The
container has a fixed address, because k3s does not start again with a new one.

## A fake API server on real sockets

k3s cannot refuse a token for a time, send GOAWAY on order or give no answer to one path. The
fake API server of the test kit makes these faults. It uses real sockets with TLS, HTTP/2 and
HTTP/1.1, so the app runs its real network code against it. It is not a Kubernetes server: a
test sets each body and sends each watch event.

## The fault proxy waits before a reset

On the loopback of macOS, a TCP reset can be lost when data is on the way: 11 of 200 resets
in a test. The client then sees a silent connection, a different fault. Thus the proxy stops
the data, waits until the peer acknowledged all bytes, 2 seconds at most, and closes with a
reset. The proxy also delays each byte with no limit of the data rate, so a far cluster does
not look slower than it is.
