//! The notifications of Windows: a balloon of the icon of the app in the notification area,
//! which Windows 10 and 11 show as a toast with the name of the program.
//!
//! The toast API needs an identity of the app (an AppUserModelID), which only a shortcut in
//! the Start menu or a key of the registry gives, and the app has no installer. A balloon
//! needs neither: only a window of the app, which the icon names. The icon comes with the first
//! notice and goes with the app; the journal has each notice in each case.

use std::sync::{Mutex, PoisonError};

use fresnel_core::app::Notice;
use fresnel_core::journal::Journal;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HICON, IDI_APPLICATION, LoadIconW};

use super::front::window_of;

/// The source of the journal entries of the notifications.
const SOURCE: &str = "notice";
/// The number of the icon of the app among the icons of its window.
const ICON: u32 = 1;
/// The icon in the resources of `fresnel.exe` (`cargo xtask windows`).
const RESOURCE: u16 = 1;

/// The icon of the app in the notification area, once a notice added it.
pub(crate) struct Notifications {
    journal: Journal,
    /// The window of the icon, as a number: a handle of a window is no pointer to memory of
    /// the app, and the notices come from the task of the core.
    window: Mutex<Option<isize>>,
}

impl Notifications {
    pub(crate) fn start(journal: Journal) -> Self {
        Self { journal, window: Mutex::new(None) }
    }

    /// Shows a balloon. Before the window of the app opens, no icon can show it.
    pub(crate) fn show(&self, notice: &Notice) {
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let added = window.is_some();
        let Some(handle) = window.or_else(|| window_of(std::process::id()).map(|handle| handle as isize)) else {
            let text = format!("Windows shows no notification \"{}\": the app has no window yet", notice.title);
            return self.journal.warning(SOURCE, text);
        };
        let mut data = data(handle as HWND);
        data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
        data.hIcon = icon();
        data.dwInfoFlags = NIIF_WARNING;
        copy(&mut data.szTip, "Fresnel");
        copy(&mut data.szInfoTitle, &notice.title);
        copy(&mut data.szInfo, &notice.body);
        // A new Explorer forgets the icons of the old one, so a failed change adds it again.
        // SAFETY: `data` lives to the end of each call and has its own size in `cbSize`.
        let shown = unsafe {
            (added && Shell_NotifyIconW(NIM_MODIFY, &raw const data) != 0)
                || Shell_NotifyIconW(NIM_ADD, &raw const data) != 0
        };
        if shown {
            *window = Some(handle);
        } else {
            self.journal.warning(SOURCE, format!("Windows did not take the notification \"{}\"", notice.title));
        }
    }
}

impl Drop for Notifications {
    /// Removes the icon: the system keeps the icon of an ended app until the pointer moves
    /// over it.
    fn drop(&mut self) {
        let window = *self.window.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(handle) = window {
            let data = data(handle as HWND);
            // SAFETY: as in `show`. The result tells nothing that the app could still change.
            if unsafe { Shell_NotifyIconW(NIM_DELETE, &raw const data) } == 0 {
                self.journal.warning(SOURCE, "Windows did not remove the icon of the notifications");
            }
        }
    }
}

/// The data that names the icon of the app: its window and its number.
fn data(window: HWND) -> NOTIFYICONDATAW {
    let size = u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX);
    NOTIFYICONDATAW { cbSize: size, hWnd: window, uID: ICON, ..NOTIFYICONDATAW::default() }
}

/// The icon of `fresnel.exe`, or the icon of a program for a build with no resources.
fn icon() -> HICON {
    // SAFETY: the module of the program lives as long as the process, and each name is a number
    // of a resource, as `MAKEINTRESOURCE` gives it.
    unsafe {
        let own = LoadIconW(GetModuleHandleW(std::ptr::null()), RESOURCE as usize as *const u16);
        if own.is_null() { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) } else { own }
    }
}

/// Copies a text into a field of the system: as many whole characters as fit before the zero
/// at its end.
fn copy(field: &mut [u16], text: &str) {
    let room = field.len().saturating_sub(1);
    let mut used = 0;
    for character in text.chars() {
        let mut units = [0; 2];
        let encoded = character.encode_utf16(&mut units);
        if used + encoded.len() > room {
            break;
        }
        for unit in encoded.iter() {
            if let Some(place) = field.get_mut(used) {
                *place = *unit;
            }
            used += 1;
        }
    }
    if let Some(end) = field.get_mut(used) {
        *end = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fresnel_core::net::WallClock;

    /// A field holds whole characters and the zero at its end: a pair of UTF-16 units is not cut.
    #[test]
    fn a_text_fits_into_its_field_with_whole_characters() {
        let mut field = [7_u16; 4];
        copy(&mut field, "ab✓");
        assert_eq!(field, [97, 98, 0x2713, 0]);
        copy(&mut field, "a😀b");
        assert_eq!(field, [97, 0xD83D, 0xDE00, 0], "the pair fits, and b does not");
        copy(&mut field, "😀😀");
        assert_eq!(&field[..3], [0xD83D, 0xDE00, 0], "half of the second pair does not go in");
    }

    /// A test process has no window: the journal tells why no balloon shows, and no icon stays.
    #[test]
    fn a_notice_with_no_window_goes_to_the_journal() {
        let journal = Journal::new(10, WallClock::system());
        let notifications = Notifications::start(journal.clone());
        notifications.show(&Notice { title: "A drain failed".to_owned(), body: "node-1".to_owned() });
        let view = journal.view();
        let texts: Vec<&str> = view.entries.iter().map(|entry| entry.text.as_str()).collect();
        assert_eq!(texts, ["Windows shows no notification \"A drain failed\": the app has no window yet"]);
        assert!(notifications.window.lock().expect("a lock").is_none());
    }
}
