// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! X11's XDND, as the source: the window owns `XdndSelection` with a `text/uri-list` of the
//! files, tells the window under the pointer that a drag has begun, follows the pointer, and on
//! release drops and serves the list to whoever asks for it.
//!
//! A connection of its own, so the toolkit's event loop is never touched: the source window is a
//! one-pixel window of this connection that only owns the selection, and the pointer is asked
//! where it is rather than listened to. The drag runs on the calling thread and ends when the
//! button is let go (or the window goes away).

use std::path::Path;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent, Window, WindowClass,
};
use x11rb::{COPY_FROM_PARENT, NONE};

/// How long to wait for the target to answer once the drop was made (it asks for the list then).
const AFTER_DROP: Duration = Duration::from_secs(3);
/// How often the pointer is looked at, and how often a position is sent at most.
const POLL: Duration = Duration::from_millis(10);
const POSITION_EVERY: Duration = Duration::from_millis(50);

struct Atoms {
    aware: Atom,
    selection: Atom,
    enter: Atom,
    position: Atom,
    drop: Atom,
    finished: Atom,
    leave: Atom,
    uri_list: Atom,
}

pub(super) fn start(paths: &[&Path]) -> Result<(), String> {
    let list = super::uri_list(paths);
    let (conn, screen_num) = x11rb::connect(None).map_err(|why| why.to_string())?;
    let screen = conn.setup().roots.get(screen_num).ok_or("the display has no screen")?;
    let root = screen.root;
    let atoms = Atoms {
        aware: intern(&conn, b"XdndAware")?,
        selection: intern(&conn, b"XdndSelection")?,
        enter: intern(&conn, b"XdndEnter")?,
        position: intern(&conn, b"XdndPosition")?,
        drop: intern(&conn, b"XdndDrop")?,
        finished: intern(&conn, b"XdndFinished")?,
        leave: intern(&conn, b"XdndLeave")?,
        uri_list: intern(&conn, b"text/uri-list")?,
    };
    // The source: a window of this connection that only owns the selection.
    let source: Window = conn.generate_id().map_err(|why| why.to_string())?;
    let aux = CreateWindowAux::new().override_redirect(1).event_mask(EventMask::NO_EVENT);
    conn.create_window(COPY_FROM_PARENT as u8, source, root, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, COPY_FROM_PARENT, &aux)
        .map_err(|why| why.to_string())?;
    conn.set_selection_owner(source, atoms.selection, x11rb::CURRENT_TIME).map_err(|why| why.to_string())?;
    conn.flush().map_err(|why| why.to_string())?;

    let mut target: Option<Window> = None;
    let mut last_position = Instant::now() - POSITION_EVERY;
    loop {
        // Selection requests can come at any time; serve them.
        while let Some(event) = conn.poll_for_event().map_err(|why| why.to_string())? {
            serve(&conn, &atoms, &list, event)?;
        }
        let pointer = conn.query_pointer(root).map_err(|why| why.to_string())?.reply().map_err(|why| why.to_string())?;
        let held = pointer.mask.contains(x11rb::protocol::xproto::KeyButMask::BUTTON1);
        if !held {
            // Let go: drop on the window under the pointer, if it takes drops, then serve the
            // list it asks for.
            let under = target_at(&conn, &atoms, root)?;
            if let Some(window) = under.or(target) {
                send(&conn, window, atoms.drop, [x11rb::CURRENT_TIME, 0, 0, 0, 0])?;
                conn.flush().map_err(|why| why.to_string())?;
                let deadline = Instant::now() + AFTER_DROP;
                while Instant::now() < deadline {
                    let mut finished = false;
                    while let Some(event) = conn.poll_for_event().map_err(|why| why.to_string())? {
                        if let Event::ClientMessage(message) = &event
                            && message.type_ == atoms.finished
                        {
                            finished = true;
                        }
                        serve(&conn, &atoms, &list, event)?;
                    }
                    if finished {
                        break;
                    }
                    std::thread::sleep(POLL);
                }
            }
            return Ok(());
        }
        let under = target_at(&conn, &atoms, root)?;
        if under != target {
            if let Some(old) = target {
                send(&conn, old, atoms.leave, [0, 0, 0, 0, 0])?;
            }
            if let Some(new) = under {
                send(&conn, new, atoms.enter, [5 << 24, atoms.uri_list, 0, 0, source])?;
            }
            target = under;
            last_position = Instant::now() - POSITION_EVERY;
        }
        if let Some(window) = target
            && last_position.elapsed() >= POSITION_EVERY
        {
            let x = u32::from(u16::try_from(pointer.root_x).unwrap_or(0));
            let y = u32::from(u16::try_from(pointer.root_y).unwrap_or(0));
            send(&conn, window, atoms.position, [0, x, y, x11rb::CURRENT_TIME, source])?;
            last_position = Instant::now();
        }
        conn.flush().map_err(|why| why.to_string())?;
        std::thread::sleep(POLL);
    }
}

/// The deepest window under the pointer that says it takes drops (`XdndAware`), from the root
/// down: toolkits put the property on the window that is the target, sometimes a child.
fn target_at(conn: &impl Connection, atoms: &Atoms, root: Window) -> Result<Option<Window>, String> {
    let mut window = root;
    loop {
        let pointer = conn.query_pointer(window).map_err(|why| why.to_string())?.reply().map_err(|why| why.to_string())?;
        if pointer.child == NONE {
            return Ok(None);
        }
        window = pointer.child;
        if aware(conn, atoms, window)? {
            return Ok(Some(window));
        }
    }
}

fn aware(conn: &impl Connection, atoms: &Atoms, window: Window) -> Result<bool, String> {
    let reply =
        conn.get_property(false, window, atoms.aware, AtomEnum::ATOM, 0, 1).map_err(|why| why.to_string())?.reply().map_err(|why| why.to_string())?;
    Ok(reply.value32().and_then(|mut values| values.next()).is_some_and(|version| version >= 1))
}

fn send(conn: &impl Connection, window: Window, kind: Atom, data: [u32; 5]) -> Result<(), String> {
    conn.send_event(false, window, EventMask::NO_EVENT, ClientMessageEvent::new(32, window, kind, data)).map_err(|why| why.to_string())?;
    Ok(())
}

/// Answer a selection request: the target asks the source for the list, the source puts it in the
/// target's property and says so.
fn serve(conn: &impl Connection, atoms: &Atoms, list: &str, event: Event) -> Result<(), String> {
    let Event::SelectionRequest(request) = event else { return Ok(()) };
    if request.selection != atoms.selection {
        return Ok(());
    }
    let property = if request.property == x11rb::NONE { request.target } else { request.property };
    let bytes = list.as_bytes();
    let answered =
        conn.change_property(PropMode::REPLACE, request.requestor, property, request.target, 8, bytes.len() as u32, bytes).and_then(|_| {
            conn.send_event(
                false,
                request.requestor,
                EventMask::NO_EVENT,
                SelectionNotifyEvent {
                    response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property,
                },
            )
        });
    answered.map_err(|why| why.to_string())?;
    Ok(())
}

fn intern(conn: &impl Connection, name: &[u8]) -> Result<Atom, String> {
    conn.intern_atom(false, name).map_err(|why| why.to_string())?.reply().map(|reply| reply.atom).map_err(|why| why.to_string())
}
