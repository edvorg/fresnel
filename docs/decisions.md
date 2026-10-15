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
that did not change. Thus `fresnel_net::task::Tasks::start` is the only way to start a task,
and `Tasks::start_thread` the only way to start a thread. Each one gives a name, a state, a
report for the journal and a restart with backoff. A lint refuses a direct `tokio::spawn` and
a direct `std::thread::spawn`.

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

## The fake server probes each idle connection

A client that closes with data on its way sends a reset, and the loopback of macOS can lose
it. The fake API server then held the connection with no end: 1 of 80 runs of a test that
stops the core during its first lists. So the server sets a TCP keepalive of 1 second on
each connection. The answer of the system to a probe for a closed socket is a reset, and
the connection ends.

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

## A long outage costs one connect attempt each 5 seconds

In a break of 10 minutes, one cluster made 2,917 connect attempts: each channel ran a hedged
race of up to 16 attempts after a pause of 3 s at most. Now, after one minute with no connect
that worked, the channels of a cluster take turns: one attempt for each turn, one turn each 4
to 6 s. A round of the signal and a channel that connects end each wait at once.

## A raise ends the waits of the connects one time in 10 seconds at most

The detectors can tell a change each 2 s, and "Retry now" can come often. Each raise ended the
wait of each channel and started its connect again as a race. In a long outage with 3 held
channels, a raise each 2 s gave 240 rounds and 720 races in 8 minutes. Now a raise starts a
round only 10 s after the last one: 48 rounds. The first raise after a calm time acts at once.

## A raise does not give the test of a connection a new limit

After a raise, a live channel tests its connection with a limit of 3 s. Each raise started a new
test, so a raise each 2 s kept a silent connection open until the ping ended it, 15 s after its
last frame. Now a light request in progress that ends inside the limit of a test is the test,
and a silent connection ends 3 s after the first raise.

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

## An HTTP 401 to a light request goes to the credential

A kube-apiserver also ends an HTTP/2 connection after HTTP 401 to a token that it refuses.
With no request of a caller, only the light request can tell. On k3s, a refused token cost
51 connections in 30 seconds, and the credential showed as valid. Thus a channel reports
this HTTP 401 to the credential task and ends the connection in the clean way, so open
streams continue. The credential then shows its failure, and no channel sends a request.

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

## A child process starts with posix_spawn, not with a fork

A fork runs the fork handlers of the system libraries in the child. On macOS, the handler of
libnotify killed the child when an other thread made its first call into libnotify at that
moment. `posix_spawn` runs no code of the app in the child. Its flags give the new session
and, on macOS, only the three descriptors of the child, so no pipe of one start reaches the
child of an other start (M0: a spawn waited 600 s).

## The kill of a process group repeats until the group is empty

A process that a member of a group starts at the moment of a kill is not in the group yet,
and the kill does not reach it. The command of the exit hook of a login shell stayed for this
reason after 2 of 12 test runs. Thus the process module kills the group again after it
collected the first process, until the system reports a group with no process.

## A child that drops before its end is collected at once, and its group ends

The drop of a run kills its process group. Before, only a later child collected the killed
child, so the last plugin of a stopped core stayed a zombie. Now a thread of the blocking pool
waits for its end and then repeats the kill of the group, as a stop does: one kill missed a
new process in 70 to 128 of 320 test drops. It is not a task of `Tasks`: a drop has none.

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

## The selection at the start is the selection of the last run

The core writes the selected clusters to `state.json` after each change by the user, and
selects them again at its start. With no state file, it selects the current context, as
`kubectl` does. It never connects each context by itself: a connect runs the credential
plugin, and a login plugin can open a browser window. A saved cluster that no kubeconfig
file has stays in the file, because a kubeconfig file can be absent for one run.

## The app does not write over a state file that it cannot read

A state file with a different version number, with a key that this version does not know or
with broken JSON can be the file of a newer version of the app. The journal tells the problem
with its line and column. The app starts as with no file and does not save its state in that
run. Each write goes to a new file that then gets the name of the state file.

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

## The install keeps the folder of the app

`cargo xtask install` makes and signs the bundle in a folder inside the destination folder.
One rename moves the `Contents` of `Fresnel.app` into a new `Fresnel.app.prev`, and one more
moves the new `Contents` into `Fresnel.app`. The Dock finds an app by the identity of its
folder: after an install that renamed `Fresnel.app`, the Dock started the earlier version.
macOS does not show `.prev` as an app. The signature is ad hoc: Apple silicon starts only
signed code.

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

## One list of the kinds, with a typed identity

`kinds/list.rs` has one line for each kind: its identity, its tier and its object type. A
`Kind` is a value of an enum, so no code compares the text of a kind. The line gives the API
path from the constants of the object type. The sequence of the list is the priority of a
kind for its first list. Each later part of a kind, such as its columns, has this one place
to grow from.

## Tier B stores ask the server for the metadata form

The store of a tier B kind asks for `PartialObjectMetadata`, so the data of a Secret never
arrives in the app. `kubectl apply` puts a copy of the object into an annotation, and the
server sends annotations also with the metadata form: 61 values in the cluster of the
prototype. Thus each store removes that annotation and `managedFields` from each object.
`managedFields` were 56 percent of a pod in memory.

## A store follows the state of its channel

One task for each cluster publishes a small copy of the channel states. A store sends no
request into a connection that is gone, counts no failure for it, and starts again at the
moment the channel is live. It is not live while no channel for watches is live. In the
prototype, the last store was live 26 seconds after the network returned. On k3s, each
store is live 0.1 seconds after the channel found a silent connection.

## A watch that worked starts again with no pause

A watch that gave an event or stayed open for one second did work. When it ends with an
error, the next watch starts at once: the usual cause is the loss of the connection, and the
channel counts that loss one time. Only a second failure with no answer between gets a
pause. The loop of the prototype counted each reset. After 7 resets in one minute, one
reset cost 17.7 seconds of old data.

## A store stays live while the renewal of its watch starts

The server ends each watch after 60 to 90 seconds, and the next one starts at once after
the last version. The store stays live in that time: with a state for each such end, each
view would mark its rows each minute with no fault. The start has a time limit of 10
seconds, and a silent connection shows through the channel. Each other start of a watch
is a state: after a list, a failure or a wait.

## A complete list or a watch that worked ends the count of failures

The count of failures ends at a complete list and at a watch that worked. One good page
does not end it: on the fake server, a list that failed at its second page each time started
again each 100 ms, with all its pages. The good status of a watch does not end it: a server
can end each stream at once. The pause grows to 3 seconds, with the jitter of each other task.

## Only the first list of a store has the priority of its kind

