//! The output of `kubectl get pods`, which a test compares with the Pod table of the app.

use super::K3s;
use crate::KitError;

/// One line of `kubectl get pods --all-namespaces --no-headers`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodLine {
    /// The column NAMESPACE.
    pub namespace: String,
    /// The column NAME.
    pub name: String,
    /// The column READY, for example `1/2`.
    pub ready: String,
    /// The column STATUS.
    pub status: String,
    /// The count of the column RESTARTS.
    pub restarts: u32,
    /// True when the column RESTARTS has the time of the last restart, as in `3 (25s ago)`.
    pub restarted_ago: bool,
}

impl PodLine {
    /// Reads one line. The column RESTARTS has one word or three, and the last column is
    /// the age of the pod.
    pub fn parse(line: &str) -> Result<Self, KitError> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let (columns, restarted_ago) = match words.as_slice() {
            [columns @ .., _since, "ago)", _age] => (columns, true),
            [columns @ .., _age] => (columns, false),
            [] => (&[][..], false),
        };
        let problem = || KitError::new(format!("not a line of `kubectl get pods`: {line:?}"));
        let [namespace, name, ready, status, restarts] = columns else {
            return Err(problem());
        };
        Ok(Self {
            namespace: (*namespace).to_owned(),
            name: (*name).to_owned(),
            ready: (*ready).to_owned(),
            status: (*status).to_owned(),
            restarts: restarts.parse().map_err(|_| problem())?,
            restarted_ago,
        })
    }

    /// Reads each line of the output of `kubectl get pods --all-namespaces --no-headers`.
    pub fn parse_all(output: &str) -> Result<Vec<Self>, KitError> {
        output.lines().map(Self::parse).collect()
    }
}

impl K3s {
    /// The output of `kubectl get pods` for each namespace, with no header line.
    pub async fn pod_lines(&self) -> Result<String, KitError> {
        self.kubectl(&["get", "pods", "--all-namespaces", "--no-headers"]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_kubectl_gives_its_columns_with_and_with_no_time_of_the_last_restart() {
        let output = "scene   crash   0/1   CrashLoopBackOff   3 (25s ago)   2m5s\n\
                      kube-system   coredns-64fd4b4794-drnpt   1/1   Running   0   20s\n";
        let lines = PodLine::parse_all(output).expect("two lines");
        let columns =
            |line: &PodLine| (line.namespace.clone(), line.name.clone(), line.ready.clone(), line.status.clone());
        let text = |words: [&str; 4]| words.map(str::to_owned).into();
        assert_eq!(columns(&lines[0]), text(["scene", "crash", "0/1", "CrashLoopBackOff"]));
        assert_eq!((lines[0].restarts, lines[0].restarted_ago), (3, true));
        assert_eq!(columns(&lines[1]), text(["kube-system", "coredns-64fd4b4794-drnpt", "1/1", "Running"]));
        assert_eq!((lines[1].restarts, lines[1].restarted_ago), (0, false));

        for bad in ["", "No resources found", "scene crash 0/1 Error many 5s"] {
            let error = PodLine::parse(bad).expect_err("no line of a pod").to_string();
            assert_eq!(error, format!("not a line of `kubectl get pods`: {bad:?}"));
        }
    }
}
