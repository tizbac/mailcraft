// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Printing a message on the system's default printer: the printable text goes to a file of its
//! own under the temporary folder, and the system's own print command hands it over — Notepad's
//! `/p` on Windows, `lp` (CUPS), or `lpr`, on Linux and the BSDs. Nothing is rendered by a browser
//! and no viewer is opened: the message is text, as `file.print` makes it.

use std::process::Command;

/// Prints `text` under `name`. Returns when the print command has taken the file (the job may
/// still be queued); an error says why nothing was printed.
pub fn print(name: &str, text: &str) -> Result<(), String> {
    let folder = std::env::temp_dir().join("mailcraft-print");
    std::fs::create_dir_all(&folder).map_err(|why| format!("no temporary folder to print from: {why}"))?;
    // A name of our own making: the file name's own characters, one line, short.
    let mut stem: String = name.chars().take(64).map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    if !stem.to_ascii_lowercase().ends_with(".txt") {
        stem.push_str(".txt");
    }
    let file = folder.join(stem);
    std::fs::write(&file, text.as_bytes()).map_err(|why| format!("couldn't write the message to print: {why}"))?;
    let printed = platform(&file);
    let _ = std::fs::remove_file(&file);
    printed
}

#[cfg(windows)]
fn platform(file: &std::path::Path) -> Result<(), String> {
    // Notepad prints a text file to the default printer and exits (`/p`).
    let status = Command::new("notepad.exe").arg("/p").arg(file).status().map_err(|why| format!("couldn't start the printer: {why}"))?;
    if status.success() { Ok(()) } else { Err("the printer didn't take the message".to_owned()) }
}

#[cfg(not(windows))]
fn platform(file: &std::path::Path) -> Result<(), String> {
    // CUPS's `lp` first, then the older `lpr`. Both take a text file and hand it to the default
    // printer; neither opens a window.
    for command in ["lp", "lpr"] {
        match Command::new(command).arg(file).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(_) => return Err(format!("{command} didn't take the message")),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => continue,
            Err(why) => return Err(format!("couldn't start {command}: {why}")),
        }
    }
    Err("no printer command here (lp or lpr): printing needs CUPS".to_owned())
}
