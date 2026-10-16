//! The status of the process of an exec: the `Status` object on the error channel.
//!
//! A process that ends with 0 has the status `Success`. An other exit code is a `Failure`
//! with the reason `NonZeroExitCode` and the code as the message of the cause `ExitCode`.
//! Each other `Failure` tells that the server did not run the process or lost it.

use serde::Deserialize;

/// How the process of an exec or an attach ended, by its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The process ended with this exit code.
    Code(i32),
    /// The server did not run the process, or lost it. The reason and the message of the
    /// server: an executable that does not exist gives `InternalError` with the text of the
    /// container runtime.
    Error {
        /// The reason of the `Status`, for example `InternalError`.
        reason: String,
        /// The message of the `Status`.
        message: String,
    },
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Code(code) => write!(f, "the process ended with the exit code {code}"),
            Self::Error { reason, message } => write!(f, "the server ended the process: {reason}: {message}"),
        }
    }
}

/// The fields of a `Status` that the exit needs. Each one is optional on the wire.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Status {
    status: String,
    reason: String,
    message: String,
    details: Details,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Details {
    causes: Vec<Cause>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Cause {
    reason: String,
    message: String,
}

/// The exit of the status in the data of the error channel.
pub(super) fn exit(data: &[u8]) -> Exit {
    let status = match serde_json::from_slice::<Status>(data) {
        Ok(status) => status,
        Err(error) => {
            let (line, column) = (error.line(), error.column());
            let message = format!("the status of the process is not JSON: error at line {line}, column {column}");
            return Exit::Error { reason: "InvalidStatus".to_owned(), message };
        }
    };
    if status.status == "Success" {
        return Exit::Code(0);
    }
    let code = (status.reason == "NonZeroExitCode").then(|| {
        let cause = status.details.causes.iter().find(|cause| cause.reason == "ExitCode")?;
        cause.message.parse().ok()
    });
    match code.flatten() {
        Some(code) => Exit::Code(code),
        None => Exit::Error { reason: status.reason, message: status.message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The texts of k3s 1.33 in the terminal prototype.
    #[test]
    fn the_status_gives_the_exit_code_or_the_reason_of_the_server() {
        assert_eq!(exit(br#"{"metadata":{},"status":"Success"}"#), Exit::Code(0));
        let three = r#"{"metadata":{},"status":"Failure","message":"command terminated with non-zero exit code: error executing command [sh -c exit 3], exit code 3","reason":"NonZeroExitCode","details":{"causes":[{"reason":"ExitCode","message":"3"}]}}"#;
        assert_eq!(exit(three.as_bytes()), Exit::Code(3));
        let missing = r#"{"metadata":{},"status":"Failure","message":"Internal error occurred: error executing command in container: exec: \"nope\": executable file not found in $PATH","reason":"InternalError","details":{"causes":[{"message":"exec: \"nope\""}]},"code":500}"#;
        let message = "Internal error occurred: error executing command in container: exec: \"nope\": executable file not found in $PATH";
        assert_eq!(
            exit(missing.as_bytes()),
            Exit::Error { reason: "InternalError".to_owned(), message: message.to_owned() }
        );
    }

    #[test]
    fn a_status_with_no_code_or_no_json_is_an_error_with_its_reason() {
        let no_code =
            r#"{"status":"Failure","reason":"NonZeroExitCode","message":"exit code unknown","details":{"causes":[]}}"#;
        let error = Exit::Error { reason: "NonZeroExitCode".to_owned(), message: "exit code unknown".to_owned() };
        assert_eq!(exit(no_code.as_bytes()), error);
        let text = "the status of the process is not JSON: error at line 1, column 4";
        assert_eq!(exit(b"{\"st"), Exit::Error { reason: "InvalidStatus".to_owned(), message: text.to_owned() });
        assert_eq!(Exit::Code(130).to_string(), "the process ended with the exit code 130");
    }
}
