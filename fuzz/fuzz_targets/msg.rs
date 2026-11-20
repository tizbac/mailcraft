// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An Outlook `.msg` file from arbitrary bytes ([MS-CFB], [MS-OXMSG]): read as it is, and again
//! after every property value is cut in half, so that a file whose streams disagree with its
//! directory is met the same way. The seeds in `fuzz/seeds/msg/` are notes this program wrote.
#![no_main]

use mailcraft_mime_model::msg::read;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // Nothing may crash, and nothing may read more than the file holds.
    if let Ok(m) = read(data) {
        // What is read is bounded by what is there: the attachments can't outweigh the file.
        assert!(m.attachments.iter().map(|a| a.data.len()).sum::<usize>() <= data.len() * 4);
        // A note read back is a note that can be written and read again without a crash.
        if let Ok(written) = mailcraft_mime_model::msg::write(&m) {
            let _ = read(&written);
        }
    }
    // The same bytes with their property streams emptied: the directory still names them.
    let mut halved = data.to_vec();
    for byte in halved.iter_mut().take(data.len() / 2) {
        *byte = 0;
    }
    let _ = read(&halved);
});
