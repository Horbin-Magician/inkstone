//! Set the Dock icon even when launched as a standalone executable by Cargo.
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

pub fn init() {
    // GPUI invokes its application callback on the AppKit main thread.
    let mtm = MainThreadMarker::new().expect("application icon requires the main thread");
    let data = NSData::with_bytes(include_bytes!("../assets/inkstone-icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        eprintln!("无法解码应用图标");
        return;
    };
    // SAFETY: AppKit is initialized and this call runs on its main thread.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}
