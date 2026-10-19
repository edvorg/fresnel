//! The answer of the app to a quit on macOS.
//!
//! The Quit item of the app menu and Cmd+Q, the Quit item of the Dock, a script, a log out, a
//! restart and a shut down each reach `terminate:` of `NSApplication`, which asks the delegate
//! of the app with `applicationShouldTerminate:` before the end. winit owns the delegate and
//! leaves that question out, so the program adds its answer to the class of the delegate. The
//! Quit item keeps `terminate:` with no target: the app takes it in each state of the window,
//! also minimized. The answer is the decision of the window ([`fresnel_ui::QuitQuestion`]):
//! `NSTerminateNow`, or `NSTerminateCancel` with the app in front for the question.

use std::cell::RefCell;
use std::ffi::CStr;

use fresnel_ui::Quit;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{ffi, msg_send, sel};

/// `NSTerminateCancel` and `NSTerminateNow` of `NSApplicationTerminateReply`.
const CANCEL: usize = 0;
const NOW: usize = 1;
/// The types of the answer: an `NSUInteger` from the delegate, the selector and the app.
const TYPES: &CStr = c"Q@:@";
/// The Apple Event of a quit, and the key of its reason (`AppleEvents.h`, `AERegistry.h`).
const CORE_EVENT: u32 = code(*b"aevt");
const QUIT_EVENT: u32 = code(*b"quit");
const QUIT_REASON: u32 = code(*b"why?");
/// The reasons of an end of the session: a quit of each app, a log out, a restart, a shut
/// down, and the dialogs of the system for the last three.
const SESSION_ENDS: [u32; 7] =
    [code(*b"quia"), code(*b"logo"), code(*b"rlgo"), code(*b"rrst"), code(*b"rest"), code(*b"rsdn"), code(*b"shut")];

/// The function of the answer, as AppKit calls `applicationShouldTerminate:`.
type Answer = extern "C" fn(&AnyObject, Sel, Option<&AnyObject>) -> usize;

thread_local! {
    /// The decision of the window. AppKit asks on the main thread, where the window gives it.
    static DECIDE: RefCell<Option<Box<dyn Fn() -> Quit>>> = const { RefCell::new(None) };
}

/// A four character code of the Apple Events.
const fn code(text: [u8; 4]) -> u32 {
    u32::from_be_bytes(text)
}

/// Each quit of the app takes the decision of the window while this value lives. Before and
/// after, a quit ends the app at once, as with no answer.
pub(crate) struct QuitAnswer(());

impl QuitAnswer {
    fn new(decide: impl Fn() -> Quit + 'static) -> Self {
        DECIDE.set(Some(Box::new(decide)));
        Self(())
    }
}

impl Drop for QuitAnswer {
    fn drop(&mut self) {
        DECIDE.set(None);
    }
}

/// Gives `decide` of the window to each quit of the app. The window calls this one time, on
/// the main thread, after winit set its delegate.
pub(crate) fn answer_quit(decide: impl Fn() -> Quit + 'static) -> Result<QuitAnswer, String> {
    add_answer(delegate_class()?)?;
    Ok(QuitAnswer::new(decide))
}

/// The class of the delegate of the app, which winit made.
fn delegate_class() -> Result<&'static AnyClass, String> {
    let class = AnyClass::get(c"NSApplication").ok_or("AppKit is not loaded")?;
    // SAFETY: `sharedApplication` and `delegate` take no argument and give an object or nil.
    let delegate: Option<Retained<AnyObject>> = unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![class, sharedApplication];
        msg_send![&app.ok_or("AppKit has no application")?, delegate]
    };
    Ok(delegate.ok_or("the app has no delegate")?.class())
}

/// Adds the answer to `class`. A class that answers already keeps its answer: a newer winit
/// can answer itself, and the journal then tells that a quit asks no question.
fn add_answer(class: &AnyClass) -> Result<(), String> {
    let selector = sel!(applicationShouldTerminate:);
    if class.instance_method(selector).is_some() {
        return Err(format!("{} answers applicationShouldTerminate: itself", class.name().to_string_lossy()));
    }
    let answer: Answer = should_terminate;
    // SAFETY: an `Imp` is the untyped pointer of a method. The runtime calls it with the types
    // of `TYPES`, which are those of `Answer`, and the function lives as long as the program.
    let added = unsafe {
        let answer = std::mem::transmute::<Answer, Imp>(answer);
        ffi::class_addMethod(std::ptr::from_ref(class).cast_mut(), selector, answer, TYPES.as_ptr())
    };
    if added.as_bool() { Ok(()) } else { Err("the runtime refused the answer".to_owned()) }
}

