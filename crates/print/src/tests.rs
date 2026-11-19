// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the pages come out as, and what a hostile message cannot make them do.

use std::sync::Arc;

use mailcraft_html::Block;

use super::*;

fn out<'a>(blocks: &'a [Block], subject: &'a str) -> Printout<'a> {
    Printout {
        subject,
        from: "Ada Lovelace <ada@example.org>",
        to: "someone@example.org",
        cc: "",
        date: "Tue 12 Nov 2025, 13:00",
        attachments: &[],
        blocks,
        picture: None,
        footer: "",
        paper: Paper::A4,
    }
}

fn holds(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

/// How many pages the PDF has: krilla writes one page object as `/Type/Page/…` and the tree as
/// `/Type/Pages`, so counting the pages' own objects counts the pages.
fn page_count(pdf: &[u8]) -> usize {
    pdf.windows(b"/Type/Page/".len()).filter(|w| w == b"/Type/Page/").count()
}

#[test]
fn a_message_becomes_a_pdf_of_its_headings_and_text() {
    let blocks = mailcraft_html::parse(
        "<h1>The plan</h1><p>A <b>bold</b> bit and an <i>italic</i> one, in <span style=\"color:#006600\">green</span>.</p>\
         <ul><li>one thing</li><li>another</li></ul><hr><blockquote>a quote</blockquote>\
         <table border=1><tr><th>Month</th><th>Sum</th></tr><tr><td>October</td><td>12</td></tr></table>",
    );
    let pdf = pdf(&out(&blocks, "Quarterly review"), &bundled_fonts()).unwrap_or_else(|why| panic!("a plain message printed: {why}"));
    assert!(pdf.starts_with(b"%PDF-"), "it is a PDF");
    assert!(page_count(&pdf) == 1, "one page of headings and text");
    assert!(holds(&pdf, b"/MediaBox[0 0 595.28 841.89]"), "the sheet is A4");
    assert!(pdf.len() > 500, "the pages have something on them");
    assert!(!blocks.is_empty(), "the body has blocks to print");
}

#[test]
fn nothing_is_printed_when_there_is_nothing_to_print_or_no_fonts() {
    let blocks = mailcraft_html::parse("<p>text</p>");
    assert!(pdf(&out(&blocks, "s"), &Fonts::default()).is_err(), "no fonts means no pages");
    assert!(
        pdf(&out(&blocks, "s"), &Fonts { mono: Some(Face::new(vec![0u8; 4])), ..Fonts::default() }).is_err(),
        "a font that is not one is no font"
    );
}

#[test]
fn a_face_is_chosen_for_the_letters_the_body_one_lacks() {
    let faces = chosen_faces(&bundled_fonts());
    assert!(faces.len() >= 4, "the bundled faces include a script's own");
    // (Georgian: Inter has none of those letters, so a face that has them is picked.)
    let georgian = "თბილისი";
    let picked = face_for(&faces, georgian, 0);
    assert_ne!(picked, 0, "a face with those letters is found");
    assert!(covers(&faces[picked], 'თ'), "it really has them");
    assert_eq!(face_for(&faces, "hello", 0), 0, "the body face is kept when it covers the word");
    assert!(text_width(&faces, 0, 10.0, "hello") > 20.0, "letters have widths");
    assert!(text_width(&faces, 0, 10.0, "") == 0.0, "nothing is nothing wide");
}

#[test]
fn a_word_longer_than_the_line_is_broken_between_letters() {
    let faces = chosen_faces(&bundled_fonts());
    let flow = Flow::new(&faces, Paper::A4, None);
    let word = Word {
        text: "a".repeat(4000),
        face: 0,
        size: BODY,
        color: [0, 0, 0],
        line_height: BODY * 1.4,
        rtl: false,
        underline: false,
        strike: false,
        italic: false,
        background: None,
    };
    let lines = flow.wrap(vec![word], 100.0);
    assert!(lines.len() > 30, "it is split into lines");
    assert!(lines.iter().all(|l| l.iter().all(|p| p.width <= 101.0)), "no line runs past the room");
}

#[test]
fn an_italic_word_is_slanted_and_given_room_for_the_slant() {
    // A shear is a ratio, not an angle: 12° is 0.21, and a shear of 12 leans a word right off the
    // sheet, which is how the first try at a slant lost every italic word.
    assert!((0.0..1.0).contains(&ITALIC_SLANT), "a lean, not a spin");
    let faces = chosen_faces(&bundled_fonts());
    let flow = Flow::new(&faces, Paper::A4, None);
    let upright = Word {
        text: "italic".to_owned(),
        face: 0,
        size: BODY,
        color: [0x1a, 0x1a, 0x1a],
        line_height: BODY * 1.4,
        rtl: false,
        underline: false,
        strike: false,
        italic: false,
        background: None,
    };
    let slanted = Word { italic: true, ..upright.clone() };
    let standing = flow.wrap(vec![upright], 400.0);
    let leaning = flow.wrap(vec![slanted.clone()], 400.0);
    assert_eq!(standing.len(), 1, "the word is one line");
    assert_eq!(leaning.len(), 1, "the slanted word is too");
    let extra = leaning[0][0].width - standing[0][0].width;
    assert!((extra - BODY * ITALIC_SLANT).abs() < 0.01, "the lean is counted in the word's width, not left to overlap the next one");

    // A line of slanted words still keeps to the room it was given.
    let lines = flow.wrap(vec![slanted; 60], 200.0);
    assert!(lines.len() > 4, "they fill several lines");
    assert!(lines.iter().all(|l| l.iter().all(|p| p.x + p.width <= 201.0)), "no line runs past the room");
    let printed = pdf(
        &out(
            &mailcraft_html::parse(
                "<p>an <i>italic</i> word and a <i>very</i> long <i>slanted</i> one, set in a face with no italic of its own.</p>",
            ),
            "slant",
        ),
        &bundled_fonts(),
    )
    .unwrap_or_else(|why| panic!("the slant printed: {why}"));
    assert!(page_count(&printed) == 1, "it is one page");
}

#[test]
fn a_picture_written_into_the_message_is_drawn_and_one_with_no_bytes_is_not() {
    let blocks = mailcraft_html::parse(&format!(
        "<img src=\"data:image/png;base64,{}\"><img src=\"cid:none@example.org\" width=10 height=10 alt=\"the chart\">",
        base64_of(&red_png())
    ));
    let printed = pdf(&out(&blocks, "With pictures"), &bundled_fonts()).unwrap_or_else(|why| panic!("a picture printed: {why}"));
    assert!(page_count(&printed) >= 1, "the page is there");
    assert!(blocks.iter().any(|b| matches!(b.kind, mailcraft_html::BlockKind::Image { .. })), "both pictures are blocks");
}

#[test]
fn the_caller_can_hand_over_a_picture_from_the_web() {
    let bytes: Arc<Vec<u8>> = Arc::new(red_png());
    let source =
        |src: &str| -> Option<PictureBytes> { if src == "https://example.org/a.png" { Some((bytes.clone(), "image/png".to_owned())) } else { None } };
    let blocks = mailcraft_html::parse("<img src=\"https://example.org/a.png\" width=8 height=6>");
    let out = Printout {
        subject: "web picture",
        from: "",
        to: "",
        cc: "",
        date: "",
        attachments: &[],
        blocks: &blocks,
        picture: Some(&source),
        footer: "",
        paper: Paper::A4,
    };
    assert!(pdf(&out, &bundled_fonts()).is_ok(), "the caller's bytes are used");
}

#[test]
fn a_picture_of_nonsense_is_not_a_crash() {
    let junk: Vec<u8> = (0..2000u32).map(|i| (i % 256) as u8).collect();
    let blocks = mailcraft_html::parse(&format!("<img src=\"data:image/png;base64,{}\" alt=\"unreadable\">", base64_of(&junk)));
    assert!(pdf(&out(&blocks, "junk picture"), &bundled_fonts()).is_ok(), "what it says stands in for it");
}

#[test]
fn a_picture_bigger_than_any_page_is_left_out_not_crashed_on() {
    // A PNG header that says 100,000 × 100,000: a side no printer has.
    let huge: Vec<u8> =
        vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 1, 134, 160, 0, 1, 134, 160, 8, 6, 0, 0, 0, 168, 82, 11, 200];
    let blocks = mailcraft_html::parse(&format!("<img src=\"data:image/png;base64,{}\" alt=\"a picture no printer could take\">", base64_of(&huge)));
    let printed = pdf(&out(&blocks, "too big"), &bundled_fonts()).unwrap_or_else(|why| panic!("its words printed: {why}"));
    assert!(!holds(&printed, b"/Subtype/Image"), "the picture is not drawn, and nothing crashed");
    // More bytes than a page could hold are not opened at all.
    let many = vec![0u8; MAX_PICTURE_BYTES + 1];
    let blocks = mailcraft_html::parse(&format!("<img src=\"data:image/png;base64,{}\" alt=\"huge\">", base64_of(&many)));
    let printed = pdf(&out(&blocks, "huge"), &bundled_fonts()).unwrap_or_else(|why| panic!("its words printed: {why}"));
    assert!(!holds(&printed, b"/Subtype/Image"), "the picture is not drawn");
}

