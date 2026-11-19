// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dragging files out of the window to the desktop: an Explorer folder on Windows, a file
//! manager on Linux (X11's XDND).
//!
//! The owner drags a message out of the list; the window makes the file (see the window's own
//! `drag_out`), then asks here to hand that file to the system. The drag runs on the calling
//! thread, as the platform's own drag loops do, and returns when it is over: dropped somewhere,
//! or cancelled. The file belongs to the caller, which removes it afterwards.
//!
//! **Unsafe, on purpose, here only.** Both platforms' drag-and-drop are C interfaces with
//! callbacks (OLE's `IDataObject` and `IDropSource` on Windows), which can't be spoken from safe
//! Rust. Every other crate forbids unsafe; this one is the door, kept as small as it can be.
//!
//! Wayland and macOS are not served yet: a Wayland drag source needs the input serial that only
//! the toolkit has, and macOS needs a view to start a dragging session from.
#![allow(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};

/// Hands `paths` to the system as a drag the owner is making now, and returns when it ends.
///
/// `Ok(())` means the system was asked and the drag is over (the owner may have dropped it
/// anywhere, or cancelled it). `Err` says why a drag could not be started at all.
pub fn start_drag(paths: &[PathBuf]) -> Result<(), String> {
    let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).filter(|path| !path.as_os_str().is_empty()).collect();
    if paths.is_empty() {
        return Err("nothing to drag".to_owned());
    }
    platform::start(&paths)
}

/// `file://` URIs of `paths`, one a line, as the X11 protocol carries them (`text/uri-list`).
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn uri_list(paths: &[&Path]) -> String {
    let mut list = String::new();
    for path in paths {
        list.push_str("file://");
        list.push_str(&file_uri_path(path));
        list.push_str("\r\n");
    }
    list
}

/// `path` as a URI's path: `/` kept, every byte a URI does not allow (space, `%`, non-ASCII, the
/// controls of a name) written as `%XX`, its UTF-8 bytes otherwise.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn file_uri_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.' | b'_' | b'~' | b'/' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@'
            );
        if keep {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0').to_ascii_uppercase());
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0').to_ascii_uppercase());
        }
    }
    // A path that is exactly `/` is already the URI's root.
    if out.is_empty() { "/".to_owned() } else { out }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod x11;

#[cfg(windows)]
mod windows_drag;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    pub(super) fn start(paths: &[&std::path::Path]) -> Result<(), String> {
        // On a Wayland session there is no X server to speak XDND to; the caller's window may
        // still be an X11 one (XWayland), which is what this serves.
        if std::env::var_os("DISPLAY").is_none() {
            return Err("dragging files out needs an X11 display".to_owned());
        }
        super::x11::start(paths)
    }
}

#[cfg(windows)]
mod platform {
    pub(super) fn start(paths: &[&std::path::Path]) -> Result<(), String> {
        super::windows_drag::start(paths)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd", windows)))]
mod platform {
    pub(super) fn start(_paths: &[&std::path::Path]) -> Result<(), String> {
        Err("dragging files out is not served on this system yet".to_owned())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