Four lists of a cluster run at the same time. The first list of each store gets its slot in
the sequence of the kinds, so the kinds that each view needs are ready first. Each later list
waits in the sequence of its request. With the priority of its kind for each list, eight
kinds whose lists got no answer held the four slots for all time on the fake server, and no
other kind got its first list.

## A second HTTP 410 gives a pause

After HTTP 410, only a new list gives the current objects, and the first one starts at once.
A good list does not end a sequence of HTTP 410: a server that gives a list and then HTTP 410
to each watch would get a new list with no pause, with no end. Only a watch that worked ends
the sequence. The same holds for the token of a list page that the server no longer has.

## The generation of a store changes only with its objects

A reader computes its rows again only for a new generation. Thus a state, a bookmark, an
event with a known version and a new list with no change keep the generation. Two copies of
an object are the same when UID and resource version are equal. A new list keeps the
allocation of each object with no change. In the prototype, each new list was a new
generation, so each view computed all again after each reconnect.

## A pending change is a comparison, not a mark in the store

After a write, the object shows "change pending" until its watch event arrives. The answer
to the write gives the UID and the resource version of the object, and
`Snapshot::is_pending` compares them with the object of a snapshot. A mark in the store
would need a second writer, and a mark that no event removes would stay for all time. The
versions of a kube-apiserver are numbers. For a version that is not a number, the answer
is "not pending".

## Tier A stores keep the full object

The first version keeps the full typed object of each tier A kind: 30.8 KB for one pod in
the prototype, and about 1.8 GiB for four clusters with 10,000 pods each. The slim pod
type of the prototype has 5.4 KB. A store takes each object type with the trait `Object`,
so a slim type can replace the full one for a large cluster with no change of the watch
loop.

## Readers hold a published state as a copy of one pointer

`published::Publisher` gives a state to its readers. A read copies one pointer and never
waits for the task. The last holder of an old snapshot frees it: 22 ms for 10,000 pods that
changed, more than one frame. Thus a reader gets a `Held` value. Its drop moves its one
pointer into a bin, which one worker thread empties. With a second pointer for the bin, the
reader could be the last holder. The bin has a limit.

## A fault of a store task shows as a state of the store

A store whose task ended showed "live" in the prototype, with objects that did not change.
The task of a store catches a panic of its loop, publishes the state "internal error" with
the message, and lets the panic go on to the start function, which writes the report and
starts the task again. The new run keeps the objects that the readers see and starts with a
list, because the run before can have lost an event.

## The state of a refusal stays until an answer

A store that the server refused with HTTP 403 or HTTP 404 asks again with a list, each 60
seconds or each 10 minutes, and at the signal of the cluster. The state "forbidden" or "not
supported" stays during such a try, so a view does not show each attempt. A watch that the
server refuses after a good list also gets a new list: the objects of the store are old then.

## The store of the Events has a maximum count

Events were 32,000 of 47,000 objects in the cluster of the prototype. A setting selects all
Events, warnings only or no store, and the store keeps 20,000 at most. The oldest Event by
its creation time leaves first, and the snapshot counts each object that left. For warnings
only, the server selects with a field selector, so the other Events never arrive.

## The loop reads from a source that a test can script

The watch loop knows two requests, a list page and a watch, behind the trait `Source`. The
app gives a source that sends them through the gateway. A test gives a source with a script
and uses paused time, so it sets each answer of the server and asserts each time exactly:
the stall limit, each pause and the batch of 30 ms. The connection states come the same way,
as a value that a test sets.

## The fault proxy drops a held connection that its client closed

In a freeze, a new connection gets no answer, and the app closes each connect attempt at
its time limit. On a real network, nothing of such an attempt reaches the server. Without
this rule, the end of a break of 10 minutes gave the server about 2,900 connections at one
moment, each one with a client that was gone. The time to recover after the break would then
be a property of the proxy.

## The breaks of the acceptance run at the same time

The acceptance asks for breaks of 30 seconds, 2 minutes and 10 minutes. The test starts
three apps before one k3s cluster, each one behind a fault proxy of its own, and runs the
three breaks at the same time. Thus the test needs 11 minutes and not 13. Each app has its
own connections and stores, so no break changes the result of the other ones.

## The k3s helper starts a new container when k3s ends at its start

On a CI runner, k3s sometimes ends some seconds after its start, when a second cluster
starts at the same moment: its kubelet finds the cgroup `kubepods` with no controller. A new
container starts well. Thus the helper makes a new container for a k3s that ended by
itself, 3 times at most, and each such end is a line of the test output. A restart of a
cluster gets no second attempt: the cluster has data.

## An index follows its stores by the differences of two snapshots

The task of an index compares the last snapshot that it read with the current one, and
applies each difference. A reader can miss an event, and a difference of two snapshots
cannot lose a change. Its cost follows the count of the changes: the map shares each part
with no change, and two entries are equal when they are one allocation. A new status of a
pod changes no relation, so it gives no new value of the index.

## The first relations of an index after a restart get a new number

The task of an index that starts again builds each relation from nothing. It cannot compare
them with the relations of before the fault: an object that left a store in the pause gives
no difference. Thus the first value of that run always has a new generation, and a view
that computes again only for a new number never keeps an object that is gone.

## Each answer of an index tells its basis

The prototype gave "no pods" before the pod store had its first list. Thus a snapshot tells
when its store had a complete list, and each answer of an index carries a basis: current,
old, not loaded or not followed. The basis reads only the stores behind that answer. A user
who cannot list Jobs still gets a good answer for the pods of a Deployment.

## The walk below an owner visits each object one time

The API server takes two Jobs that name each other as their owner. In the prototype, the
walk from an owner to its pods never ended for them, and a view that asked would stop the
app. The walk keeps the set of the UIDs that it visited. The scene of the cluster test has
such a pair of Jobs, so the fixture of the unit tests has it too.

## The Pod status text is the rule of the API server

`kubectl get pods` prints a table that the API server makes. The app ports that function,
`printPod` of Kubernetes 1.33, step by step and in its sequence, so a later version is easy
to compare. A unit test holds each pod of the fixture equal to the recorded output of
kubectl. A cluster test compares the app with kubectl and with the table of the API, for
one resource version of each pod.

## A health value reads the object alone

The health value of an object has no clock and reads no other object. A row of a table
then changes its value only with its own object, and a unit test needs one object. The
price: a value cannot tell that a state takes too long. An object on its way is
"progressing" until the object itself tells a fault, for example the deadline of a rollout.
The pods of a workload carry the other faults.

## The fixture of the unit tests is a copy of a scene on k3s

No object of a real cluster goes into the repository, and an object written by hand has
only the fields that its writer knows. Thus one cluster test creates a scene on k3s with
one object for each relation and each state, waits until it is at rest, and can write each
tier A object and the output of kubectl to `fixtures/k3s`. The unit tests of the indexes,
the Pod status text and the health values read these files.

