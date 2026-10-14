//! The `kubectl` commands of an object (plan section 10, "copy as kubectl command"): get,
//! edit, the logs and the exec of each container, and the port forward of each port. Each
//! command names the context of its cluster (`fresnel_core::ops::kubectl`).

use egui::{Grid, Label, RichText, Ui};
use fresnel_core::kinds::Kind;
use fresnel_core::ops::kubectl;

use super::Subject;
use super::dialog::{Close, title};
use crate::theme;
use crate::widgets;

/// The most commands of one kind: a pod can have many containers and ports.
const LIMIT: usize = 20;
/// The first local port of a forward of a port below 1024, which needs root on the Mac.
const HIGH: u16 = 8000;

pub(crate) struct Commands {
    title: String,
    /// The words of each command and the command.
    lines: Vec<(String, String)>,
}

impl Commands {
    pub(crate) fn of(subject: &Subject) -> Self {
        let (target, facts) = (&subject.target, &subject.facts);
        let (cluster, kind, key) = (&target.cluster, target.kind, &target.key);
        let mut lines = vec![
            ("get".to_owned(), kubectl::get(cluster, kind, key)),
            ("edit".to_owned(), kubectl::edit(cluster, kind, key)),
        ];
        let containers = facts.containers.iter().take(LIMIT);
        match kind {
            Kind::Pod => {
                for container in containers.clone() {
                    lines.push((format!("logs of {container}"), kubectl::logs(cluster, kind, key, Some(container))));
                }
                for container in containers {
                    lines.push((format!("exec in {container}"), kubectl::exec(cluster, key, Some(container))));
                }
            }
            Kind::Deployment
            | Kind::StatefulSet
            | Kind::DaemonSet
            | Kind::ReplicaSet
            | Kind::ReplicationController
            | Kind::Job => {
                lines.push(("logs of a pod".to_owned(), kubectl::logs(cluster, kind, key, None)));
            }
            _ => {}
        }
        let forwards = matches!(
            kind,
            Kind::Pod
                | Kind::Service
                | Kind::Deployment
                | Kind::StatefulSet
                | Kind::DaemonSet
                | Kind::ReplicaSet
                | Kind::ReplicationController
                | Kind::Job
        );
        if forwards {
            for port in facts.ports.iter().take(LIMIT) {
                let command = kubectl::port_forward(cluster, kind, key, local_port(*port), *port);
                lines.push((format!("port-forward {port}"), command));
            }
        }
        Self { title: format!("kubectl commands of the {} {}", kind.info().name, key.name), lines }
    }

    pub(super) fn show(&mut self, ui: &mut Ui) -> Close {
        let palette = theme::palette(ui);
        title(ui, &self.title);
        let mut close = Close::Open;
        Grid::new("kubectl commands").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
            // The button comes first: a long command then grows to the right of it, never over it.
            for (words, command) in &self.lines {
                let copy = ui.small_button("Copy");
                widgets::name(&copy, &format!("Copy {words}"));
                if copy.clicked() {
                    ui.ctx().copy_text(command.clone());
                    close = Close::Copied(command.clone());
                }
                ui.label(RichText::new(words).color(palette.text_muted));
                ui.add(Label::new(RichText::new(command).font(theme::mono())).wrap());
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        if ui.button("Close").clicked() {
            close = Close::Cancelled;
        }
        close
    }
}

/// The local port of a forward: the same port, or 8000 more for a port that needs root.
fn local_port(remote: u16) -> u16 {
    if remote < 1024 { HIGH + remote } else { remote }
}

#[cfg(test)]
mod tests {
    use fresnel_core::cluster::ClusterId;
    use fresnel_core::ops::Target;
    use fresnel_core::store::ObjectKey;

    use super::super::Facts;
    use super::*;

    #[test]
    fn a_pod_has_the_logs_and_the_exec_of_each_container_and_a_forward_of_each_port() {
        let target = Target {
            cluster: ClusterId::new("ams-prod"),
            kind: Kind::Pod,
            key: ObjectKey::new("shop", "web-1"),
            uid: "u".into(),
        };
        let facts = Facts { containers: vec!["app".to_owned()], ports: vec![80, 9090], ..Facts::default() };
        let commands = Commands::of(&Subject { target, facts });
        let words: Vec<&str> = commands.lines.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(words, ["get", "edit", "logs of app", "exec in app", "port-forward 80", "port-forward 9090"]);
        let forward = commands.lines.iter().find(|(words, _)| words == "port-forward 80").map(|(_, command)| command);
        assert_eq!(
            forward.map(String::as_str),
            Some("kubectl --context ams-prod port-forward --namespace shop pods/web-1 8080:80")
        );
        assert_eq!(local_port(9090), 9090);
    }
}
