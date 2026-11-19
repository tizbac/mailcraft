// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! When a message drag counts as out of the window: the pointer gone (X11) or its last position
//! past the window's edge (Wayland and Windows, where a grab keeps sending motion).

use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

use super::outside;

fn frame(ctx: &Context, events: Vec<Event>) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |_| {});
    output.textures_delta.clear();
}

fn press(at: Pos2) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }
}

#[test]
fn a_drag_whose_pointer_leaves_the_window_is_out() {
    let ctx = Context::default();
    frame(&ctx, vec![Event::PointerMoved(Pos2::new(20.0, 20.0)), press(Pos2::new(20.0, 20.0))]);
    // Far enough to be a drag, still inside.
    frame(&ctx, vec![Event::PointerMoved(Pos2::new(120.0, 120.0))]);
    assert!(!ctx.input(outside), "inside the window it is not out");
    // Past the edge: outside.
    frame(&ctx, vec![Event::PointerMoved(Pos2::new(900.0, 120.0))]);
    assert!(ctx.input(outside), "past the edge it is out");
}

#[test]
fn a_drag_whose_pointer_is_gone_is_out() {
    let ctx = Context::default();
    frame(&ctx, vec![Event::PointerMoved(Pos2::new(20.0, 20.0)), press(Pos2::new(20.0, 20.0))]);
    frame(&ctx, vec![Event::PointerMoved(Pos2::new(120.0, 120.0))]);
    frame(&ctx, vec![Event::PointerGone]);
    assert!(ctx.input(outside), "gone is out");
}

#[test]
fn a_click_without_a_drag_is_not_out() {
    let ctx = Context::default();
    frame(&ctx, vec![Event::PointerMoved(Pos2::new(20.0, 20.0)), press(Pos2::new(20.0, 20.0))]);
    frame(&ctx, vec![Event::PointerGone]);
    assert!(!ctx.input(outside), "a press that never moved is no drag");
}