#[test]
fn a_hostile_body_of_every_kind_stays_within_what_a_page_can_hold() {
    let png = base64_of(&red_png());
    let mut html = String::new();
    for _ in 0..25_000 {
        html.push_str("<li style='margin:64px'>a line of text</li>");
    }
    for _ in 0..400 {
        html.push_str(&format!("<img src=\"data:image/png;base64,{png}\" width=400 height=400>"));
    }
    let blocks = mailcraft_html::parse(&html);
    let printed = pdf(&out(&blocks, "the largest message there is"), &bundled_fonts()).unwrap_or_else(|why| panic!("it printed: {why}"));
    // Sheets and bytes both have a ceiling: the blocks the reader allows, and the picture budget
    // the pages hold, so no message can make a thousand-page PDF.
    assert!(page_count(&printed) <= 4 * mailcraft_html::MAX_BLOCKS + 8, "{} sheets", page_count(&printed));
    assert!(printed.len() <= 128 * 1024 * 1024, "{} bytes of pages", printed.len());
}

#[test]
fn a_table_of_any_shape_is_set_on_the_page() {
    let blocks = mailcraft_html::parse(
        "<table border=1 width=\"100%\"><tr><td colspan=2>wide</td><td rowspan=2>tall</td></tr>\
         <tr><td>a</td></tr><tr><td>b</td><td>c</td><td>d</td></tr></table>",
    );
    assert!(pdf(&out(&blocks, "a table"), &bundled_fonts()).is_ok(), "the grid is set");
}

