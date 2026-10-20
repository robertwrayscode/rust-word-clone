//! macOS: receive documents opened from Finder (double-click, "Open With", drag onto the
//! Dock icon). AppKit sends these to the application delegate's `application:openURLs:`,
//! which winit's delegate does not implement, so we add that method to NSObject (the
//! delegate's superclass) before the event loop starts.

use std::path::PathBuf;
use std::sync::Mutex;

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

/// Paths received since the last call.
pub fn take() -> Vec<PathBuf> {
    PENDING.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// Remember the egui context so a file opened while running wakes the UI.
pub fn set_context(ctx: &egui::Context) {
    if let Ok(mut c) = CTX.lock() {
        *c = Some(ctx.clone());
    }
}

#[cfg(target_os = "macos")]
pub fn install() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{msg_send, sel};
    use std::ffi::{c_char, CStr};

    unsafe extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *mut AnyObject) {
        if urls.is_null() {
            return;
        }
        let mut paths = Vec::new();
        unsafe {
            let n: usize = msg_send![urls, count];
            for i in 0..n {
                let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                if url.is_null() {
                    continue;
                }
                let path: *mut AnyObject = msg_send![url, path];
                if path.is_null() {
                    continue;
                }
                let s: *const c_char = msg_send![path, UTF8String];
                if !s.is_null() {
                    paths.push(PathBuf::from(CStr::from_ptr(s).to_string_lossy().into_owned()));
                }
            }
        }
        if let Ok(mut v) = PENDING.lock() {
            v.extend(paths);
        }
        if let Ok(c) = CTX.lock() {
            if let Some(ctx) = c.as_ref() {
                ctx.request_repaint();
            }
        }
    }

    unsafe {
        let Some(cls) = AnyClass::get(c"NSObject") else { return };
        let imp: Imp = std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject), Imp>(open_urls);
        objc2::ffi::class_addMethod(cls as *const AnyClass as *mut AnyClass, sel!(application:openURLs:), imp, c"v@:@@".as_ptr());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
