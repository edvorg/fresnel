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

## A request carries its kind

The caller sets a `RequestKind` on each request: get, list page, watch, log stream, write or
upgrade. The kind selects the connection, the time limits and gzip. The prototype read the
kind from the text of the URL. A change of that text could send a watch to the request
channel with no error. A request with no kind is refused, so a new call site cannot forget it.

## A channel connects by itself

A channel connects at its start and again after each loss. In the prototype, a channel waited
for a request, and the first request came from a watch after its own backoff. After 7 resets
in one minute, the recovery took 7.8 seconds. Now a loss counts one time, in the channel, and
a connection that was live for 2 seconds connects again with no pause. Eight resets each
recover in about 20 ms.

## Live means that a request got an answer

A channel is live only after the answer to a request, most often its own light request. An
API server that starts can accept a connection and close it. In the prototype, the state then
showed live 4 times for 10 ms each. The light request also proves the token after a second
HTTP 401, keeps an idle connection open, and tests a connection after a wake.

## No request before a token

A kube-apiserver ends an HTTP/2 connection after its answer to a request with no credential.
Thus a channel of a cluster with a credential plugin sends no light request while the cluster
has no token. It keeps its connection open in the state "waits for a token", and the ping
still finds a silent connection. In milestone M0, a plugin that hung for 20 seconds cost 43
connections in place of 3.

## The hedge delay follows the connect time

The hedged connect covers TCP and TLS, because a held connection can stop before the first
byte of the server. The delay is 1.5 times the typical connect time of the cluster, between
150 ms and 1 second. A fixed 300 ms started two attempts for each connect to a server with a
round trip of 250 ms. A cluster with no measured time gets 200 ms and 5 attempts.

## A proof before the state "refused"

After a second HTTP 401, one small request to a path that each user can read proves the
token. If it passes, the HTTP 401 goes to its caller and nothing else changes. In the
prototype, one path with a broken trust setup stopped all requests of the cluster for 60
seconds. A proof counts for 30 seconds, so a path that refuses each token costs one small
request each 30 seconds.

## A request waits 2 seconds for the plugin

A request waits 2 seconds at most for a plugin run. Then it fails with the credential state
as its error. The plugin has a limit of 20 seconds, and a read has 10. Without the short
wait, each request would fail with its own time limit, and the cause would not show.

## The text of the plugin stays out of each log

The plugin and the login shell write text for the user, and that text can hold a secret.
`Display` and `Debug` of their errors show the failure type only. The text is in `detail()`,
which the UI reads. Thus a log line or a journal entry cannot get the text by mistake.
`fresnel-net` writes no log.

## A warm upgrade connection gets a test

An idle HTTP/1.1 connection has no ping. In the prototype, two exec calls on silent warm
connections each failed after 10 seconds. Now a warm connection answers a light request
before its use, with a limit of 1 second. One failed test drops the full warm set, and the
request uses a new connection. A wake and a network change also drop the set.

## The login shell starts from the process environment

The login shell runs one time, with the environment of the process as its start. Its profile
then decides which values stay, as in a terminal. The resolve returns when the dump is
complete, and it kills the process group: a profile can end slowly or leave a process. Fixed
folders such as `/opt/homebrew/bin` always go to the end of `PATH`.

## One spawn of a child process at a time

On macOS, a new pipe gets its close-on-exec flag in a second step. A child that some other
thread starts between the two steps keeps that pipe open, and a spawn waits for the end of one
such pipe. In milestone M0, a test stayed 600 seconds in its spawn for this reason. Thus the
process module holds one lock for each spawn, and plugins that start at the same moment start
one after the other.

## hyper runs inside the task of the channel

hyper gives the socket loop and the future of each request to an executor. The executor of
`fresnel-net` hands them to the task of the channel. Thus no task starts outside the start
function, and the drop of a connection ends each of its streams. After GOAWAY, the task keeps
the old connection until its open streams end.

## The stream budget fits into the windows

The windows are fixed: an adaptive window lets one body that nobody reads stop each other
stream. The stream budget is the smaller of 70 percent of the server limit and one less than
the count of stream windows that fit into the connection window. The test of a connection
needs that room. In the prototype, 70 unread bodies of 1 MiB stopped a channel with a window
of 64 MiB, and its state stayed live.

## An own reader for the kubeconfig files