/// The answer to `applicationShouldTerminate:`, on the main thread, with the app as the
/// sender. An end of the session ends the app at once; each other quit takes the decision of
/// the window, and its question brings the app to the front.
extern "C" fn should_terminate(_delegate: &AnyObject, _selector: Sel, app: Option<&AnyObject>) -> usize {
    let quit = if ends_the_session(current_event().as_deref()) {
        Quit::Now
    } else {
        DECIDE.with_borrow(|decide| decide.as_ref().map_or(Quit::Now, |decide| decide()))
    };
    match quit {
        Quit::Now => NOW,
        Quit::Ask => {
            if let Some(app) = app {
                come_to_front(app);
            }
            CANCEL
        }
    }
}

/// The Apple Event that AppKit handles now: the quit of the Dock, of a script or of the
/// system. A quit from the menu has none.
fn current_event() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"NSAppleEventManager")?;
    // SAFETY: `sharedAppleEventManager` and `currentAppleEvent` take no argument and give an
    // object or nil.
    unsafe {
        let manager: Option<Retained<AnyObject>> = msg_send![class, sharedAppleEventManager];
        msg_send![&manager?, currentAppleEvent]
    }
}

/// True for the quit of a log out, a restart or a shut down. A question then holds the end of
/// the session until the user answers, also when the user is away, and `NSTerminateLater`
/// would run the window in the modal loop of AppKit. So the app ends at once, and the quit
/// sequence ends the shells in 5 s at most.
fn ends_the_session(event: Option<&AnyObject>) -> bool {
    let Some(event) = event else {
        return false;
    };
    // SAFETY: `eventClass` and `eventID` give a four character code. The two descriptor
    // methods take one and give a descriptor or nil; `typeCodeValue` gives the code that a
    // descriptor holds, 0 for none.
    unsafe {
        let class: u32 = msg_send![event, eventClass];
        let id: u32 = msg_send![event, eventID];
        if (class, id) != (CORE_EVENT, QUIT_EVENT) {
            return false;
        }
        let attribute: Option<Retained<AnyObject>> = msg_send![event, attributeDescriptorForKeyword: QUIT_REASON];
        let reason: Option<Retained<AnyObject>> = match attribute {
            Some(reason) => Some(reason),
            None => msg_send![event, paramDescriptorForKeyword: QUIT_REASON],
        };
        let Some(reason) = reason else {
            return false;
        };
        let code: u32 = msg_send![&reason, typeCodeValue];
        SESSION_ENDS.contains(&code)
    }
}

