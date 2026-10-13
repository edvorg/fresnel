//! The commands as a program, on the fake API server of the test kit.
//!
//! Each test starts the program as a child process with the environment of a program from
//! Finder. The home folder is a folder of the test, so the login shell of the user runs a
//! profile of the test and not the profile of the user.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod doctor;
mod home;
mod watch;