## The Event index finds an Event with no UID by the name

An Event names its object with a UID, and an other object with the same name has an other
UID. The kubelet writes the name of its node in the place of the UID. Thus the index has
two relations: by the UID, and by kind and name for an Event with no UID of the API. An
Event of an earlier pod with the same name then stays out of the answer for the new pod.

## The reference index needs no store of ConfigMaps and Secrets

A pod names its ConfigMaps, its Secrets, its ServiceAccount and its Node by name. The index
maps that name to the pods, so "used by" needs only the store of the pods, and no store
holds a value of a Secret. A reference to an object that does not exist is in the index
too: a missing ConfigMap is the reason why a pod cannot start.

## The kubelet of the test cluster has no limit for image pulls

A kubelet refuses each image pull above 5 in one second. A scene that starts 20 pods at
once then had some pods in `ErrImagePull`, a different set in each run, and the pods of k3s
itself too. The k3s helper sets `registry-qps=0`, so the state of a scene depends on its
manifest only.

## A list ends when a page names the token of its own request

A server that gives the token of a request back as the next token would get that request
again with no pause and with no end. The page type has the one function that takes the next
token and refuses this one. The list of a store and the read of the Events of one object
use it, so the rule holds for each list in pages.

## The core runs the stores of each connected cluster

The app core starts the stores and the indexes of a cluster with its connections and stops
them with the disconnect. Its view gives the Pod snapshot and the Event index as copies of
one pointer. Stores that do not start are a connect that failed: a connected cluster with
no stores would show an empty list in place of an error.

## New data does not stop the computation of rows

In the UI prototype, each new snapshot stopped the sort in progress. With snapshots faster
than the sort, the table got no rows at all. Here the worker thread ends each computation,
and its mailbox holds the newest request only. A new sort or filter stops a computation,
because its rows have no use. Each result carries the query and the data number of its
request, and the table drops a result of an older query.

## A request for rows stays in the mailbox until its rows are there

In the UI prototype, a worker thread that ended left a table that showed "updating" with no
end. Here the thread is a task of the start function. After a panic, the journal has a
report, the thread starts again and finds the request of the panic in the mailbox. The table
reads the state of the task and shows the reason next to the rows from before the fault.

## The selection holds objects, and a difference of snapshots removes them

The cursor and the selection of a table hold the identity of an object, never a row
number. The Pods view compares the snapshot of each frame with the one before, as an
index does: the cost follows the count of the changes. A pod that left its store leaves the
selection, and a new pod with the name of an old one is an other object.

## The identity of an object has its name and its UID

An `ObjectId` is the cluster, the namespace with the name, and the UID. The UID tells a new
object with the name of an old one from the old one. The name is a part too: with the UID
alone, two pods that a server gave no UID were one object, and a click on one of them
opened the panel of the other one, as a deleted pod.

## A click on older rows selects no object that went away

The rows on the screen are older than the stores while the worker computes, and the view
removes a deleted pod from the selection one time, at its change. A click on the old row of
such a pod selected it again, with no end. So a table keeps the objects of its last
request. After a click or a key on older rows, it asks its model for each selected row if
the objects still have it.

## A view that is not painted holds no rows

While the window showed some other page, the Pods view kept its rows and the snapshots of
its last frame, so old pods stayed in memory for that time. The shell now tells the view
that it is not painted. The view gives its rows to its worker thread, empties the mailbox and
drops its snapshots. Its selection and its panel go too: no frame would remove a pod that
goes away. A hidden window is no such case yet.

## The freshness mark comes from the state of the store at the paint

A watch that stops changes no object, so the worker computes no rows for it. Thus the mark
of a row is not a part of the row: each frame reads the state of the store of each cluster,
and the next frame after a fault shows the mark and the note. A row with the mark has no
state color, because its state is not a state of now.

## The detail panel reads its pod in each frame

In the UI prototype, the panel built its data one time and showed a deleted object with each
button. Here the panel finds its pod in the snapshot of each frame. A pod that a store with
a complete list does not have shows as deleted, with its last state. A store with no complete
list does not know its pods, so the panel says "not loaded" then.

## A view cuts a text of a cluster before its layout

The API gives most texts of an object no limit of their length. Measured in a release
build: a status reason of 1 MB cost a frame of 63 ms each time its row came into view, and
2,000 annotations cost two frames of 20 ms. So a cell lays out 400 characters at most and
the panel 240 of one value, with a mark. The panel draws the first 100 labels and
annotations and counts the rest.

## The namespace filter lists the namespaces of the pods

The filter of milestone M1 is in the bar of the Pods view, and its menu has each namespace
with a pod: the worker collects them with the rows. So the filter needs no store of the
Namespaces. A namespace of the filter stays in the menu when its last pod goes away. The
filter moves to the top bar with the tables of the other kinds.

## The limit of a frame time holds in a release build

A debug build runs the steps of the time tests and checks no time. CI runs them in a release
build, one at a time. The limits 16 ms and 50 ms hold on the Mac on which they were set. A
shared runner is slower and less steady, so each runner has limits of about twice its longest
time. A frame time that grows with the size of an object is a defect, not a slow runner.

## The dock is a panel until the user can move a tab

Milestone M1 has two tabs below the center. A panel of egui with a tab bar draws them, and
the user changes its height with the pointer. `egui_tiles` comes with the tabs that the user
can move to some other place. Then `shell/dock.rs` wraps it, and no view changes.

## A filler after the last column takes the free width

In M1 the Name column took the free width. A drag that made some other column wider took
the same width from Name: the edge stayed, the columns moved the other way, and each frame
of the drag added more width. Now no column of a model takes the free width: a drag changes
one column, and its edge stays at the pointer. A filler without a title takes the rest, and
a narrow table scrolls to the side.

## The filler holds the place of a scrolled table

At the right end of a narrow table, a narrower column made the content narrower than the
place on the screen. The scroll area then moved each column to the right, the edge stayed
away from the pointer, and each frame took more width. Now the filler keeps the width of
the place that the last frame showed. It never makes the content wider than before, so a
scroll to the right cannot add width without an end.

## The state file keeps the widths and the columns that the user gave to a table

Version 3 of `state.json` has the layout of each table that the user changed: the width of
each column that differs from the width that its table gives, in whole points, and each
column that the user showed or hid, by the title of the column. A title that the table does
not have changes nothing. The app reads the files of versions 1 and 2 too.

## A table saves its widths after a drag and reads the saved ones one time

A drag changes a width in each frame. The table sends `SaveTable` when no pointer button is
down, so the core writes the file one time for each drag. The table takes the saved layout one
time, after the core read the file. A later layout of the core only repeats the commands of
the table, and a table that took it could undo a drag in progress.

