// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Windows' drag-out through the `drag` crate: it builds the shell's own data object over the
//! files and runs OLE's `DoDragDrop`, which is what Explorer accepts (a hand-made `CF_HDROP`
//! object got the no-drop cursor). The window it asks for is remembered here at startup.

use std::num::NonZeroIsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};

/// The window the app runs in, as Windows names it; `drag` only checks that it is a Win32 one.
static WINDOW: Mutex<Option<isize>> = Mutex::new(None);

/// Remembers the window a drag starts from (called once, when the window is made).
pub(super) fn set_window(hwnd: isize) {
    if let Ok(mut window) = WINDOW.lock() {
        *window = Some(hwnd);
    }
}

struct Window(isize);

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
        // Safety: it is the window this program runs in, and it outlives the drag.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd))) })
    }
}

pub(super) fn start(paths: &[&Path]) -> Result<(), String> {
    let hwnd = WINDOW.lock().ok().and_then(|window| *window).ok_or("the window isn't known yet")?;
    let files: Vec<PathBuf> = paths.iter().map(|path| path.to_path_buf()).collect();
    // The picture beside the pointer: the program's own icon, at the small size the shell draws
    // it at — a 256 px one comes out as a huge icon following the pointer.
    let icon = drag::Image::Raw(crate::DRAG_ICON.to_vec());
    let window = Window(hwnd);
    drag::start_drag(&window, drag::DragItem::Files(files), icon, |_result, _at| {}, drag::Options::default()).map_err(|why| why.to_string())
}
