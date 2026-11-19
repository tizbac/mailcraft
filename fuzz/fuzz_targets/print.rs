// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A message's HTML and its pictures set on paper as a PDF: nothing crashes, no page is written
//! for a sheet that isn't one, and the pages stay a small multiple of what came in — the blocks
//! the reader allows and the picture budget the pages hold.
#![no_main]

use std::sync::Arc;

use mailcraft_print::{Paper, Printout, bundled_fonts, pdf};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let blocks = mailcraft_html::parse(&html);
    // The same bytes are also every picture the message calls for: pages never fetch one, so what
    // the caller hands over is what the message's own bytes are.
    let bytes = Arc::new(data.to_vec());
    let source = |src: &str| -> Option<(Arc<Vec<u8>>, String)> { Some((bytes.clone(), src.to_owned())) };
    for paper in [Paper::A4, Paper::LETTER] {
        let out = Printout {
            subject: &html,
            from: "",
            to: "",
            cc: "",
            date: "",
            attachments: &[],
            blocks: &blocks,
            picture: Some(&source),
            footer: "",
            paper,
        };
        if let Ok(pages) = pdf(&out, &bundled_fonts()) {
            let sheets = pages.windows(b"/Type/Page/".len()).filter(|w| w == b"/Type/Page/").count();
            assert!(sheets <= 4 * mailcraft_html::MAX_BLOCKS + 8, "{sheets} sheets from {} bytes of input", data.len());
            assert!(pages.len() <= 128 * 1024 * 1024, "{} bytes of pages from {} of input", pages.len(), data.len());
        }
    }
});
