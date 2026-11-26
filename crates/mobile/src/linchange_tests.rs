// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Linchange server as Add Account takes it: an `https://` origin and nothing else, the mail
//! host without the web port, and the form's `mail.<domain>` guess recognised so the DNS is asked.

use crate::accounts::{linchange_is_guess, linchange_mail_host, linchange_origin, linchange_target};

#[test]
fn a_linchange_server_is_a_host_or_an_https_origin() {
    assert_eq!(linchange_origin("mail.example.org"), Some("https://mail.example.org".to_string()));
    assert_eq!(linchange_origin(" https://mail.example.org/ "), Some("https://mail.example.org".to_string()));
    assert_eq!(linchange_origin("[::1]:8443"), Some("https://[::1]:8443".to_string()));
    for bad in ["", " ", "http://mail.example.org", "mail.example.org/path", "mail example.org", ".example.org", "example.org."] {
        assert_eq!(linchange_origin(bad), None, "{bad}");
    }
}

#[test]
fn the_mail_host_loses_the_web_port() {
    assert_eq!(linchange_mail_host("https://mail.example.org"), Some("mail.example.org".to_string()));
    assert_eq!(linchange_mail_host("https://mail.example.org:8443"), Some("mail.example.org".to_string()));
    assert_eq!(linchange_mail_host("https://[::1]:8443"), Some("::1".to_string()));
    assert_eq!(linchange_mail_host("https://"), None);
}

#[test]
fn the_forms_mail_guess_is_recognised() {
    assert!(linchange_is_guess("noor@example.org", "https://mail.example.org"));
    assert!(linchange_is_guess("noor@Example.ORG", "https://mail.example.org/"));
    assert!(!linchange_is_guess("noor@example.org", "https://linchange.example.org"));
    assert!(!linchange_is_guess("noor@example.org", "https://mail.example.com"));
    assert!(!linchange_is_guess("not-an-address", "https://mail.example.org"));
}

#[test]
fn a_typed_server_keeps_its_port_for_the_page_but_not_for_mail() {
    assert_eq!(
        linchange_target("noor@example.org", "https://linchange.example.org:8443"),
        Some(("https://linchange.example.org:8443".to_string(), "linchange.example.org".to_string()))
    );
    assert_eq!(
        linchange_target("noor@example.org", "linchange.example.org"),
        Some(("https://linchange.example.org".to_string(), "linchange.example.org".to_string()))
    );
}
