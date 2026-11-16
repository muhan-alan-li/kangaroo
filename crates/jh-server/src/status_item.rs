//! The macOS menu-bar status item, written directly against AppKit.

use anyhow::Result;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEventModifierFlags, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::NSString;
use std::cell::RefCell;

/// The message a menu click sends to the server's control thread.
#[derive(Clone, Copy)]
pub enum MenuAction {
    Open,
    ScanNow,
    Settings,
    Quit,
}

struct StatusTargetIvars {
    tx: flume::Sender<MenuAction>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "JHServerStatusTarget"]
    #[ivars = StatusTargetIvars]
    struct StatusTarget;

    impl StatusTarget {
        #[unsafe(method(openDashboard:))]
        fn open_dashboard(&self, _sender: *mut AnyObject) {
            let _ = self.ivars().tx.send(MenuAction::Open);
        }

        #[unsafe(method(scanNow:))]
        fn scan_now(&self, _sender: *mut AnyObject) {
            let _ = self.ivars().tx.send(MenuAction::ScanNow);
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: *mut AnyObject) {
            let _ = self.ivars().tx.send(MenuAction::Settings);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: *mut AnyObject) {
            let _ = self.ivars().tx.send(MenuAction::Quit);
        }
    }
);

fn make_target(tx: flume::Sender<MenuAction>, mtm: MainThreadMarker) -> Retained<StatusTarget> {
    let this = StatusTarget::alloc(mtm).set_ivars(StatusTargetIvars { tx });
    unsafe { msg_send![super(this), init] }
}

// AppKit retains neither the status item nor its menu target. Keeping both in
// thread-local storage makes them live for the process lifetime.
thread_local! {
    static STATUS_ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    static STATUS_TARGET: RefCell<Option<Retained<StatusTarget>>> = const { RefCell::new(None) };
}

/// Install the status item on the main thread.
pub fn install(tx: flume::Sender<MenuAction>) {
    let mtm = MainThreadMarker::new().expect("status item installs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let target = make_target(tx, mtm);
    let menu = NSMenu::new(mtm);
    for (title, selector, key) in [
        ("Open Dashboard", sel!(openDashboard:), ""),
        ("Scan now", sel!(scanNow:), "r"),
        ("Settings", sel!(openSettings:), ","),
        ("Quit", sel!(quit:), "q"),
    ] {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(selector),
                &NSString::from_str(key),
            )
        };
        if !key.is_empty() {
            item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
        }
        // SAFETY: the target object implements every selector above and is kept
        // alive for the process lifetime in STATUS_TARGET.
        unsafe {
            item.setTarget(Some(target.as_super().as_super()));
        }
        menu.addItem(&item);
    }

    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = status_item.button(mtm) {
        button.setTitle(&NSString::from_str("Job Hunt"));
    }
    status_item.setMenu(Some(&menu));

    STATUS_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
    STATUS_ITEM.with(|slot| *slot.borrow_mut() = Some(status_item));
}

/// Enter the AppKit run loop. Blocks until the process exits.
pub fn run_app() -> Result<()> {
    let mtm = MainThreadMarker::new().expect("the app runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.run();
    Ok(())
}
