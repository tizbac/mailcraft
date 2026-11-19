// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Printing: MailCraft sets the pages itself (`mailcraft-print`), and the system's own spooler takes
//! them — CUPS on Linux, macOS and the BSDs, the Windows print spooler in this process, with no
//! shell or other program started (`craft-print`). A job that the spooler refuses is an error in
//! words the owner can act on; nothing is written to disk for it.

use craft_print::{Duplex, Job as Spool, Scaling};

use mailcraft_ui_egui::print::{Job, Printer};

/// Most copies the spooler is asked for at once.
const MAX_COPIES: u32 = 100;

/// The printers the system knows: its queues, then the driverless ones CUPS can print to without a
/// driver. Empty where there is no spooler.
pub fn printers() -> Vec<Printer> {
    craft_print::printers().into_iter().map(|p| Printer { name: p.name, default: p.default, two_sided: p.two_sided }).collect()
}

/// Hand the pages to the spooler. Answers what it said — CUPS a job number, Windows how many
/// sheets it took — or why nothing was printed.
pub fn spool(job: Job) -> Result<String, String> {
    if job.pdf.is_empty() {
        return Err("there is nothing to print".to_owned());
    }
    let spool = Spool {
        printer: job.printer.clone(),
        copies: job.copies.clamp(1, MAX_COPIES),
        collate: job.collate,
        // `None` would keep the printer's own habit; the owner said which they want.
        duplex: Some(if job.two_sided { Duplex::LongEdge } else { Duplex::Off }),
        scaling: Scaling::Exact,
        grayscale: job.grayscale,
        // MailCraft's own pages are not pre-greyed: the driver is asked for grey, and a CUPS
        // queue that has no grey option still prints.
        grey_pages: false,
        title: job.title.clone(),
        options: Vec::new(),
        // The Windows driver's own dialog settings are not taken: every choice in the Print
        // dialog is MailCraft's.
        driver_settings: None,
    };
    craft_print::submit(&job.pdf, &spool).map_err(|why| format!("the printer didn't take the message: {why}"))
}
