//! The text report of the `doctor` command, for the terminal.

use fresnel_core::kubeconfig::{AuthKind, Kubeconfig};

use super::report::{Cluster, Connects, Credential, IdleClose, Measured, Plugin, Report, Session, Step, Summary};

/// The width of the label at the start of each line of a cluster.
const LABEL: usize = 14;

fn line(label: &str, text: impl AsRef<str>) -> String {
    format!("  {label:<LABEL$}{}", text.as_ref())
}

/// A count with its noun: "1 context", "3 contexts".
fn count(number: impl Into<u64>, noun: &str) -> String {
    match number.into() {
        1 => format!("1 {noun}"),
        number => format!("{number} {noun}s"),
    }
}

fn size(length: usize, noun: &str) -> String {
    count(u64::try_from(length).unwrap_or(u64::MAX), noun)
}

/// The smallest, the middle and the largest time.
fn times(summary: Option<&Summary>) -> String {
    match summary {
        Some(times) => format!("{:.1} / {:.1} / {:.1} ms", times.min, times.median, times.max),
        None => "no time".to_owned(),
    }
}

/// The text of a step, or the reason for its failure.
fn step<T>(step: &Step<T>, text: impl FnOnce(&T) -> String) -> String {
    match step {
        Step::Done(value) => text(value),
        Step::Failed { error } => format!("failed: {error}"),
    }
}

/// The text of the credential of a context, for the list and for the report.
pub(super) fn auth(kind: &AuthKind) -> String {
    match kind {
        AuthKind::Plugin(command) => format!("plugin {command}"),
        AuthKind::Certificate => "client certificate".to_owned(),
        AuthKind::Anonymous => "no credential".to_owned(),
    }
}

/// The list of the contexts, for a run with no selected context.
pub(super) fn contexts(kubeconfig: &Kubeconfig) -> String {
    let files = size(kubeconfig.sources.len() - kubeconfig.absent.len(), "kubeconfig file");
    let mut lines = vec![format!("{} in {files}:", size(kubeconfig.contexts.len(), "context"))];
    for context in &kubeconfig.contexts {
        lines.push(match &context.access {
            Ok(access) => format!("  {}  {}  {}", context.name, access.server, auth(&access.auth())),
            Err(error) => format!("  {}  not usable: {error}", context.name),
        });
    }
    let problems = kubeconfig.problems.iter();
    lines.extend(problems.map(|problem| format!("problem: {}: {}", problem.path.display(), problem.text)));
    lines.push("Select a context with --context <name>, or each context with --all.".to_owned());
    lines.join("\n") + "\n"
}

/// The full report.
pub(super) fn render(report: &Report) -> String {
    let environment = &report.environment;
    let mut lines = vec![
        format!("{} doctor, {}", report.tool, report.time),
        format!(
            "environment: {} in {:.1} ms, {} folders in PATH ({} in the PATH of this process)",
            environment.source, environment.resolve_ms, environment.path_entries, environment.process_path_entries
        ),
    ];
    lines.extend(environment.problem.iter().map(|problem| format!("  problem: {problem}")));
    let kubeconfig = &report.kubeconfig;
    lines.push(format!("kubeconfig: {}", size(kubeconfig.contexts, "context")));
    let present = kubeconfig.sources.iter().filter(|path| !kubeconfig.absent.contains(path));
    lines.extend(present.map(|path| format!("  file: {path}")));
    lines.extend(kubeconfig.absent.iter().map(|path| format!("  no such file: {path}")));
    lines.extend(kubeconfig.problems.iter().map(|problem| format!("  problem: {problem}")));
    for cluster in &report.clusters {
        lines.push(String::new());
        lines.extend(cluster_lines(cluster));
    }
    if !report.task_reports.is_empty() {
        lines.push(String::new());
        lines.push("tasks of the app that ended:".to_owned());
        lines.extend(report.task_reports.iter().map(|task| format!("  {task}")));
    }
    lines.join("\n") + "\n"
}

fn cluster_lines(cluster: &Cluster) -> Vec<String> {
    let facts: Vec<&str> = [&cluster.server, &cluster.auth].into_iter().flatten().map(String::as_str).collect();
    let mut lines = vec![format!("cluster {} ({})", cluster.context, facts.join(", "))];
    lines.extend(cluster.plugin.iter().map(|plugin| line("plugin", plugin_text(plugin))));
    match &cluster.result {
        Step::Done(measured) => lines.extend(measured_lines(measured)),
        Step::Failed { error } => lines.push(line("not measured", error)),
    }
    lines
}

fn plugin_text(plugin: &Plugin) -> String {
    let place = match (&plugin.folder, plugin.in_process_path) {
        (Some(folder), true) => format!("in {folder}"),
        (Some(folder), false) => format!("in {folder}, which only the login shell has in PATH"),
        (None, _) => "not found".to_owned(),
    };
    format!("{} {place}; search {:.1} ms", plugin.command, plugin.search_ms)
}