/// Brings the app to the front for its question, as a click of its Dock icon does: a hidden
/// app shows its window again, and the app takes the focus from the app in front. The logic of
/// the window brings back a minimized window.
fn come_to_front(app: &AnyObject) {
    let none: Option<&AnyObject> = None;
    // SAFETY: `isHidden` gives a `BOOL`, `unhide:` takes a sender or nil, and
    // `activateIgnoringOtherApps:` takes a `BOOL`. AppKit calls the answer on the main thread.
    unsafe {
        let hidden: Bool = msg_send![app, isHidden];
        if hidden.as_bool() {
            let () = msg_send![app, unhide: none];
        }
        let () = msg_send![app, activateIgnoringOtherApps: Bool::YES];
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use objc2::ClassType as _;
    use objc2::runtime::{ClassBuilder, NSObject};

    use super::*;

    /// Where a quit event carries its reason: the system sets an attribute, and the check
    /// reads a parameter too.
    enum Reason {
        Attribute(&'static [u8; 4]),
        Parameter(&'static [u8; 4]),
    }

    /// An Apple Event as the Dock, a script or the system sends it.
    fn event(id: u32, reason: Option<Reason>) -> Retained<AnyObject> {
        let class = AnyClass::get(c"NSAppleEventDescriptor").expect("Foundation");
        let none: Option<&AnyObject> = None;
        // SAFETY: the class methods give a new descriptor: an Apple Event of a class, an ID,
        // no target, a new return ID (-1) and no transaction (0), or a four character code.
        // The setters take a descriptor and a key.
        unsafe {
            let event: Option<Retained<AnyObject>> = msg_send![
                class,
                appleEventWithEventClass: CORE_EVENT,
                eventID: id,
                targetDescriptor: none,
                returnID: -1_i16,
                transactionID: 0_i32
            ];
            let event = event.expect("an Apple Event");
            match reason {
                Some(Reason::Attribute(text)) => {
                    let reason: Retained<AnyObject> = msg_send![class, descriptorWithTypeCode: code(*text)];
                    let () = msg_send![&event, setAttributeDescriptor: &*reason, forKeyword: QUIT_REASON];
                }
                Some(Reason::Parameter(text)) => {
                    let reason: Retained<AnyObject> = msg_send![class, descriptorWithEnumCode: code(*text)];
                    let () = msg_send![&event, setParamDescriptor: &*reason, forKeyword: QUIT_REASON];
                }
                None => {}
            }
            event
        }
    }

    #[test]
    fn only_the_quit_of_a_log_out_a_restart_or_a_shut_down_ends_the_session() {
        for text in [b"rlgo", b"rest", b"shut", b"quia", b"logo", b"rrst", b"rsdn"] {
            assert!(ends_the_session(Some(&event(QUIT_EVENT, Some(Reason::Attribute(text))))), "{text:?}");
            assert!(ends_the_session(Some(&event(QUIT_EVENT, Some(Reason::Parameter(text))))), "{text:?}");
        }
        // The Quit item of the Dock and a script send a quit with no reason, and the Quit item
        // of the menu sends no event: the question opens for them.
        assert!(!ends_the_session(Some(&event(QUIT_EVENT, None))));
        assert!(!ends_the_session(None));
        assert!(!ends_the_session(Some(&event(QUIT_EVENT, Some(Reason::Attribute(b"yes "))))));
        let open = code(*b"odoc");
        assert!(!ends_the_session(Some(&event(open, Some(Reason::Attribute(b"shut"))))), "not a quit");
    }

    /// A class that stands in for the delegate of winit: a test opens no window.
    fn delegate(name: &CStr) -> Retained<AnyObject> {
        let class = ClassBuilder::new(name, NSObject::class()).expect("a new class").register();
        add_answer(class).expect("the answer");
        // SAFETY: `new` gives a new object of the class.
        unsafe { msg_send![class, new] }
    }

    /// A quit, as `terminate:` asks the delegate, with no app as the sender.
    fn quit(delegate: &AnyObject) -> usize {
        let none: Option<&AnyObject> = None;
        // SAFETY: the class has the answer with the types of `TYPES`.
        unsafe { msg_send![delegate, applicationShouldTerminate: none] }
    }

    #[test]
    fn a_quit_gets_the_decision_of_the_window_while_its_answer_lives() {
        let delegate = delegate(c"FresnelTestDelegate");
        assert_eq!(quit(&delegate), NOW, "no window yet: the app ends");
        let (decision, asked) = (Rc::new(Cell::new(Quit::Ask)), Rc::new(Cell::new(0)));
        let answer = QuitAnswer::new({
            let (decision, asked) = (Rc::clone(&decision), Rc::clone(&asked));
            move || {
                asked.set(asked.get() + 1);
                decision.get()
            }
        });
        assert_eq!(quit(&delegate), CANCEL, "the question opens, and the app stays");
        decision.set(Quit::Now);
        assert_eq!(quit(&delegate), NOW, "nothing to end, or the choice Quit");
        assert_eq!(asked.get(), 2);
        drop(answer);
        assert_eq!(quit(&delegate), NOW, "the window ended");
        assert_eq!(asked.get(), 2);
    }

    /// A newer winit can answer itself; the program then keeps that answer and says so.
    #[test]
    fn a_delegate_with_an_answer_of_its_own_keeps_it() {
        // Raw pointers: the class builder takes no function that is generic over lifetimes.
        extern "C" fn own(_delegate: *mut AnyObject, _selector: Sel, _app: *mut AnyObject) -> usize {
            NOW
        }
        let mut builder = ClassBuilder::new(c"FresnelTestOwnDelegate", NSObject::class()).expect("a new class");
        let own: extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize = own;
        // SAFETY: the function has the types of the answer.
        unsafe { builder.add_method(sel!(applicationShouldTerminate:), own) };
        let error = add_answer(builder.register()).expect_err("one answer");
        assert_eq!(error, "FresnelTestOwnDelegate answers applicationShouldTerminate: itself");
    }
}
