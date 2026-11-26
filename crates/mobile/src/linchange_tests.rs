// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Linchange server as Add Account takes it: an `https://` origin and nothing else.

use crate::accounts::linchange_origin;

#[test]
fn a_linchange_server_is_a_host_or_an_https_origin() {
    assert_eq!(linchange_origin("mail.example.org"), Some("https://mail.example.org".to_string()));
    assert_eq!(linchange_origin(" https://mail.example.org/ "), Some("https://mail.example.org".to_string()));
    assert_eq!(linchange_origin("[::1]:8443"), Some("https://[::1]:8443".to_string()));
    for bad in ["", " ", "http://mail.example.org", "mail.example.org/path", "mail example.org", ".example.org", "example.org."] {
        assert_eq!(linchange_origin(bad), None, "{bad}");
    }
}
