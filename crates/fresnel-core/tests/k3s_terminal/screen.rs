//! The screen of a terminal as text, the keys of the user, and the processes of a pod.

use fresnel_core::terminal::{Frame, Palette, Terminal};

use crate::bed::Bed;

/// The rows of the screen, as the widget copies them.
pub(crate) fn rows(terminal: &Terminal) -> Vec<String> {
    let mut frame = Frame::default();
    terminal.frame(&mut frame, &Palette::DARK);
    (0..usize::from(frame.size.rows)).map(|row| frame.text(row)).collect()
}

/// True while the program shows the alternate screen, as `vi` does.
pub(crate) fn alternate(terminal: &Terminal) -> bool {
    let mut frame = Frame::default();
    terminal.frame(&mut frame, &Palette::DARK);
    frame.modes.alt_screen
}

/// The column of the cursor. `None` while it is hidden.
pub(crate) fn cursor_column(terminal: &Terminal) -> Option<usize> {
    let mut frame = Frame::default();
    terminal.frame(&mut frame, &Palette::DARK);
    frame.cursor.map(|cursor| cursor.column)
}

/// Keys of the user, as the widget gives them.
pub(crate) fn keys(terminal: &Terminal, text: &str) {
    terminal.send(text.as_bytes()).expect("the input has room");
}

/// The processes of a container of a pod of the namespace `default`, by `ps` of busybox,
/// besides `ps` itself: each line is "PID USER TIME COMMAND".
pub(crate) async fn processes(bed: &Bed, pod: &str, container: &str) -> Vec<String> {
    let listed = bed.kubectl(&["exec", pod, "-c", container, "--", "ps", "-o", "pid,args"]).await;
    listed.lines().skip(1).filter(|line| !line.ends_with("ps -o pid,args")).map(str::to_owned).collect()
}