## The first column stays inside the table

The first column does not scroll to the side. A saved width, a drag or a narrower window
could make it wider than the table: then its edge and each other column were off the
screen, and the user could not change that. Now a frame shows the first column inside the
table, with room for the other columns, and a drag of its edge stops there. The width of
the user stays: a wider window shows it again.

## A double click on an edge fits the column to its cells

After a double click on an edge, `egui_table` lays out the cells of that column on the
screen in a pass that sizes only, in a place of the narrowest width. The table takes the
widest title or cell of that pass as the width. The cells cut their text to their place, so
the column got the narrowest width and the state file kept it. In that pass, a title and a
cell now give their full width.

## The table owns the handle of each edge

The handle of `egui_table` took clicks and drags, so egui started its drag after a move of
6 points: until then the edge stayed. It also changed a width after its layout, one frame
late. Now the table puts a handle that takes only drags on each edge, above the titles and
rows. Before the layout, the width is the width at the press plus the move of the pointer.
A click changes nothing, and Escape gives back the width.

## A handle ends at the scroll bars of the rows

The handles went down to the bottom of the table, over the scroll bar below the rows. In a
table wider than its place, a drag of that scroll bar at an edge changed the width of the
column, and the table did not scroll. A handle now ends at the scroll bars of the rows.

## A click on the edge of a cut first column keeps its width

A narrow window shows less of the first column. A drag starts from the width on the screen,
so a click or Escape gave that cut width to the column, and the state file kept it. A move
to the right cannot make that column wider on the screen. Now a click, Escape and a move to
the right keep the width of the table.

## The first widths come from the table, not from the cells

A column has the width that its table gives until the user changes it. A width from the
cells of the first frame would depend on the cluster whose rows came first: the layout would
differ from run to run, and the state file could not tell a width of the user from a
measured one. A double click on an edge fits a column to its cells when the user wants it.

## A test of a drag moves as a mouse does

The first drag tests moved the pointer 6 points or more in each frame, at a scale of 1. A
mouse moves one point or less in a frame, on the pixel grid of a Retina display, and a
fast one moves far. The tests in `pods/mouse.rs` do each of these in a table of 1,750
points at a scale of 1 and of 2. They check the edge in the frame of each move.

## A kind is a trait of its object type

Each kind implements `Describe` for its type of `k8s-openapi`, in a file of its own. A
column is a function pointer from the object to a `Value`, so a view of each kind is one
generic table, and the compiler checks each field that a column reads. `Kind::visit` turns
the identity into the type. A const check in the list refuses a type that names an other
kind.

## A column has the text of kubectl, and a test compares it with the server

A user knows the columns of `kubectl get`, and the API server prints them in its table
form. A column that kubectl shows too has its text. The cluster test of the kinds reads
that table with each object for each kind, and compares the two for each cell. A copy of
those tables is the fixture of a unit test, so `cargo test` checks the columns too.

## A store of full objects starts on demand and stays 60 seconds

A tier B kind has a store of metadata for the search and the indexes. A table of it needs
the full objects, and a tier C kind has no store. Each such view holds a `Demand`; the
first holder starts the store, and the store stops 60 seconds after the last one. A view
that the user opens again soon then shows current rows with no new list.

## The manifest is a read of the server, not the copy of a store

A store removes `managedFields`, the copy of `kubectl apply` and the values of a Secret.
The manifest view needs the object as the server has it, so one GET through the gateway
reads it. A switch keeps `managedFields`, which most users never need. The YAML has the
keys in the sequence of the alphabet, as kubectl writes it.

## A cluster with no group of the API comes from the flag of k3s

The test of the state "not supported" needs a real server that does not serve a kind of
the app. k3s starts with `runtime-config` to turn off `autoscaling/v2`, as on a cluster
before Kubernetes 1.23. With `node.k8s.io` off, the API server never became ready: its own
informers read the RuntimeClasses.

## The view names its kinds, and the core holds their stores

A table of a tier B or C kind needs the full objects, and a view holds no handle to the core.
So the view sends `ShowKinds` with the kinds that it shows. The core holds a `Demand` of each
in each connected cluster, also in one that connects later, and the view reads the snapshots
in `ClusterData::objects`. The Pods need no command: each view has them.

## One task reads the manifest that the view asked for last

The detail panel shows one object, so a new request ends the read in progress. The task does
not end after a read: a job for each read would give a journal entry for each change of an
object. A read that fails shows in the panel with its reason. A request names the UID of the
object, and an answer with an other UID fails: a StatefulSet can make a new pod with the old
name during a read.

## A count of the sidebar that is not current has the mark of old rows

A store that is forbidden has no object, and a store in a pause keeps its old count. A plain
"0" or "25" would then look like the current count of each cluster (design rules 4 and 5). So
a count is plain only when each selected cluster has a live store with a complete list; any
other count has the open ring of the rows, and the page of the kind gives the reason.

## Each store asks for a repaint

The sidebar shows the count of objects of each kind, so each new snapshot of a store and each
new value of an index asks for a repaint. The notifier keeps the limit of 60 in one second.

## One page type for each kind, and the Pods keep their own model

The page of a kind (`views/objects`) has the follower of the stores, the notes, the bar, the
table and the detail panel. A kind gives a model: the Pods keep the model of milestone M1 with
the status of `kubectl get pods`; each other kind gets one generic model from the columns of its
description. So the weakness tests of each kind test one code, and a kind needs no view code.

## The worker computes the value of each column one time

A row of a kind keeps the value of each column, which the worker reads from the object. A sort
then compares values and reads no object, the text filter reads the texts of the row, and a
frame lays out the cells on the screen only. With 10,000 Deployments, the values cost memory
for each row; the frame time test holds the limit of 16 ms.

## A hidden sort column gives the sort back to the name

The arrow of the sort is in the title of its column. When the user hides that column, the
rows would keep a sequence that nothing on the screen tells. So the table sorts by the first
column again, which always shows.

## The shell holds the page of one kind

Each table has a worker thread and holds rows. The shell keeps the page of the open kind only:
a click on an other kind gives the rows of the old page to its worker and ends the thread. The
Connections page keeps the page and the store of the kind before it, so the way back is quick.

## The detail panel reads the conditions from the JSON of the status

Each kind has a condition type of its own. The panel writes the field `status` of its object
as JSON one time for each new version and reads `conditions`, so one section serves each kind.
The JSON of the whole object cost 9 ms in a frame for a ConfigMap of 70,000 keys, and 26 ms on
a CI runner. The section gives the status no color: `True` is good news for `Ready` and bad
news for `MemoryPressure`.

## A link of the panel opens the page of its kind and waits for the object there

A related object comes from an index as a kind and a name. The link opens the page of that kind
and waits until the store of the cluster has a complete list: then the panel opens, or a note
says that the cluster has no such object. A store of a tier B kind starts with the page.

