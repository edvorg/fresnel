//! The Pod table: its values, the sort, the filters, the count and the selection.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;

use crate::data::{live, pod, pods, renamed};
use crate::fake::{Window, shows, until, view, window};
use crate::pods::column;

/// A window that is large enough for the 25 pods of the scene, also next to the detail panel.
fn tall(clusters: Vec<fresnel_core::app::ClusterRow>) -> Window {
    let mut window = window(view(clusters));
    window.set_size(egui::vec2(1800.0, 1100.0));
    window.run();
    window
}

fn click_title(window: &mut Window, title: &str) {
    window.get_by_role_and_label(Role::Button, title).click();
    window.run();
}

fn until_first(window: &mut Window, title: &str, first: &str) {
    until(window, &format!("{first} is the first value of {title}"), |window| {
        column(window, title).first().map(String::as_str) == Some(first)
    });
}

/// Plan section 11.2: the columns Ready, Status, Restarts and Last restart have the
/// values of `kubectl get pods`. The fixture has the output of kubectl for its pods.
#[test]
fn the_columns_have_the_values_of_kubectl_get_pods() {
    let window = tall(vec![live("ams-prod", &pods())]);
    let columns = ["Namespace", "Name", "Ready", "Status", "Restarts", "Last restart", "Age", "Cluster", "Node"];
    let [namespaces, names, ready, status, restarts, last_restart, age, clusters, nodes] =
        columns.map(|title| column(&window, title));
    assert_eq!(names.len(), 25, "each pod of the scene shows");
    let mut rows: Vec<String> = (0..names.len())
        .map(|row| {
            let restarts = match last_restart[row].as_str() {
                "–" => restarts[row].clone(),
                ago => format!("{} ({ago})", restarts[row]),
            };
            [namespaces[row].as_str(), &names[row], &ready[row], &status[row], &restarts, &age[row]].join(" ")
        })
        .collect();
    rows.sort();

    let kubectl = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fresnel-core/fixtures/k3s/pods.kubectl.txt"));
    // kubectl prints "27s", and the app prints "27 s".
    let spaced = |text: &str| text.replace("s ago)", " s ago)");
    let mut expected: Vec<String> = kubectl
        .lines()
        .map(|line| {
            let line = spaced(&line.split_whitespace().collect::<Vec<_>>().join(" "));
            let (start, age) = line.rsplit_once(' ').expect("an age");
            format!("{start} {} s", age.trim_end_matches('s'))
        })
        .collect();
    expected.sort();
    assert_eq!(rows, expected);
    assert!(clusters.iter().all(|cluster| cluster == "ams-prod"), "{clusters:?}");
    assert_eq!(nodes.iter().filter(|node| *node == "–").count(), 2, "two pods of the scene have no node");
}

#[test]
fn a_click_on_a_title_sorts_by_typed_values_and_a_second_click_turns_the_sequence() {
    let mut window = tall(vec![live("ams-prod", &pods())]);
    assert_eq!(column(&window, "Name")[..2], ["agent-8djvd", "broken-6bf768c954-gjvsw"], "the first sort is by name");

    // The youngest pod is first: the sort value is the creation time, not the text.
    click_title(&mut window, "Age");
    until_first(&mut window, "Name", "tick-manual-d6v5w");
    assert_eq!(column(&window, "Age")[..4], ["35 s", "36 s", "37 s", "45 s"]);
    click_title(&mut window, "Age");
    until(&mut window, "the oldest pods are first", |window| {
        column(window, "Age").last().is_some_and(|age| age == "35 s")
    });

    click_title(&mut window, "Restarts");
    click_title(&mut window, "Restarts");
    until_first(&mut window, "Restarts", "2");
    // Two pods have 2 restarts. The last key of each sort gives them one sequence.
    // The pods with no restart follow by namespace and name.
    assert_eq!(column(&window, "Name")[..3], ["crash", "init-crash", "coredns-64fd4b4794-6lsjp"]);
    click_title(&mut window, "Status");
    until_first(&mut window, "Status", "Completed");
}

