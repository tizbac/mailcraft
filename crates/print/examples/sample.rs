// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A sample message to a PDF file, to look at: `cargo run -p mailcraft-print --example sample`.

use mailcraft_html::Block;
use mailcraft_print::{Paper, Printout, bundled_fonts, pdf};

fn main() {
    let blocks: Vec<Block> = mailcraft_html::parse(
        "<h1>The plan for November</h1><p>A <b>bold</b> word, an <i>italic</i> one, <span style='color:#006600'>green</span> text, \
         and a <a href='https://example.org'>link</a>.</p><ul><li>first</li><li>second</li></ul><hr>\
         <blockquote>someone said this, quoted</blockquote>\
         <table border='1' width='100%'><tr><th>Month</th><th>Sum</th></tr><tr><td>October</td><td>12</td></tr><tr><td>November</td><td>7</td></tr></table>",
    );
    let attachments = vec!["photo.jpg".to_owned()];
    let out = Printout {
        subject: "The plan for November",
        from: "Ada Lovelace <ada@example.org>",
        to: "Gil Clifton <gil@example.org>",
        cc: "",
        date: "Mon 17 Nov 2025, 09:12",
        attachments: &attachments,
        blocks: &blocks,
        picture: None,
        footer: "MailCraft · Inbox",
        paper: Paper::A4,
    };
    match pdf(&out, &bundled_fonts()) {
        Ok(bytes) => {
            let path = std::env::var("PRINT_SAMPLE").unwrap_or_else(|_| "/tmp/opencode/mailcraft-print-sample.pdf".to_owned());
            if std::fs::write(&path, bytes).is_ok() {
                println!("wrote {path}");
            }
        }
        Err(why) => println!("no pages: {why}"),
    }
}