The kubeconfig loader has its own types for the file format and reads YAML with `serde-saphyr`.
It does not use the loader of kube, so a kube upgrade cannot change which clusters and
credentials the app sees. The app controls each rule. An entry with no body stays in the list
with its error. A name in two files gives a problem line. The error of a broken file has
only the position: the file can hold a key.

## KUBECONFIG and the default file are one list

`kubectl` reads `~/.kube/config` only when `KUBECONFIG` is not set. The app reads the files of
`KUBECONFIG` and then `~/.kube/config` as one list, in which the first file wins. A desktop app
shows all clusters of the user, and a project file in `KUBECONFIG` must not hide the other
clusters. The value comes from the login shell, because a program that starts from Finder has
no `KUBECONFIG`.

## An entry that the app does not support is an error

A kubeconfig user with a static token, a password, an auth provider or a user to act as shows
as not usable. The same holds for a cluster with a proxy, with no certificate authority or
with no check of the server certificate. A connect with less than the entry asks for would
act as a different user or trust a different server. The error names the entry, so the cause
shows in the cluster list.

## The gateway reads each answer itself

The gateway uses the kube client for the send and for the WebSocket handshake only. It checks
the status and reads the JSON itself. The request functions of kube write the body of a bad
answer to the log, and that body can be a Secret. The watch function of kube gives a bad
status as the first event, so a watch would show as live too early. A gateway error for
bad JSON gives the position only.

## The kube client keeps one task of its own

kube keeps its service in a `tower` buffer, and the buffer starts one task with a direct
`tokio::spawn`. It is the one task outside the start function. It only hands each request to
the service of the gateway. That service is always ready, and its call only builds a future,
so the task ends only when the last handle of the client drops. A request runs in the task
of its caller, where the time limits of its kind apply.

## The fate of a failed write is on the safe side

A failed write tells its fate: not sent, refused or not known. The app can send again only
after "not sent". A refusal is an answer with a status below 500. A status of 500 or more
counts as not known, because a server can fail after it stored the object. A network error
and a timeout also count as not known: the connection layer does not tell yet if the request
left.

## One connect of a cluster at a time

The registry runs one connect at a time, and a second caller gets the connections of the
first. In a test, 8 threads that connected one cluster at the same moment started it 8 times:
up to 24 connections in place of 3. Each start runs the plugin of the user, and a login plugin
can open a browser window. A connect has its own lock, so a reader of the states never waits
for a connect.

## Each read of the gateway has a size limit

The gateway holds at most 1 GiB of one answer, 32 MiB of one watch event and 1 MiB of an
error body. More is an error. A list of 10,000 pods in one page has about 300 MiB, so a good
answer is far below the limit. Without a limit, one broken answer could use all the memory of
the app.

## The doctor measures with the code of the app

The `doctor` command uses the login shell resolve, the kubeconfig loader, the hedged connect,
the channels and the gateway of the app, so its numbers hold for the app. Each connect sample
closes after the TLS handshake, and the only request is `GET /version`. A run on a real
cluster thus reads no object. For the idle close time, the channels for log streams send no
light request. The text of a plugin shows at the terminal only.

## The doctor counts an idle close from the start of its wait

A channel can be on a later connection when it is live, for example after a reset during
the start. That loss is not the close of an idle connection. Thus the doctor compares with
the connections at the start of its idle wait. With no token, it does not wait for the
channels: a channel sends no request then and is never live. With no HTTP/2 connection, it
takes no sample and starts no session.

## A view is a function of a copy of the state

A view gets a copy of the state of the core and returns the commands of the user. It holds no
handle to the core, so it can start no network work and read no file. A test builds the copy
of each state by hand and needs no network and no window. A command runs in the task of the
core, because a connect reads certificate files.

## The core starts after the window is open

`Core::new` starts no task, reads no file and opens no connection. The window calls
`Core::start` when it exists, and gives the function that asks for a repaint. Thus a slow
login shell or a network with no answer cannot delay the first frame. Until the list of
clusters is complete, the window tells how far the start is.

## A cluster has one state

The status bar and the journal show one state for each cluster: off, failed, connecting, live,
no connection or no credential. A failed credential wins, because the channels stay live only
for the rest of the life of the old token. A channel that waits for the first token of the
plugin counts as connecting: it has a connection, and the cluster cannot do work yet.

