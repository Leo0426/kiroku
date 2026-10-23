//! AppKit's Cmd+Q/menu/Dock Quit bypass Tauri's ExitRequested in the pinned runtime.
//! Keep Tao's delegate and add only its currently absent termination decision.

use objc2::{class, ffi, msg_send, runtime::{AnyObject, Imp, Sel}, sel, Encode, MainThreadMarker};
use std::{ffi::CString, sync::OnceLock};
use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn install(handle: &AppHandle) -> Result<(), String> {
    let _main = MainThreadMarker::new().ok_or("退出保护必须在主线程安装")?;
    // SAFETY: setup runs on the main thread after Tao creates NSApplication and
    // its delegate. Neither object is replaced or released by this module.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let app = app.as_ref().ok_or("无法取得 macOS 应用对象")?;
        let delegate: *mut AnyObject = msg_send![app, delegate];
        let class = delegate.as_ref().ok_or("无法取得 macOS 应用代理")?.class();
        let selector = sel!(applicationShouldTerminate:);
        // class_addMethod would override an inherited method. Refuse both local
        // and inherited implementations so a runtime upgrade cannot silently
        // change the shutdown contract.
        if class.instance_method(selector).is_some() {
            return Err("macOS 退出代理已存在，请检查运行时兼容性".into());
        }
        APP.set(handle.clone()).map_err(|_| "macOS 退出保护已安装")?;
        // NSApplicationTerminateReply is NSUInteger, with self/_cmd/sender args.
        let encoding = CString::new(format!("{}@:@", usize::ENCODING)).map_err(|e| e.to_string())?;
        let callback: Imp = std::mem::transmute(
            should_terminate as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
        );
        if !ffi::class_addMethod(class as *const _ as *mut _, selector, callback, encoding.as_ptr()).as_bool() {
            return Err("无法安装 macOS 退出保护".into());
        }
    }
    Ok(())
}

unsafe extern "C-unwind" fn should_terminate(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> usize {
    // NSTerminateCancel ends this Cocoa request; our existing waiter starts a
    // fresh Tauri exit after the guard releases. Unlike NSTerminateLater, this
    // does not enter AppKit's modal termination loop. During OS logout/restart,
    // Cancel also cancels that system request; it is not a transparent delay.
    match APP.get() {
        Some(handle) if !crate::defer_shutdown(handle, 0) => 1, // NSTerminateNow
        _ => 0, // NSTerminateCancel
    }
}
