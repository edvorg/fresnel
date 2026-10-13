use std::time::Duration;

use super::*;
use crate::testing::Scratch;

fn entry(number: u64, level: Level, source: &str, text: &str) -> Entry {
    let time = SystemTime::UNIX_EPOCH + Duration::from_millis(1_800_000_000_123 + number * 1000);
    Entry { number, time, level, source: source.to_owned(), text: text.to_owned() }
}

#[test]
fn each_entry_is_one_line_and_a_new_run_continues_the_file() {
    let folder = Scratch::new("log-lines");
    let mut log = LogFile::open(folder.path(), Rotation::default()).expect("the log file");
    log.write(&entry(0, Level::Info, "app", "fresnel 0.1.0 started"));
    log.write(&entry(1, Level::Warning, "sgp-prod", "no connection (connect: no answer in 10s)"));
    log.flush();
    drop(log);
    let mut log = LogFile::open(folder.path(), Rotation::default()).expect("the log file of the next run");
    log.write(&entry(2, Level::Error, "task", "task watch [ams]\npanicked: a \\ in\r\nthe text"));

    let expected = "\
2027-01-15T08:00:00.123Z INFO  [app] fresnel 0.1.0 started
2027-01-15T08:00:01.123Z WARN  [sgp-prod] no connection (connect: no answer in 10s)
2027-01-15T08:00:02.123Z ERROR [task] task watch [ams]\\npanicked: a \\\\ in\\r\\nthe text
";
    assert_eq!(folder.read("fresnel.log"), expected);
    assert_eq!(folder.files(), ["fresnel.log"]);
}

#[test]
fn a_full_file_starts_a_new_one_and_only_the_newest_files_stay() {
    let folder = Scratch::new("log-rotation");
    // One line has 51 bytes, so a file takes 3 lines.
    let rotation = Rotation { max_bytes: 160, keep: 2 };
    let mut log = LogFile::open(folder.path(), rotation).expect("the log file");
    let numbers = |file: &str| -> Vec<u64> {
        let text = folder.read(file);
        assert!(text.ends_with('\n') && text.len() <= 160, "{file} has complete lines and its limit: {text:?}");
        text.lines().map(|line| line.rsplit(' ').next().expect("a number").parse().expect("a number")).collect()
    };
    for number in 0..7 {
        log.write(&entry(number, Level::Info, "app", &format!("entry number {number}")));
    }
    assert_eq!(folder.files(), ["fresnel.1.log", "fresnel.2.log", "fresnel.log"]);
    assert_eq!(
        (numbers("fresnel.2.log"), numbers("fresnel.1.log"), numbers("fresnel.log")),
        (vec![0, 1, 2], vec![3, 4, 5], vec![6])
    );

    for number in 7..10 {
        log.write(&entry(number, Level::Info, "app", &format!("entry number {number}")));
    }
    assert_eq!(folder.files(), ["fresnel.1.log", "fresnel.2.log", "fresnel.log"], "the oldest file went away");
    assert_eq!(
        (numbers("fresnel.2.log"), numbers("fresnel.1.log"), numbers("fresnel.log")),
        (vec![3, 4, 5], vec![6, 7, 8], vec![9])
    );

    // One entry that is larger than the limit gets a file of its own.
    drop(log);
    let mut log = LogFile::open(folder.path(), Rotation { max_bytes: 160, keep: 0 }).expect("the log file");
    log.write(&entry(10, Level::Info, "app", &"long ".repeat(60)));
    log.write(&entry(11, Level::Info, "app", "entry number 11"));
    assert_eq!(numbers("fresnel.log"), [11], "with no older files, the file starts again");
}

#[test]
fn entries_that_an_error_lost_show_as_a_count_in_the_next_line() {
    let folder = Scratch::new("log-error");
    let logs = folder.path().join("logs");
    std::fs::create_dir(&logs).expect("the log folder");
    // Each entry starts a new file, and a new file needs the folder.
    let mut log = LogFile::open(&logs, Rotation { max_bytes: 1, keep: 1 }).expect("the log file");
    log.write(&entry(0, Level::Info, "app", "before the error"));
    std::fs::remove_dir_all(&logs).expect("the folder goes away");
    log.write(&entry(1, Level::Info, "app", "lost"));
    log.write(&entry(2, Level::Info, "app", "lost too"));

    std::fs::create_dir(&logs).expect("the folder is back");
    log.write(&entry(3, Level::Info, "app", "after the error"));
    log.write(&entry(4, Level::Info, "app", "the last entry"));
    let older = std::fs::read_to_string(logs.join("fresnel.1.log")).expect("the older file");
    let lines: Vec<&str> = older.lines().collect();
    assert_eq!(lines.len(), 2, "{older}");
    assert!(lines[0].contains(" ERROR [log file] an error of the log file ("), "{older}");
    assert!(lines[0].ends_with("); entries that are not in the file: 2"), "{older}");
    assert!(lines[1].ends_with(" INFO  [app] after the error"), "{older}");
    let newest = std::fs::read_to_string(logs.join("fresnel.log")).expect("the log file");
    assert!(newest.ends_with(" INFO  [app] the last entry\n") && newest.lines().count() == 1, "{newest}");
}