## The sidebar is 196 points wide

The kinds have long names, such as HorizontalPodAutoscalers. At 148 points, half of them were
cut. The view tests run in a window that is 48 points wider than before, so the drags of the
column edges have the same places in the table.

## The quick search reads the stores of the app

The quick search sends no request: a worker thread reads the snapshot of each store of each
selected cluster. A tier B store holds the metadata, so a Secret is found by its name, labels
and owners, never by a value. A kind of tier C and a store that the server refuses are notes
with their reason, so no hit is never a proof of no object. 4 clusters with 60,000 objects
took 12 to 15 ms.

## The command table is in the core

The settings name a command and its key, and the app reports a bad name or a bad key with its
line and column. So the table, the shortcut type and the list of key names are in
`fresnel-core`, which has no GUI toolkit. Each key name is a name that egui reads, and a test of
`fresnel-ui` holds that rule.

## A shortcut needs Cmd, Ctrl or Alt

egui gives the text of a field as text events, and the key events of the same press come too.
A key with no modifier, or with Shift only, would act while the user types into a filter. So a
shortcut needs Cmd, Ctrl or Alt; a function key is the one exception. A shortcut with more
modifiers is read first, because Cmd+K of egui also takes Cmd+Shift+K.

## The settings file comes first, and only the settings view writes it

The core reads `config.toml` before the login shell, which can take seconds, so the keys of the
user act from the first frames. Only a save of the settings view writes the file. A parse error or
a newer version at the start leaves each setting at its default, with an error in the journal. An
unknown key is a warning, and the other keys stay in use.

## One namespace filter for each view

The namespace filter moved from the bar of the Pods table to the top bar, as plan section 11.1
has it, so each page shows the same namespaces. An object with no namespace passes each filter,
and the Nodes stay visible. The quick search ignores the filter. The menu lists the Namespaces
of the stores, the namespaces of the page and the namespaces of the filter.

## Back and Forward follow the object of the panel

Each object that the detail panel shows is a visit: by a click, a key, a menu, a link or a hit.
The same object again is no visit, and a visit after a step back drops the objects after it, as
in a browser. The history keeps 100 visits. When a filter hides the row of an object that a
link opened, a note offers to show it.

## The quick search holds snapshots only while it is open

A request to the worker of the quick search holds a snapshot of each store of each selected
cluster. The dialog empties the mailbox at its close, so a closed search holds no objects (plan
section 7.3). A new snapshot, a new state or a cluster that comes or goes gives a new request,
so a hit gets its mark when its store leaves the state live.

## A key view reads its one object while it is open

No store keeps a value of a Secret. The key view of a Secret or a ConfigMap reads its one object
with a GET, and the core reads it again when the metadata store, which is always on, shows a new
version. A watch of one object would need a second watch loop. The values live in the holder of
the view, a part of the connected cluster, so they go with the view and with the connections.

## A save of keys is a merge patch of the changed keys

The patch has each changed key and `metadata.resourceVersion`, never `stringData`. The server
leaves each other key as it is and refuses a save of an old read with HTTP 409. A change back to
the bytes of the read is no change. After a conflict, the view reads again, and the draft keeps
the changes of the user and names each key that both sides changed. The app does not send again.

## A value is text only when a text editor keeps each byte

A value is text when it is UTF-8 with no control character but tab, line feed and carriage
return. Each other value shows as its size and the type of its first bytes, because a text editor
would change bytes that the user does not see. A key of a ConfigMap stays in its map while its
value fits there; a value that is not UTF-8 goes to `binaryData`.

## A save in progress ends also when the view closes

The stop of a write in progress leaves its result unknown (design rule 6). So the holder of a key
view with a save that waits or runs stays until the save ends, and then drops its values. A
create of a Secret runs in one task of the core, one create after the other, for the same reason.

## The journal shows the end of a save that no view shows

A save can end after its view moved away, after a later save of the view, or when the
connections of its cluster close. No view shows that end, so the journal does, with the object
and no value (design rule 1). The same save again while it is on its way does not go: it would
come back as a conflict with the first one.

## The panel of a Secret hides each value until the user shows it

The panel reads the values when it opens, as Headlamp and Lens do, so Copy and the views of the
certificates and the registries work at once. A hidden value shows dots and has no editor. The
diff before a save hides the values too, and its summary still tells a change of the line feed at
the end. The values live in the panel and in the dialog, and go when they close.

## A draft moves onto each new read, and a save waits for each conflict

A new version can come while the user edits. A draft with no change takes its place. A draft
with changes moves onto it: a change that the server has now goes, and a key that both sides
changed shows both values. The save waits for a choice for each one, so it never writes over a
change that the user did not see, and it goes with the version of the new read.

## The save of keys asks with the diff of each key, and its bar is above the keys

The dialog names the cluster, namespace, kind, name and version (plan section 11.2) and shows
the lines that each changed key loses and gets. It holds its save, so the save that goes is the
one that the user saw. The bar with Save and the state of the last save is above the keys: a
long list of keys would hide it below the dock.

## The keys pass the gate of the panel and wait for a save

A change of the keys is the action "Edit keys" of the panel, so its gate decides it: the rules,
an access review when they cannot tell, and an object that left its store. The dialog of a save
reads that gate in each frame. While a save is on its way, the keys and Discard wait: an edit
then would meet the answer of the save as a change of the server, a conflict with the own save.

## A create of a Secret is pending from its click

The core takes a command a moment after the click, and later while it connects a cluster. A
second click in that time would send the create again, and the server would refuse it as
"already exists" for the Secret of the first one. So the dialog shows the create as pending
until the view has a newer job. Two rows with one key name stop the create: the Secret would
keep one value with no word of the server.

## New Secret is a command of the command table

Cmd+Shift+N, the quick search and a button of the Secrets page open one dialog over each page.
The cluster and the namespace are fields of the dialog, with the first connected cluster and the
one namespace of the filter as defaults, so a Secret goes to a cluster that the user sees. The
dialog shows the state of its create, and "Open it" opens the new Secret.

## Each operation is a job in a task of its own

An order runs in a task that runs one time, so a rollout status that waits does not hold the
other jobs, and each row of a bulk action gets its own state. The list keeps 500 jobs and counts
the ones that leave. A job whose task stops before its end gets the state "result not known" when
it can have sent a write, and "failed" when it only reads.

## A JSON patch goes as the request of a merge patch with its own content type

kube builds a JSON patch only with the `jsonpatch` feature and one more crate. The rollback of a
Deployment is the one JSON patch of the app, and its body is JSON already, so the request of a
merge patch with the content type `application/json-patch+json` is the same request.

## History and rollback work on the JSON of the server

