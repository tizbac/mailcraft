// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! "Download messages in full": the size each choice names, and the end "Everything" has — a
//! server's answer is never read without one, so an account that wants all of its mail stops at
//! the ceiling every kind of account has.

use crate::{DownloadInFull, MAX_WHOLE_MESSAGE};

#[test]
fn each_choice_names_its_own_size() {
    for (choice, bytes) in [
        (DownloadInFull::UpTo1Mb, 1 << 20),
        (DownloadInFull::UpTo10Mb, 10 << 20),
        (DownloadInFull::UpTo25Mb, 25 << 20),
        (DownloadInFull::UpTo50Mb, 50 << 20),
    ] {
        assert_eq!((choice.limit(), choice.whole_limit()), (Some(bytes), bytes), "{choice:?}");
    }
    assert_eq!(DownloadInFull::Everything.limit(), None, "no limit of its own");
    assert_eq!(DownloadInFull::Everything.whole_limit(), MAX_WHOLE_MESSAGE, "but the ceiling all have");
}

#[test]
fn the_ceiling_is_big_enough_for_a_hundred_megabyte_message() {
    // A message of 100 MB comes whole on every kind of account, and the ceiling is well past it.
    const _: () = assert!(MAX_WHOLE_MESSAGE >= 100 << 20);
    // Still bounded: nothing from a server is read without an end.
    const _: () = assert!(MAX_WHOLE_MESSAGE < 1 << 30);
    let app = DownloadInFull::Everything.whole_limit();
    assert!(app >= 100 << 20, "{app} bytes");
}
