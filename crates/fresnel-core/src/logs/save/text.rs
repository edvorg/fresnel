//! The text of a log file: a line of the view as the file has it, the head of the log of
//! one source, and the name of a file.

use std::time::SystemTime;

use k8s_openapi::jiff::Timestamp;

use super::{LogSave, SavePart};
use crate::logs::line::{LineKind, LogLine};
use crate::logs::view::SourceView;

/// Adds one line of the view to `out`, with a line end: the source in the form of `kubectl
/// logs --prefix` when the view has more than one, the time of the server in the form of
/// `kubectl logs --timestamps` when `times`, and the text. A marker of the app has `---`
/// before its text, so that no reader takes it for a line of the container.
pub(super) fn line(out: &mut String, line: &LogLine, source: Option<&SourceView>, times: bool) {
    if let Some(source) = source {
        out.push_str(&prefix(source));
        out.push(' ');
    }
    if times {
        match Timestamp::from_nanosecond(i128::from(line.time)) {
            Ok(time) => out.push_str(&format!("{time:.9} ")),
            Err(_) => out.push_str("- "),
        }
    }
    if line.kind == LineKind::Marker {
        out.push_str("--- ");
    }
    out.push_str(&line.text);
    out.push('\n');
}

/// The head of the log of one source in a file with the logs of many: the form of `tail`
/// for many files.
pub(super) fn head(source: &SourceView) -> String {
    format!("==> {} <==\n", prefix(source))
}

/// "[pod/web-1/app]".
fn prefix(source: &SourceView) -> String {
    format!("[pod/{}/{}]", source.pod.name, source.container)
}

/// The most characters of the cluster and of the object in the name of a file. A file system
/// takes a name of 255 bytes at most, and a context or a pod can have a name of 253: the
/// longest file name has 60 + 63 + 32 + 60 + 40 bytes.
const NAME_PART: usize = 60;
/// The most characters of a namespace: its limit in Kubernetes.
const NAMESPACE: usize = 63;

/// The name of the file of a save, for example `ams-prod_shop_pod-web-1_full_20270115-080000.log`:
/// the cluster, the namespace, the kind and the name, each one cut to its limit, the part and
/// the time in UTC. Each character that some file system or shell takes in a special way is
/// `_`. `attempt` 0 gives no number; a later attempt adds one, when a file of the name exists.
pub(super) fn file_name(save: &LogSave, at: SystemTime, attempt: u32) -> String {
    let request = &save.request;
    let kind = request.kind.info().name.to_ascii_lowercase();
    let part = match save.part {
        SavePart::Buffer => "buffer",
        SavePart::Full if request.options.previous => "previous",
        SavePart::Full => "full",
    };
    let time =
        Timestamp::try_from(at).map_or_else(|_| "no-time".to_owned(), |at| at.strftime("%Y%m%d-%H%M%S").to_string());
    let number = if attempt == 0 { String::new() } else { format!("-{}", attempt + 1) };
    let (cluster, namespace) = (safe(request.cluster.as_str(), NAME_PART), safe(&request.key.namespace, NAMESPACE));
    let name = safe(&request.key.name, NAME_PART);
    format!("{cluster}_{namespace}_{kind}-{name}_{part}_{time}{number}.log")
}

/// The first `most` characters of a text, each one that is not a letter, a digit, `.`, `-` or
/// `_` as `_`.
fn safe(text: &str, most: usize) -> String {
    let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' };
    text.chars().take(most).map(safe).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::cluster::ClusterId;
    use crate::kinds::Kind;
    use crate::logs::request::{LogOptions, LogRequest};
    use crate::logs::view::SourceState;
    use crate::store::ObjectKey;

    fn log(time: i64, text: &str, kind: LineKind) -> LogLine {
        LogLine { time, source: 0, text: text.into(), spans: None, kind, late: false, cut: false }
    }

    #[test]
    fn a_line_has_the_prefix_and_the_time_of_kubectl_and_a_marker_has_its_mark() {
        let source = SourceView {
            pod: ObjectKey::new("shop", "web-1"),
            container: Arc::from("app"),
            state: SourceState::Streaming,
            lines: 0,
            repeated: 0,
        };
        let mut out = String::new();
        let at = 1_760_299_200_987_654_321;
        line(&mut out, &log(at, "hello", LineKind::Log), Some(&source), true);
        line(&mut out, &log(at, "the stream started again", LineKind::Marker), None, false);
        line(&mut out, &log(1_760_299_201_000_000_000, "", LineKind::Log), None, true);
        assert_eq!(
            out,
            "[pod/web-1/app] 2025-10-12T20:00:00.987654321Z hello\n--- the stream started again\n\
             2025-10-12T20:00:01.000000000Z \n"
        );
        assert_eq!(head(&source), "==> [pod/web-1/app] <==\n");
    }

    #[test]
    fn a_file_name_tells_the_object_the_part_and_the_time_and_has_no_special_character() {
        let request = |kind, previous| LogRequest {
            cluster: ClusterId::new("ams prod"),
            kind,
            key: ObjectKey::new("shop", "web-1"),
            uid: "u".into(),
            options: LogOptions { previous, ..LogOptions::default() },
        };
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let save = |kind, part, previous| LogSave { request: request(kind, previous), part, times: true };
        assert_eq!(
            file_name(&save(Kind::Pod, SavePart::Full, false), at, 0),
            "ams_prod_shop_pod-web-1_full_20270115-080000.log"
        );
        assert_eq!(
            file_name(&save(Kind::Deployment, SavePart::Buffer, false), at, 2),
            "ams_prod_shop_deployment-web-1_buffer_20270115-080000-3.log"
        );
        assert_eq!(
            file_name(&save(Kind::Pod, SavePart::Full, true), at, 0),
            "ams_prod_shop_pod-web-1_previous_20270115-080000.log"
        );
    }

    /// A file system refuses a name of more than 255 bytes, and an object can have a name of
    /// 253 characters: each part has its limit, and the longest name fits.
    #[test]
    fn the_longest_names_give_a_file_name_of_255_bytes_or_less() {
        let kind = Kind::ALL.iter().copied().max_by_key(|kind| kind.info().name.len()).unwrap_or(Kind::Pod);
        let request = LogRequest {
            cluster: ClusterId::new("arn:aws:eks:eu-west-1:123456789012:cluster/c".repeat(8)),
            kind,
            key: ObjectKey::new(&"n".repeat(70), &"p".repeat(253)),
            uid: "u".into(),
            options: LogOptions { previous: true, ..LogOptions::default() },
        };
        let save = LogSave { request, part: SavePart::Full, times: true };
        let name = file_name(&save, SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000), 99);
        assert!(name.len() <= 255, "{} bytes: {name}", name.len());
        assert!(name.starts_with("arn_aws_eks_eu-west-1_123456789012_cluster_carn_aws_eks_eu-w_nnn"), "{name}");
        assert!(name.ends_with(&format!("-{}_previous_20270115-080000-100.log", "p".repeat(60))), "{name}");
    }
}
