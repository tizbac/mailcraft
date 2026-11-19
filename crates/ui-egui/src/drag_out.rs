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

/// Whether a message drag has just left the window, and if so, hands it to the system.
pub(crate) fn escalate(app: &mut MailApp, ctx: &egui::Context) {
    let Some(drag) = egui::DragAndDrop::payload::<MessageDrag>(ctx) else { return };
    // The pointer is outside the window with the button still held: the owner is dragging out.
    let left = ctx.input(|i| i.pointer.is_decidedly_dragging() && i.pointer.hover_pos().is_none());
    if !left {
        return;
    }
    let ids: Vec<u64> = drag.0.iter().copied().take(MOST).collect();
    egui::DragAndDrop::clear_payload(ctx);
    if ids.is_empty() {
        return;
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
    if let Some(start) = app.services.drag_out.as_ref() {
        // The system's drag runs until the owner lets go or cancels; the window waits with it.
        if start(&files).is_err() {
            log::info!("dragging files out isn't served here");
        }
    }
    // What the drop target copied is its own; this is only the original.
    let _ = std::fs::remove_dir_all(&folder);
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
