// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `text/uri-list` a dragged file goes as: `file://` and the path, with every byte a URI
//! does not allow written as `%XX`.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
fn a_path_becomes_a_file_uri() {
    assert_eq!(super::file_uri_path(Path::new("/tmp/a.eml")), "/tmp/a.eml");
    assert_eq!(super::file_uri_path(Path::new("/tmp/hello world.eml")), "/tmp/hello%20world.eml");
    assert_eq!(super::file_uri_path(Path::new("/tmp/100%.eml")), "/tmp/100%25.eml");
    assert_eq!(super::file_uri_path(Path::new("/tmp/Café.eml")), "/tmp/Caf%C3%A9.eml");
    assert_eq!(super::file_uri_path(Path::new("/tmp/a#b?c.eml")), "/tmp/a%23b%3Fc.eml");
    assert_eq!(super::file_uri_path(Path::new("/")), "/");
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
fn a_drag_carries_one_uri_a_line() {
    let list = super::uri_list(&[Path::new("/tmp/one.eml"), Path::new("/tmp/two eml.msg")]);
    assert_eq!(list, "file:///tmp/one.eml\r\nfile:///tmp/two%20eml.msg\r\n");
}
