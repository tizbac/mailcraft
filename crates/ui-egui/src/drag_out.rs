// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dragging a message out of the list to the desktop: when the drag leaves the window, the
//! message is written to a scratch file named from its subject (the format File › Options
//! chooses, `.eml` or `.msg`), and the system's own drag takes it from there — Explorer on
//! Windows, a file manager on X11. The file is taken away when the drag ends. An attachment's
//! chip carries its file the same way (#126).

use serde_json::json;

use crate::MailApp;
use crate::mail_view::{AttachmentDrag, MessageDrag};

/// The most messages one drag carries: a selection larger than this drags its first few.
const MOST: usize = 20;

/// The files of the last drag, taken away when the next one starts (the drop target may still be
/// copying them when the drag itself is over), or when the program quits.
static LAST: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Whether a message drag or an attachment drag has just left the window, and if so, hands it to
/// the system.
pub(crate) fn escalate(app: &mut MailApp, ctx: &egui::Context) {
    let messages = egui::DragAndDrop::payload::<MessageDrag>(ctx);
    let attachment = egui::DragAndDrop::payload::<AttachmentDrag>(ctx).map(|drag| (drag.id, drag.index));
    if messages.is_none() && attachment.is_none() {
        return;
    }
    // A frame now and then while it is held: on Wayland an implicit grab keeps sending motion
    // even outside the window (so no "pointer gone" arrives), and the frame that sees the
    // pointer outside must still come.
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    if !ctx.input(outside) {
        return;
    }
    log::info!(
        "drag: the pointer left the window; handing over {}",
        match attachment {
            Some((id, index)) => format!("attachment {index} of message {id}"),
            None => format!("{} message(s)", messages.as_ref().map_or(0, |drag| drag.0.len().min(MOST))),
        }
    );
    egui::DragAndDrop::clear_payload(ctx);
    // The files of the drag before this one have been copied by now.
    if let Some(previous) = LAST.lock().ok().and_then(|mut last| last.take()) {
        let _ = std::fs::remove_dir_all(previous);
    }
    let Some(folder) = scratch() else { return };
    let files = match attachment {
        Some((id, index)) => attachment_file(app, id, index, &folder).into_iter().collect(),
        None => message_files(app, messages.as_ref().map(|drag| drag.0.as_slice()).unwrap_or_default(), &folder),
    };
    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&folder);
        return;
    }
    start(app, files, folder);
}

/// The messages written into `folder`, as File › Options chooses (`.eml` or `.msg`), named from
/// their subjects. Those that can't be written are left out.
fn message_files(app: &mut MailApp, ids: &[u64], folder: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for id in ids.iter().copied().take(MOST) {
        let Some(subject) = app.session.mailbox.message(id).map(|message| message.subject.clone()) else { continue };
        let name = mailcraft_engine::opening::drag_name(&subject, app.session.prefs.save_message_format);
        let file = unique(folder, &name);
        if app.execute("file.exportMessage", json!({"id": id, "path": file})).is_ok() {
            files.push(file);
        }
    }
    files
}

/// Attachment `index` of message `id` written into `folder`, named as Save As names it. Its bytes
/// go through `mail.saveAttachment`, so a file the mailbox keeps on disk is read from there, and
/// the file is marked as from the Internet as every save of a sender's bytes is.
fn attachment_file(app: &mut MailApp, id: u64, index: usize, folder: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = app.session.mailbox.message(id).and_then(|m| m.attachments.get(index)).map(|a| mailcraft_engine::opening::save_name(&a.name))?;
    let file = unique(folder, &name);
    app.execute("mail.saveAttachment", json!({"id": id, "index": index, "path": file})).ok().map(|_| file)
}

/// Hands the files to the system's own drag. The scratch folder stays until the next drag (the
/// drop target may still be copying them), and is taken away now when the drag never started.
fn start(app: &mut MailApp, files: Vec<std::path::PathBuf>, folder: std::path::PathBuf) {
    let started = app.services.drag_out.as_ref().is_some_and(|start| {
        // X11 and Windows wait here until the drag is over; Wayland only starts it.
        match start(&files) {
            Ok(()) => true,
            Err(why) => {
                log::warn!("dragging files out failed: {why}");
                false
            }
        }
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
    // (The clock the browser build has too; the count keeps two drags in one second apart.)
    static DRAGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let drag = DRAGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!("mailcraft-drag-{}-{}-{drag}", std::process::id(), mailcraft_time::now()));
    std::fs::create_dir_all(&folder).ok().map(|()| folder)
}

#[cfg(test)]
#[path = "drag_out_tests.rs"]
mod drag_out_tests;