A rollback copies a pod template back into a workload. A type of `k8s-openapi` drops each field
that it does not know, so a field of a newer server would leave the pods. The procedure reads the
workload and its ReplicaSets or ControllerRevisions as JSON and compares templates as JSON.

## Run again checks the new Job before it deletes the old one

The server checks the new manifest in a dry run with a name that it makes, because the old Job
holds the name. The store of the Jobs must follow them, and the manifest must have its text. Then
come the delete, a wait for its watch event and the create. After the delete, the outcome holds
the manifest for a copy. A Job whose labels copy its template gets none, so the server copies
the new ones.

## The rules of a namespace decide an action, and HTTP 403 drops them

One SelfSubjectRulesReview for each namespace that a view opens gives the rules, which stay 5
minutes. An action on the cluster and rules that are not complete are unknown: an access review
asks the server. A review with no status is a failure, not a denial. The rules are a hint, so a
job that the server refuses with HTTP 403 drops the rules of its namespace, and its state shows
the message of the server.

## The core keeps 256 answers of the permissions of each type

Each frame copies the permissions of each cluster, and a view can ask about each object that it
shows. Thus a cluster keeps the rules of 256 namespaces and 256 answers of access reviews. The
oldest answer leaves first, and a view that needs it asks again, so nothing is lost. A review on
its way stays: the queue of 64 reviews limits those.

## The same order does not run two times at once

A fast second click gives an order again while its job runs. A second "roll back to the revision
before" would undo the first one, and a second "run again" would race the first one for the name
of the Job. Thus that order ends at once as not sent, with the number of the job that runs.

## An action passes a gate before it runs

The header, the menus, the keys and the quick search ask `views/act` for each action. Its gate
closes it for a later milestone, for a state that kubectl refuses (a restart or a rollback of a
paused Deployment, a second suspend) and for a denial of the rules, each time with the reason.
A permission that the app does not know leaves the action on: the rules are a hint, the server
decides, and the core handles HTTP 403.

## A view asks for a permission one time until the core has it

A frame decides each action that it shows, so a question in each frame would fill the queue of
the core. The panel, the menus and the dialog each keep what they asked. A question goes again
when the core has no entry any more, for example after HTTP 403 dropped the rules, and a failed
read goes again one time for each new panel, not in a loop.

## The confirm dialog lists each target and decides it in each frame

The dialog names the cluster with its color, the namespace, the kind and the name of each
target, and tells the warnings of research 08. In each frame it reads each target from its
store and the rules: a target that goes away, that a new object of its name replaces, or that
the rules forbid shows its reason, and the button is off. So is a count of targets over the
free places of the job list.

## The bulk actions are a menu of the selection

A bar of its own for the selected rows would move the rows down when a click with Cmd selects
the second row, and the next click would hit an other row. The bar of the table has no room for
three buttons next to the filters while the panel is open. So the count "2 selected" is a menu
with scale, restart and delete, each one off when the rules forbid it for one of the rows.

## The revisions of the panel are the last job that read them

A ControllerRevision has no data in its store, and the ReplicaSets are full objects only on
their page. So the panel shows the outcome of the last "revision history" job of the object that
went out, with a rollback to each revision. A newer job that the core did not send, such as a
second click while a read runs, is a note. The Operations tab shows the same jobs, so the two
never disagree.

## "Copy kubectl command" lists each command of the object

One key copied only `get`. The dialog lists get, edit, the logs and the exec of each container,
and a port forward of each port, each with the context of its cluster. A remote port below 1024
gets the local port 8000 more, because a lower port needs root on the Mac.

## The page of the CronJobs holds the Jobs too

The panel of a CronJob shows the state of each of its Jobs. The Jobs are a kind of tier A, so
their store runs already; `ShowKinds` with both kinds gives the view their snapshot, and no
request goes. A job that the user starts opens the Operations tab, so each click has an answer.

## Own types and readers for the metrics API

`k8s-openapi` has no types of `metrics.k8s.io`. `k8s-metrics` 0.28 fails a whole list when one
container has no memory value, and `kube_quantity` 0.9 reads `5E` and `1Ei` wrong. So the core
has own types that accept a missing field, and own readers of a quantity and of a Go duration,
each with unit tests. An item that does not fit is a count with its reason, not a failed list.

## The history of the metrics stays in memory, with a limit for each cluster

The sampler keeps one hour of samples and 24 hours of minutes in memory. A limit for each
cluster, not one for all, keeps a large cluster from pushing the history of a small one out.
A history from before the start of the app needs Prometheus, a later option of the plan.

## A cluster with no metrics API gets a poll each 5 minutes

A 404 of both metrics lists means no metrics-server, which a user installs seldom; two reads each
15 s would give the same 404 again. The next poll then comes after 5 minutes, and "retry now", a
wake and a network change end that pause. After a 503, the sampler reads the APIService of the
metrics one time: its `Available` condition tells why the API server cannot reach metrics-server.

## The CPU and memory columns come after the age

The columns of the Pods fit next to the sidebar in a window of 1,328 points; the two columns with
a bar take 230 more. After the age, each other column keeps its place, and the column choice can
hide them. A table sorts by the values of one poll, so its rows hold them: each poll of a cluster
gives the table new rows, as a change of a store does.

## A chart shows a fixed range up to now, with no zoom

The range moves with the clock in each frame, so a zoom or a drag would hold for one frame only;
the choice of 5 minutes to 24 hours is the zoom. The charts of a panel share their time axis and
their cursor, and their value axes have one width, so their times line up.

## A line has a gap after 2.5 expected distances

A poll each 15 s can get the same scrape of metrics-server again, and the next poll then misses
one sample: a distance of two periods is no fault. A line stops where two samples are more than
2.5 distances of their tier apart: the period in the fine tier, one step in the coarse tier.

## A log stream asks for the times and reads raw bytes

Each stream asks for `timestamps=true`: the merge of many pods sorts by the time, and a new
stream starts from it. The view hides the time when the user wants. The stream gives bytes,
which become lines of 64 KiB at most, because a program can write with no line end; a reader of
lines would end the stream at the first byte that is not UTF-8.

## Escape codes become styles at ingest, with vte

The parser of `vte`, which the terminal of M10 needs too, gives each SGR code as a style. The
text of a line keeps no escape sequence, so a search reads plain text and a color code in a word
does not hide it. A style holds into the next line of the same source, as in a terminal. A
synchronized update would hold its bytes, so it ends with its line.

## A new stream starts at the full second of its latest line

kube rounds `sinceTime` to the nearest second, and the kubelet drops each older line: the time of
the latest line could lose half a second. The new stream thus sends the lines of that second
again. The view drops a line while its time is one of that second, as often as the view has it;
the first other line ends the drop. Each gap gets one marker line. On k3s, a reset of each
connection lost no line.

