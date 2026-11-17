//! The Kangaroo mark, as macOS shows it outside the window.
//!
//! The binaries are plain executables, not an `.app` bundle, so the dock tile
//! comes from `NSApplication` rather than from a `CFBundleIconFile`. The image
//! is the same artwork the menu bar uses; `assets/make-icons.py` draws both.

/// The dock tile: the kangaroo on the warm tile, 1024 pixels square.
#[cfg(target_os = "macos")]
const APP_ICON: &[u8] = include_bytes!("../../../assets/kangaroo-app.png");

/// Put the Kangaroo mark on the dock tile. Call it on the main thread.
#[cfg(target_os = "macos")]
pub fn install_dock_icon() {
    use objc2::AnyThread as _;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{NSData, NSSize};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(APP_ICON);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    image.setSize(NSSize::new(512.0, 512.0));
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: the main thread owns the shared application, and `image` outlives
    // the call; AppKit takes its own reference.
    unsafe {
        app.setApplicationIconImage(Some(&image));
    }
}

/// Other platforms draw their own icon, if any.
#[cfg(not(target_os = "macos"))]
pub fn install_dock_icon() {}
