//! The window of a process, and its move to the front at a second start: the lock file of
//! the first process has its process id (`lock`), so no title or class of a window is needed.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};
use windows_sys::core::BOOL;

/// The search of [`window_of`]: the process, and the window once found.
struct Search {
    process: u32,
    found: HWND,
}

/// Brings the window of the first process of the app to the front, and gives true when the
/// system did it. Windows lets the process that the user just started set the foreground
/// window; a window that is minimized comes back first. False when the process has no
/// window yet, or no more: the caller then shows its message.
pub(crate) fn show_first(process: u32) -> bool {
    let Some(window) = window_of(process) else {
        return false;
    };
    // SAFETY: the window handle comes from the system; a window that closed since then makes
    // each call fail, and nothing reads memory through the handle.
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window) != 0
    }
}

/// The main window of a process: a top-level window that is visible and has no owner.
pub(super) fn window_of(process: u32) -> Option<HWND> {
    let mut search = Search { process, found: null_mut() };
    // SAFETY: `each` gets the pointer to `search`, which lives to the end of the call; the
    // system calls `each` only during the call.
    unsafe { EnumWindows(Some(each), (&raw mut search) as LPARAM) };
    (!search.found.is_null()).then_some(search.found)
}

/// Takes one window of the list of the system. Gives 0 to end the list at the window found.
unsafe extern "system" fn each(window: HWND, search: LPARAM) -> BOOL {
    // SAFETY: `window_of` gives the pointer to its `Search`, which outlives the list.
    let search = unsafe { &mut *(search as *mut Search) };
    let mut owner = 0;
    // SAFETY: the system gives a valid window, and `owner` lives to the end of the call.
    let main = unsafe {
        GetWindowThreadProcessId(window, &raw mut owner);
        owner == search.process && IsWindowVisible(window) != 0 && GetWindow(window, GW_OWNER).is_null()
    };
    if main {
        search.found = window;
    }
    BOOL::from(!main)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test process has no window, so the caller shows its message.
    #[test]
    fn a_process_with_no_window_has_no_window_to_bring_to_the_front() {
        assert_eq!(window_of(std::process::id()), None);
        assert!(!show_first(std::process::id()));
    }
}