## A step back of the clock of a node starts the latest second again

The kubelet drops each line before `sinceTime`. A line more than a minute before the latest
second of its source tells that the clock of the node went back, and a new stream from that
second would lose each line until the clock reaches it again. The latest second thus starts again
at that line: the next stream shows some lines two times and loses none.

## A source waits for a change of its own pod

After a refusal, or while its container does not run, a source waits for its pause or for a change
of its pod in the store. The store of a busy cluster changes each 30 ms, and a wait for any change
would send a request at each one, with no pause.

## The pod tells what the end of a stream means

A stream ends when its container ends and when its connection closes. The source reads the pod in
its store: the instance that the kubelet serves by its own rule, and the restart policy. A new
instance gets a marker and a stream, an instance that still runs a new stream after a pause, and
a container with no log a wait with its reason and no request. A container that does not start
again ends its source.

## A stream with only a text of the kubelet is a refusal

On k3s, the status of a pod in a loop of crashes names for some seconds an instance that the
runtime removed. The kubelet then answers with HTTP 200 and "unable to retrieve container logs",
with no time. A stream with only lines with no time is thus a refusal: the source waits for a
change of the pod with that text as its reason. A line with no time between real lines is a
marker.

## The lines of many pods merge by their times with a hold of 250 ms

A line goes into the view when each open source has a line, after a wait of 250 ms, or when the
waiting lines hold an eighth of the bytes of the ring. A slow source holds the view 250 ms at
most, and a fast one cannot fill the memory. A later line of the slow one goes to the end with
the mark "late": each number of a line stays for a scroll position and a selection.

## The ring of a view is a vector of imbl

A view gets the lines of a moment as a copy of a pointer, and a push changes few nodes, so the
core publishes the view each 30 ms and the UI holds no lock. The ring keeps 200,000 lines and
128 MiB, and it counts the lines that left. On k3s, 1,000,000 lines of eight pods added 27 MB to
the heap at most.

## A view has 50 streams, and its default container is the one that restarts

kubectl follows 5 pods of a selector; a workload of the app can have more, and each stream is one
stream of the log channel. A container over the limit is a count in the view, and a stream that
ends frees its place. Without a choice of the user, the view reads the container that restarts
most in its pods, else the one of kubectl.

## A view that starts again after a panic has no line, and a count tells it

The sources of a view stop with the run of its task, so a new run cannot continue their lines.
It starts with an empty ring and new sources, and `LogView::restarts` counts such runs, so the
view can tell that the earlier lines are gone. A source keeps its lines after a panic of its own
task: it starts from its latest line with a marker.

## The log sessions belong to their connected cluster

`ShowLogs` names each view that the UI shows. The cluster of each view runs its session, keeps it
while the view stays, and stops it with the view or with its connections. The same request keeps
its lines; a change of the options is a new request with a new session.

## The test of a million lines counts the heap with eight pods

The test binary counts each allocation. The kubelet rotates a log above 10 MiB, and a stream that
follows can stop at a rotation, so eight pods write 125,000 lines each, some 6 MB each.

## An HTTP 429 with no Retry-After refuses one request and throttles nothing

The API server sets `Retry-After` on each throttle. An eviction that a PodDisruptionBudget blocks
gets HTTP 429 with no header and a cause that names the budget. As a throttle it went out four
times and held each request of its cluster for a second. Now the drain gets it at once, shows the
budget by its name and sends the eviction again after 5 s, as kubectl does.

## A drain follows kubectl drain, and a pod that blocks stops it before an eviction

The job cordons the node, lists its pods, reads their DaemonSets and gives each pod the verdict of
the filters of kubectl. A pod that blocks fails the job; the dialog shows the same verdicts first
and keeps its button off. A cancel or the time limit stops the drain, and the node stays cordoned
after each end, as kubectl leaves it: the Operations tab says so, and the user uncordons it.

## Each pod of a drain has its own task, and the pod store tells its end

As kubectl, the job has a task for each pod, in a `JoinSet`: a pod that a budget holds waits
with no effect on the others, and a panic fails that pod only. The wait for its end reads the pod
store. The eviction keeps the UID of the listed pod, so a new pod of its name answers HTTP 409:
the pod is gone when the store does not show it, else the conflict is a refusal for now.

## A drain evicts no pod when its list has a pod of an other node

The server selects the pods of the node by a field selector. A server or a proxy that drops it
gives each pod of the cluster, and the drain would evict them all, so the job checks the node of
each pod first. After the cordon, no end of a drain is "not sent". A pod whose eviction the server
took stays "terminating" after a cancel: it goes by itself.

## The log pane has its own rows and its own scroll position

The scroll area of egui needs the height of each row and adds them in an `f32`, which is not
exact past some 400,000 rows, and its label selection knows only the rows on the screen. The
pane keeps a line and a row of it as its position, lays out the rows on the screen only, and has
its own selection by line and byte. A frame with 200,000 lines took 1 ms or less.

## A row of the pane is a node of the accessibility tree

The pane paints its text itself, so a screen reader would read nothing there. Each row on the
screen gets a node with the role of a list item and the text of the row, with its place: egui
gives a place only to a widget that takes the focus. The view tests read the rows in that way.

## A save of a log goes to the folder of downloads, with no dialog

A dialog of the system needs one more crate and a window, which no test can drive. The core
writes each save into `~/Downloads` as a new file with the cluster, the object, the part and the
time in its name, and never over a file that exists. The tab shows the path, the size and a
cancel, which removes the part that was written.

## A download is a job that writes a new file, and a Secret asks first

The Download action of plan section 10 reads the manifest from the server, as the Manifest tab
does, with no `managedFields`, and writes it as a new YAML file in the folder of downloads with no
dialog, as a log save does. It is a job, so the Operations tab shows its file or its error. The
file of a Secret has the value of each key in base64, so that download asks first and tells it.

## A save that does not end removes its file

A part of a log would look like a full one. A cancel, a failed write, a panic and a stop of the
cluster or of the app remove the file of a save; the drop of its task does the last two. A save
holds a stream, so four saves of a cluster write at a time, and a fifth one fails at once. Each
part of a file name has a limit: the name fits in 255 bytes.

## The search of a log ignores the case until the text has a capital letter

One field and no switch: a text in small letters ignores the case of ASCII letters, and a text
with a capital letter keeps it, as `less` and ripgrep do. The search and the filter read the text
with no escape sequence on the worker thread of the tab, in parts of 20,000 lines, so the count
grows while a frame stays short.

## The dock with a log view has a height of its own

A log needs rows, and the tab has a bar of two rows and the state of each source. The dock with a
log tab is a second panel with its own default height of 380 points, which keeps the height that
the user gives it. The other tabs keep the dock of 170 points.