fn measured_lines(measured: &Measured) -> Vec<String> {
    let dns = step(&measured.dns, |dns| {
        let lookup = if dns.time_ms == 0.0 { "no lookup".to_owned() } else { format!("{:.1} ms", dns.time_ms) };
        format!("{lookup}; {}", dns.addresses.join(", "))
    });
    let offered = |offer: &Step<bool>| step(offer, |_| "yes".to_owned());
    let protocols = format!("HTTP/2: {}; HTTP/1.1: {}", offered(&measured.alpn.http2), offered(&measured.alpn.http1));
    let mut lines = vec![line("DNS", dns), line("ALPN", protocols)];
    lines.extend(measured.plain.iter().map(|plain| line("no hedge", plain_text(plain))));
    lines.extend(measured.hedged.iter().map(|hedged| line("hedge", hedged_text(hedged))));
    match &measured.session {
        Step::Done(session) => lines.extend(session_lines(session)),
        Step::Failed { error } => lines.push(line("channels", format!("failed: {error}"))),
    }
    lines
}

/// The count of connects with the share of held ones.
fn held(connects: &Connects) -> String {
    let failed = if connects.failed == 0 { String::new() } else { format!(", {} failed", connects.failed) };
    format!("{}, {} held ({:.1} %){failed}", size(connects.samples, "connect"), connects.held, connects.held_percent)
}

fn plain_text(plain: &Connects) -> String {
    let (tcp, tls) = (times(plain.tcp_ms.as_ref()), times(plain.tls_ms.as_ref()));
    format!("{}; TCP {tcp}; TLS {tls}", held(plain))
}

fn hedged_text(hedged: &Connects) -> String {
    let attempts = size(hedged.attempts.unwrap_or_default(), "attempt");
    let (total, delay) = (times(hedged.total_ms.as_ref()), hedged.hedge_delay_ms.unwrap_or_default());
    format!("{}; {total}; {attempts}; delay {delay:.0} ms", held(hedged))
}

fn session_lines(session: &Session) -> Vec<String> {
    let mut lines: Vec<String> = session.credential.iter().flat_map(credential_lines).collect();
    let connections: u64 = session.channels.iter().map(|channel| channel.connections).sum();
    let no_token = session.credential.as_ref().is_some_and(|credential| credential.first_token_ms.is_none());
    let live = match session.live_ms {
        Some(time) => format!("each one live after {time:.1} ms; {}", count(connections, "connection")),
        None if no_token => "no wait for an answer: the cluster has no token".to_owned(),
        None => "not live".to_owned(),
    };
    lines.push(line("channels", live));
    // A channel that is live at the end needs no line of its own.
    for channel in session.channels.iter().filter(|channel| channel.state != "live") {
        let error = channel.last_error.as_deref().unwrap_or("no error");
        lines.push(line("", format!("{}: {}; {error}", channel.purpose, channel.state)));
    }
    let limit = session.stream_limit.map_or_else(|| "not known".to_owned(), |limit| limit.to_string());
    lines.push(line("HTTP/2", format!("stream limit of the server: {limit}")));
    lines.push(line("ping", format!("{} (small requests on the open connection)", times(session.ping_ms.as_ref()))));
    lines.extend(session.ping_error.iter().map(|error| line("", format!("a later request failed: {error}"))));
    let version = step(&session.version, |version| format!("{} ({})", version.git_version, version.platform));
    lines.push(line("GET /version", version));
    lines.extend(session.idle_close.iter().flat_map(idle_lines));
    lines
}

fn credential_lines(credential: &Credential) -> Vec<String> {
    let token = credential.first_token_ms.map_or_else(String::new, |time| format!("; first token after {time:.1} ms"));
    let runs = count(credential.plugin_runs, "plugin run");
    let mut lines = vec![line("credential", format!("{}{token}; {runs}", credential.state))];
    // The text of the plugin is for the user at the terminal. The JSON file does not have it.
    lines.extend(credential.detail.lines().map(|text| line("", format!("plugin: {text}"))));
    lines
}

fn idle_lines(idle: &IdleClose) -> Vec<String> {
    let closed = match (idle.closed_after_s, &idle.reason) {
        (Some(after), Some(reason)) => format!("the idle connection ended after {after:.1} s ({reason})"),
        (Some(after), None) => format!("the idle connection ended after {after:.1} s"),
        (None, _) => format!("the idle connection was open after {:.1} s", idle.waited_s),
    };
    let kept = if idle.light_request_kept_open { "stayed open" } else { "did not stay open" };
    vec![line("idle close", closed), line("", format!("the channels with a light request {kept}"))]
}

#[cfg(test)]
mod tests;