#[test]
fn the_text_the_namespace_and_the_problems_filter_select_rows_and_the_count_tells_it() {
    let mut window = tall(vec![live("ams-prod", &pods())]);
    assert!(shows(&window, "25 pods"));

    window.get_by_label("Filter pods").click();
    window.run();
    window.get_by_label("Filter pods").type_text("CRASH");
    until(&mut window, "the rows of the text filter", |window| shows(window, "2 of 25 pods"));
    assert_eq!(column(&window, "Name"), ["crash", "init-crash"]);
    window.get_by_label("Filter pods").type_text("-no-such-pod");
    until(&mut window, "no row", |window| shows(window, "0 of 25 pods"));
    assert!(shows(&window, "No pod passes the filters. The selected clusters have 25 pods."));

    // The text field is empty again. The menu lists each namespace with a pod.
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.key_press(egui::Key::Backspace);
    until(&mut window, "each row again", |window| shows(window, "25 pods"));
    window.get_by_label("All namespaces").click();
    window.run();
    window.get_by_role_and_label(Role::CheckBox, "kube-system").click();
    until(&mut window, "the rows of one namespace", |window| shows(window, "2 of 25 pods"));
    assert_eq!(column(&window, "Namespace"), ["kube-system", "kube-system"]);
    assert!(shows(&window, "Namespace: kube-system"), "the button tells the filter");
    window.get_by_role_and_label(Role::CheckBox, "scene").click();
    until(&mut window, "the rows of two namespaces", |window| shows(window, "25 pods"));
    assert!(shows(&window, "2 namespaces"));
    window.get_by_role_and_label(Role::Button, "All namespaces").click();
    window.run();
    window.key_press(egui::Key::Escape);
    until(&mut window, "no namespace filter", |window| shows(window, "All namespaces"));

    window.get_by_label("Problems").click();
    until(&mut window, "the pods that are not healthy", |window| shows(window, "10 of 25 pods"));
    let expected = [
        ("broken-6bf768c954-gjvsw", "ImagePullBackOff"),
        ("crash", "CrashLoopBackOff"),
        ("failed", "Error"),
        ("fails-l8dvz", "Error"),
        ("half-done", "NotReady"),
        ("init-crash", "Init:CrashLoopBackOff"),
        ("init-error", "Init:Error"),
        ("no-config", "CreateContainerConfigError"),
        // This pod runs and is not ready. The pod `done` completed, and that is no problem.
        ("not-ready", "Running"),
        ("unschedulable", "Pending"),
    ];
    let shown: Vec<(String, String)> = column(&window, "Name").into_iter().zip(column(&window, "Status")).collect();
    assert_eq!(shown, expected.map(|(name, status)| (name.to_owned(), status.to_owned())));
}

/// A namespace of the filter whose last pod went away stays in the menu, so the user can
/// remove it from the filter.
#[test]
fn a_namespace_of_the_filter_with_no_pod_stays_in_the_menu() {
    let mut window = tall(vec![live("ams-prod", &pods())]);
    window.get_by_label("All namespaces").click();
    window.run();
    window.get_by_role_and_label(Role::CheckBox, "kube-system").click();
    until(&mut window, "the rows of one namespace", |window| shows(window, "2 of 25 pods"));

    let scene_only: Vec<_> =
        pods().into_iter().filter(|pod| pod.metadata.namespace.as_deref() == Some("scene")).collect();
    window.state_mut().view = view(vec![live("ams-prod", &scene_only)]);
    until(&mut window, "no pod of the namespace", |window| shows(window, "0 of 23 pods"));
    window.get_by_role_and_label(Role::CheckBox, "kube-system").click();
    until(&mut window, "each pod of the cluster", |window| shows(window, "23 pods"));
}

/// Plan section 11.2 and defect A1b of the prototype: the selection holds objects, it
/// follows them through a new sort, and it drops each object that goes away.
#[test]
fn the_selection_follows_its_objects_and_drops_the_ones_that_go_away() {
    let mut window = tall(vec![live("ams-prod", &pods())]);
    window.get_by_label("crash").click();
    window.get_by_label("done").click_modifiers(egui::Modifiers::SHIFT);
    window.run();
    assert!(shows(&window, "4 selected"), "crash, db-0, db-1 and done");

    // A new sort moves the rows. The selection is the same four pods.
    click_title(&mut window, "Age");
    until_first(&mut window, "Name", "tick-manual-d6v5w");
    assert!(shows(&window, "4 selected"));

    // Two of the four pods go away, and a new pod has the name of one of them.
    let without = |gone: &[&str]| -> Vec<_> {
        pods().into_iter().filter(|pod| !gone.contains(&pod.metadata.name.as_deref().unwrap_or_default())).collect()
    };
    let mut next = without(&["db-0", "db-1"]);
    next.push(renamed(&pod("db-0"), "db-0", "the-uid-of-a-new-pod"));
    window.state_mut().view = view(vec![live("ams-prod", &next)]);
    until(&mut window, "the rows of the new pods", |window| shows(window, "24 pods"));
    assert!(shows(&window, "2 selected"), "the new db-0 is an other object");

    window.state_mut().view = view(vec![live("ams-prod", &without(&["db-0", "db-1", "done"]))]);
    until(&mut window, "the rows of the new pods", |window| shows(window, "22 pods"));
    assert!(window.query_all_by_label_contains(" selected").next().is_none(), "one object is no count");
    window.state_mut().view = view(vec![live("ams-prod", &without(&["db-0", "db-1", "done", "crash"]))]);
    until(&mut window, "the rows of the new pods", |window| shows(window, "21 pods"));
    assert!(!shows(&window, "Close"), "the last selected pod went away, and its panel tells it");
}