## The overviews count on a worker thread, in the namespace filter of the tables

A count reads each object of a kind: four clusters with 10,000 pods each would hold a frame. The
page gives the snapshots of each cluster to a worker and draws its last counts; the state of each
store comes from the frame, so a count of a store that is not live has its mark. The counts follow
the namespace filter, so a link to a table with the problems filter shows the rows of its count.

## The version of a cluster is the version of its API server

The Cluster overview shows `gitVersion` of `GET /version`, as `kubectl version` gives the server,
not the version of each kubelet. A task reads it after the connect and each 10 minutes, so an
upgrade shows. A failed read keeps the version of before with its reason, and a read follows 15 s
later. The answer needs only `gitVersion`, so a proxy that drops the other fields changes nothing.

## The CPU of a cluster counts against the allocatable part of its nodes

The overview shows the sum of the NodeMetrics of a cluster against the sum of the allocatable part
of its nodes, with the requests on them at the last poll as the mark of the bar, as the row of a
node does. A node with no allocatable value leaves the sum out: a sum of some nodes would give a
false part. A node with no sample shows in the words: "sum of 2 of 3 nodes".

## One table gives each setting its key, default and bounds

`config::table` lists each setting of the fixed groups with its group, key, label, default and
bounds, as the command table lists the commands. The reader of the file, the step that gives the
values to the core and the settings view read it, so a setting has one name and one range. A test
holds each default equal to the default of the code.

## A number outside its bounds takes the nearest bound, and a period of 0 is refused

A value outside the bounds of its setting takes the nearest bound, with a note at its line and
column in the journal. A period or a time limit of 0 is refused, and the default stays: a period
of 0 would run its loop with no pause, and a limit of 0 would fail each request. The settings view
and the core refuse each such number before a save, so the file has the number that the app uses.

## The connection settings act at the next start

The network, credential plugin, metrics, log and Events settings act at the next start of the
app. The core builds the connections, stores, sampler and log views of a cluster with them, and a
change under a running connection would need a second path in each of them. The journal tells
when a change of the file waits for a start. The other settings act at once.

## A look at the files each 2 s, with no file system events

The core takes the size and the time of change of the settings file and of each kubeconfig file
each 2 s. An editor that saves a new file under the old name, a symbolic link and a folder that
comes later each need a case of their own with the events of macOS; a look has none. The stamps
come before each read, so a change during a read gives one more read.

## A settings file that does not parse keeps the settings of the last read

After a change, a file that does not parse or has an other version leaves the settings of the last
good read in use, with an error at its place. A mistake in an editor must not make a read-only
cluster writable or remove the keys of the user. At the start, such a file gives the defaults.

## The settings view writes a change with toml_edit, and never over a file it cannot read

A save sets or removes the keys of the changes and keeps each other byte: comments, blank lines
and the sequence of the keys. The core refuses a save to a file with a parse error or an other
version, and a new text with a problem that the old text has not. A new text goes to a second
file, which takes the name of the file in one step; a symbolic link stays, and its file changes.

## A read-only cluster is closed in the view and refused in the core

Each action with a confirm of the command table, a key save and a new Secret are changes. A view
closes them with the reason, and the core refuses them too: a job, a save or a create ends as not
sent, and the server gets no request. A read, such as the rollout status, runs. The confirm rule
`type-name` keeps the button of each of these changes off until the name of the cluster is typed.

## Each command that runs has a key, and a change needs Option or Shift

The command table gives each command that the app runs a default key; an action of a later
milestone gets one with its milestone. A change of a cluster uses Option or Shift, so Cmd+C,
Cmd+Backspace and the other keys of a text field start no dialog. `?` opens the list of the keys,
and a key that a text field takes as text acts only when no text field has the focus.

## The areas take the focus of egui

Cmd+1 to Cmd+4 and F6 put the focus of egui on the open entry of the sidebar, the table, the
selected tab of the panel or of the dock. egui then moves the focus with the arrows and acts on
Space and Enter, and each focused button has the active look. The app needs no focus model of its
own; the table keeps its own keys for its rows.

## The core decides a notice, and the window tells when it is hidden

A task of the core looks each second at the jobs and the selected clusters. A failed job gives one
notice; macOS shows no banner over the window of the focus, so this needs no check of the window.
A lost cluster gives one notice for each loss when it stays lost for the time of the settings while
the window is minimized or has no focus. The program shows notices only from its bundle.

## The alias of a cluster shows in the bars, and the tables keep the context

An alias of the settings replaces the name of a cluster in the top bar and the status bar, where
room is short. The tables, the panel, the dialogs and the journal keep the name of the context: it
is the identity of the cluster, and a dialog of a change must name the target as kubectl does.
## The soak compares each store with a list on a path of its own

A store and a list are seldom equal while the cluster changes, so the soak tests "at least as
new": each listed object is in the store in its version or a later one, each other object of the
store came after the list, and an absent one is gone in a second list. The lists go on a
connection with no fault proxy. A store that shows "live" and is behind for more than 20 s fails it.

## A soak check ends at its limit, and a store or a log view must recover

A check of a store that shows "live" past 20 s ends at once, so a gap near the end of the soak is
in the report. A store that is behind for 5 minutes with a visible state did not recover: the
longest break of the harness is 3 minutes. The pods of the log view write two lines each second,
so 5 minutes with no line is a view that stopped; no line at all fails too.

## The soak counts the heap with its allocator

The memory of the soak is the count of the bytes that the allocator of `fresnel-cli` holds, not
the resident size: a growth shows at once and the same in each run. The base of the growth is the
heap at the end of the warm-up, because the first lists, the history of the metrics and the ring
of the log view grow at the start.

## The harness of the soak is a program of the test kit

`xtask` uses no other crate of the workspace, so its build stays small. `cargo xtask soak` builds
`fresnel-cli` and `fresnel-soak`, a program of the test kit with k3s, the fault proxy, the changes
and the faults, and runs it. The core runs in a child process of the harness, so the heap and the
connections that the soak records are those of the core alone.

## The pictures of the README come from the view tests

`cargo xtask screenshots` draws eight pictures with the harness of the view tests, on made-up
clusters with one fixed time, so a run needs no cluster and two runs give the same bytes. A
picture leaves out the margin of the harness: the window has none, and each pixel is opaque.
oxipng compresses them with no loss: at scale 1.5 each one has less than 450 KB. The pictures
give the README 96 lines, its new limit.

## The legend of a chart is beside its heading

The legend of `egui_plot` lies over the plot: with a line for each pod, it hid the newest
samples, and the lines ran through its names. The legend is now a row beside the heading of the
chart, with the end of the name of each pod and a dash for the request and the limit. A click on
a name still hides its line, and the tooltip then leaves the line out.