## The selection at the start is the current context

At its start, the app selects the current context of the kubeconfig files, as `kubectl` does.
The user selects each other cluster in the top bar. A connect runs the credential plugin of
the cluster, and a login plugin can open a browser window, so the app does not connect each
context of the files by itself. The selection is not saved yet. The settings file of a later
milestone saves it.

## The core reads files on the threads for blocking calls

The task of the core reads the kubeconfig files and the certificate files with
`spawn_blocking`. A read can wait for a slow disk. A thread of the runtime that waits holds
the tasks that it woke, and it can leave the timers and the sockets with no thread. Tests with
a pipe in place of a file prove that the window gets its wake and that a second cluster
connects during the read.

## The stop of the core is final

The stop can come while the task of the core holds the result of a read, and the task stops
only at its next wait. Thus the task reads the phase of the core under the lock of the state
before it gives contexts to the registry and before it keeps a monitor. A stopped core starts
no task again and drops each command.

## A reload keeps the clusters of a file that does not load

"Retry now" for a cluster whose connect did not start loads the kubeconfig files again. The
user edits a file at that time, so a syntax error is probable. A file that exists and does
not load keeps the clusters that it gave before, with their connections, and the journal
tells the problem. A file that is absent gives no cluster, as for `kubectl`.

## The journal writes its log line at once

The journal gives each entry to its sink in the task that made the entry, and the log file
writes the line at once. A writer task with a queue would lose the last lines at a crash, and
those lines tell the cause. The sink has its own lock, which also puts the entries of two
tasks into one sequence. The lock of the list in memory is a different one, so a frame never
waits for the disk.

## The panic hook takes no lock of the app

A panic writes a report file with the version, the thread, the place, the message and the
backtrace. The hook does not write to the journal: the thread that panics can hold the lock
of the journal, and the hook would wait for it with no end. One run writes 20 reports at
most, because a task that panics at each start would fill the disk. The next start tells
how many reports the log folder has.

## One process, by a lock of the system

Two processes of the app would write the same settings files and bind the same local ports.
The app takes a lock of the system on a file of its data folder and writes its process id
into the file. The system frees the lock when the process ends, also after a crash, so an old
file cannot stop a start. A debug build has its own folders and thus its own lock.

## The quit sequence runs in the exit call of eframe

After Cmd+Q, macOS ends the process, and the call that runs the window does not return. Thus
the quit sequence runs in `on_exit` of eframe, and one more time after the window call for
each other end. Only the first run does something. It stops the tasks of the core, closes
each connection, ends a plugin that runs, ends the runtime and syncs the log file.

## The activity against App Nap permits system sleep

macOS slows the timers of an app with a hidden window. A keepalive ping would then be late,
and a silent connection would show late. While a cluster is selected, the app holds an
activity of the type "user initiated, idle system sleep permitted". It keeps the timers
exact and does not keep a laptop awake. The wake detector handles the sleep of the system.

## The icon is drawn by code

The icon of the app is a Fresnel lens from the front: some rings on a dark tile. A function
of `xtask` draws it at each size, so the repository holds no image file and a change of the
icon is a change of some numbers. `iconutil` of macOS makes the icon file from the PNG files,
so the format of that file is not code of this repository.

## The install is a rename

`cargo xtask install` makes the bundle in a folder inside the destination folder, signs it
and moves it to its place with a rename. The destination thus holds the old bundle or the new
one, never a mix. The earlier bundle stays as `Fresnel.app.prev`. macOS does not show that
name as an app, and a rename back is the fallback. The signature is ad hoc: macOS on Apple
silicon starts only signed code.

## The test of the program starts itself as a child process

The window cannot run in a test. Thus the program is a library with a small `main`, and its
test runs the part before the window. The app must work with the environment of Finder, so
the test program starts itself again with `env -i`. The child reads its own process
environment, as `main` does. A flag of the app for a start with no window would be a
function that only tests use.

## A lint refuses a lock in the head of a branch

A lock guard in the head of `if let` or `match` lives to the end of the body. In the app
core, such a guard held the lock of the state across a read of the kubeconfig files, and a
frame of the UI would wait for that read. The tests did not see it. The lint
`significant_drop_in_scrutinee` now refuses this form in each crate, and the code copies the
value in a statement of its own.
