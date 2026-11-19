// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dragging a message out of the list to the desktop: when the drag leaves the window, the
//! message is written to a scratch file named from its subject (the format File › Options
//! chooses, `.eml` or `.msg`), and the system's own drag takes it from there — Explorer on
//! Windows, a file manager on X11. The file is taken away when the drag ends.

use serde_json::json;

use crate::MailApp;
use crate::mail_view::MessageDrag;

/// The most messages one drag carries: a selection larger than this drags its first few.
const MOST: usize = 20;

/// The files of the last drag, taken away when the next one starts (the drop target may still be
/// copying them when the drag itself is over), or when the program quits.
static LAST: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Whether a message drag has just left the window, and if so, hands it to the system.
pub(crate) fn escalate(app: &mut MailApp, ctx: &egui::Context) {
    let Some(drag) = egui::DragAndDrop::payload::<MessageDrag>(ctx) else { return };
    // A frame now and then while the messages are held: on Wayland an implicit grab keeps
    // sending motion even outside the window (so no "pointer gone" arrives), and the frame that
    // sees the pointer outside must still come.
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    if !ctx.input(outside) {
        return;
    }
    let ids: Vec<u64> = drag.0.iter().copied().take(MOST).collect();
    log::info!("drag: the pointer left the window; handing {} message(s) over", ids.len());
    egui::DragAndDrop::clear_payload(ctx);
    if ids.is_empty() {
        return;
    }
    // The files of the drag before this one have been copied by now.
    if let Some(previous) = LAST.lock().ok().and_then(|mut last| last.take()) {
        let _ = std::fs::remove_dir_all(previous);
    }
    let Some(folder) = scratch() else { return };
    let mut files = Vec::new();
    for id in ids {
        let Some(subject) = app.session.mailbox.message(id).map(|message| message.subject.clone()) else { continue };
        let name = mailcraft_engine::opening::drag_name(&subject, app.session.prefs.save_message_format);
        let file = unique(&folder, &name);
        if app.execute("file.exportMessage", json!({"id": id, "path": file})).is_ok() {
            files.push(file);
        }
    }
    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&folder);
        return;
    }
    let started = app.services.drag_out.as_ref().is_some_and(|start| {
        // X11 and Windows wait here until the drag is over; Wayland only starts it.
        let served = start(&files).is_ok();
        if !served {
            log::info!("dragging files out isn't served here");
        }
        served
    });
    if started {
        if let Ok(mut last) = LAST.lock() {
            *last = Some(folder);
        }
    } else {
        let _ = std::fs::remove_dir_all(&folder);
    }
}

/// Whether a drag is under way with its pointer outside the window: gone (X11), or its last
/// position past the window's edge (Wayland and Windows, where a grab keeps sending motion
/// outside).
fn outside(i: &egui::InputState) -> bool {
    i.pointer.is_decidedly_dragging() && (i.pointer.hover_pos().is_none() || i.pointer.latest_pos().is_some_and(|at| !i.viewport_rect().contains(at)))
}

/// Takes away the files of the last drag, if any (at quit).
pub fn clear_scratch() {
    if let Some(last) = LAST.lock().ok().and_then(|mut last| last.take()) {
        let _ = std::fs::remove_dir_all(last);
    }
}

/// `dir/name`, or `dir/name (2)`, … when two messages of the drag share a subject.
fn unique(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    for count in 2..1000 {
        let named = if extension.is_empty() { format!("{stem} ({count})") } else { format!("{stem} ({count}).{extension}") };
        let path = dir.join(named);
        if !path.exists() {
            return path;
        }
    }
    first
}

/// A folder of this run's own under the system's temporary folder, for the files a drag carries.
fn scratch() -> Option<std::path::PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let folder = std::env::temp_dir().join(format!("mailcraft-drag-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&folder).ok().map(|()| folder)
}

#[cfg(test)]
#[path = "drag_out_tests.rs"]
mod drag_out_tests;