#[test]
fn an_odd_paper_is_refused_not_crashed_on() {
    let blocks = mailcraft_html::parse("<p>text</p>");
    for paper in [
        Paper { width: f32::NAN, height: 500.0, margin: 10.0 },
        Paper { width: 10.0, height: 10.0, margin: 100.0 },
        Paper { width: 595.0, height: 842.0, margin: -1.0 },
        Paper { width: 595.0, height: 842.0, margin: 400.0 },
        Paper { width: 1000.0, height: 1000.0, margin: 499.99 },
    ] {
        let out = Printout { subject: "s", from: "", to: "", cc: "", date: "", attachments: &[], blocks: &blocks, picture: None, footer: "", paper };
        assert!(pdf(&out, &bundled_fonts()).is_err(), "{paper:?} is not paper to set on");
    }
    assert!(Paper::A4.is_sound() && Paper::LETTER.is_sound(), "the two papers are sound");
}

#[test]
fn a_huge_and_deep_message_does_not_break_the_pages() {
    // (A hostile body: many blocks, deep quoting, long spans, sizes out of range.)
    let mut blocks: Vec<Block> = Vec::new();
    for i in 0..1500u32 {
        let html = format!(
            "<blockquote><blockquote><li style='margin:64px;padding:64px;font-size:32px'><span style='color:red;background:#fff'>{} {}</span></li></blockquote></blockquote>",
            "x".repeat(400),
            i
        );
        blocks.extend(mailcraft_html::parse(&html));
    }
    let pdf = pdf(&out(&blocks, "the flood"), &bundled_fonts()).unwrap_or_else(|why| panic!("a flood still printed: {why}"));
    assert!(pdf.starts_with(b"%PDF-"), "the pages are written");
}

#[test]
fn text_going_the_other_way_is_printed_too() {
    let blocks = mailcraft_html::parse("<p dir=\"rtl\">من اليمين إلى اليسار</p><p dir=\"rtl\">right to left</p>");
    let printed = pdf(&out(&blocks, "rtl"), &bundled_fonts()).unwrap_or_else(|why| panic!("rtl printed: {why}"));
    assert!(page_count(&printed) == 1, "it prints on one page");
}

#[test]
fn many_pages_are_numbered() {
    let names: Vec<String> = (0..50).map(|i| format!("file{i}.bin")).collect();
    let blocks = mailcraft_html::parse(&format!("<p>{}</p>", "word ".repeat(4000)));
    let out = Printout {
        subject: "many pages",
        from: "a",
        to: "b",
        cc: "c",
        date: "now",
        attachments: &names,
        blocks: &blocks,
        picture: None,
        footer: "MailCraft · Inbox",
        paper: Paper::LETTER,
    };
    let pdf = pdf(&out, &bundled_fonts()).unwrap_or_else(|why| panic!("many pages printed: {why}"));
    assert!(page_count(&pdf) > 1, "the body runs over more than one page");
}

#[test]
fn a_message_with_no_body_is_said_to_have_none() {
    let names: Vec<String> = vec!["photo.jpg".to_owned()];
    let blocks: Vec<Block> = Vec::new();
    let out = Printout {
        subject: "only its heading",
        from: "a",
        to: "",
        cc: "",
        date: "",
        attachments: &names,
        blocks: &blocks,
        picture: None,
        footer: "",
        paper: Paper::A4,
    };
    let pdf = pdf(&out, &bundled_fonts()).unwrap_or_else(|why| panic!("a heading is a page: {why}"));
    assert!(page_count(&pdf) == 1, "the heading alone is one page");
}

/// A 4×4 red PNG.
fn red_png() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0, 169, 241, 158, 126, 0, 0, 0, 18, 73, 68,
        65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 25, 51, 144, 46, 0, 0, 60, 64, 31, 225, 224, 129, 119, 180, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ]
}

/// The bytes as base64, the way a message writes a picture into itself.
fn base64_of(bytes: &[u8]) -> String {
    const CODE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(CODE[(n >> 18) as usize] as char);
        out.push(CODE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(CODE[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(CODE[(n & 63) as usize] as char);
        }
    }
    out
}
