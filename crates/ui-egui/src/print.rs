// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Printing a message: the reading pane's display blocks set on a sheet of paper as a PDF, and the
//! system's spooler handed the pages (gap 15).
//!
//! Nothing is fetched or run for the pages: a picture written into the message is printed, one on
//! the web only when the owner let pictures be downloaded and its bytes are already here, and one
//! with no bytes prints the words that say what it is. Hidden sender text never reaches them. The
//! Print dialog (`ui.printDialog`) only collects what the owner chose and then runs `file.print`;
//! the command line and an agent are given the message as plain text.

use std::sync::Arc;

use egui::{RichText, Ui, vec2};
use mailcraft_model::{Id, Message};
use mailcraft_print::{Paper, PictureBytes, Printout, bundled_fonts, pdf};

use crate::MailApp;
use crate::reading::{images, remote};
use crate::theme::{Tokens, regular, semibold};

/// The width of the Print dialog.
pub const WIDTH: f32 = 400.0;
/// Most copies one job asks for.
const MAX_COPIES: u32 = 100;

/// A printer the system knows, as the dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    /// The one the system takes jobs from when none is named.
    pub default: bool,
    /// Whether it can print two-sided; `None` where the system doesn't say.
    pub two_sided: Option<bool>,
}

/// The pages and what to do with them: what the host is handed, and spools.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Job {
    pub pdf: Vec<u8>,
    /// What the spooler shows in its queue.
    pub title: String,
    /// `None`: the system's default printer.
    pub printer: Option<String>,
    pub copies: u32,
    pub collate: bool,
    pub two_sided: bool,
    pub grayscale: bool,
}

/// What the owner chose for the pages: the sheet, the copies, how they go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    /// US Letter instead of A4.
    pub letter: bool,
    pub copies: u32,
    pub collate: bool,
    pub two_sided: bool,
    pub grayscale: bool,
}

impl Choices {
    /// The sheet chosen.
    pub fn paper(&self) -> Paper {
        if self.letter { Paper::LETTER } else { Paper::A4 }
    }
}

/// The printers the system knows; empty where there is no spooler, as in the browser.
pub fn printers(app: &MailApp) -> Vec<Printer> {
    match app.services.printers.as_ref() {
        Some(list) => list(),
        None => Vec::new(),
    }
}

/// A message's body as display blocks: its HTML when it has one and it says something, its text
/// otherwise. Hidden sender text is in neither.
fn body(m: &Message) -> Vec<mailcraft_html::Block> {
    if let Some(html) = m.body.html.as_deref().filter(|h| !h.trim().is_empty()) {
        let blocks = mailcraft_html::parse(html);
        if !blocks.is_empty() {
            return blocks;
        }
    }
    mailcraft_html::text_blocks(&m.body.text)
}

/// The people of a header line, as the reading pane writes them.
fn people(list: &[mailcraft_model::Address]) -> String {
    list.iter().map(mailcraft_model::Address::full).collect::<Vec<_>>().join("; ")
}

/// The bytes of a picture the message asks for: its own file for a `cid:` address, a web one only
/// when the owner let pictures be downloaded and the window already has them. Nothing is fetched
/// here; a picture with no bytes at all prints what it says it is.
fn picture(app: &MailApp, ctx: Option<&egui::Context>, m: &Message, src: &str) -> Option<PictureBytes> {
    if let Some(a) = images::attachment(m, src)
        && !a.data.is_empty()
    {
        return Some((Arc::clone(&a.data), a.mime.clone()));
    }
    if let Some(ctx) = ctx
        && images::is_remote(src)
        && remote::available(app)
    {
        let cache = images::Pictures::of(ctx);
        let held = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(images::Remote::Ready(bytes)) = held.remote_peek(src) {
            return Some((bytes.clone(), String::new()));
        }
    }
    None
}

/// The pages of one message on `paper`: its heading, then its text, pictures and tables, each page
/// numbered at its foot. An error says why nothing came out.
pub fn pages(app: &MailApp, ctx: Option<&egui::Context>, m: &Message, paper: Paper, footer: &str) -> Result<Vec<u8>, String> {
    if !paper.is_sound() {
        return Err("the paper asked for is not a sheet to print on".to_owned());
    }
    let names: Vec<String> = m.attachments.iter().map(|a| a.name.clone()).collect();
    let from = m.from.full();
    let to = people(&m.to);
    let cc = people(&m.cc);
    let date = mailcraft_time::format_rfc5322(m.date, app.session.offset_at(m.date));
    let blocks = body(m);
    let source = |src: &str| -> Option<PictureBytes> { picture(app, ctx, m, src) };
    let out = Printout {
        subject: &m.subject,
        from: &from,
        to: &to,
        cc: &cc,
        date: &date,
        attachments: &names,
        blocks: &blocks,
        picture: Some(&source),
        footer,
        paper,
    };
    pdf(&out, &bundled_fonts()).map_err(|why| format!("the message couldn't be set on paper: {why}"))
}

/// One message to the printer: its pages, made here, handed to the system's spooler. Answers what
/// the spooler said ("job 12 on …"), or why nothing was printed.
pub fn print(app: &MailApp, id: Id, printer: Option<String>, choices: Choices) -> Result<String, String> {
    let m = app.session.loaded_message(id).map_err(|why| why.to_string())?;
    let footer = app.session.mailbox.folder(app.session.view.folder).map(|f| f.name.clone()).unwrap_or_else(|| "MailCraft".to_owned());
    let pdf = pages(app, app.ctx.as_ref(), &m, choices.paper(), &footer)?;
    let title = if m.subject.trim().is_empty() { tl!("(no subject)").to_owned() } else { m.subject.clone() };
    let job = Job {
        pdf,
        title,
        printer,
        copies: choices.copies.clamp(1, MAX_COPIES),
        collate: choices.collate,
        two_sided: choices.two_sided,
        grayscale: choices.grayscale,
    };
    match app.services.print.as_ref() {
        Some(print) => print(job),
        None => Err("printing isn't available here".to_owned()),
    }
}

/// How many pages a PDF has: krilla writes each page's own object as `/Type/Page/…`, so counting
/// those counts the pages.
pub fn page_count(pdf: &[u8]) -> usize {
    pdf.windows(b"/Type/Page/".len()).filter(|w| w == b"/Type/Page/").count()
}

/// The Print dialog: the printer, the copies, the sheet, and Print.
pub struct Ask {
    /// The message whose pages the dialog is making.
    pub message: Id,
    /// The window of its own the dialog comes up over: `0` is the main window, the number of a
    /// message window otherwise ([`crate::windows`]).
    pub window: u64,
    pub subject: String,
    pub printers: Vec<Printer>,
    /// The printer chosen; `None` is the system's default.
    pub printer: Option<String>,
    pub choices: Choices,
    pub error: String,
}

impl Ask {
    /// The dialog for the message in view, opened from the main window (`window`: `0`) or from the
    /// message's own window. `Err` when nothing is selected, or nothing can be printed here.
    pub fn open(app: &MailApp, window: u64) -> Result<Ask, String> {
        let Some(id) = app.session.sel.focus() else {
            return Err("select a message to print first".to_owned());
        };
        if app.services.print.is_none() {
            return Err("printing isn't available here".to_owned());
        }
        let printers = printers(app);
        let subject = app.session.mailbox.message(id).map(|m| m.subject.clone()).unwrap_or_default();
        let chosen = printers.iter().find(|p| p.default).map(|p| p.name.clone());
        Ok(Ask { message: id, window, subject, printers, printer: chosen, choices: Choices::default(), error: String::new() })
    }

    /// A printer's name as the dialog shows it: the system's default marked.
    fn shown(&self, name: &str) -> String {
        if self.printers.iter().any(|p| p.name == name && p.default) { format!("{name} ({})", tl!("default")) } else { name.to_owned() }
    }

    /// Print with what has been chosen so far: the pages, then the spooler. `true` when it went;
    /// otherwise `error` says why not and the dialog stays open.
    pub fn go(&mut self, app: &mut MailApp) -> bool {
        match print(app, self.message, self.printer.clone(), self.choices) {
            Ok(said) => {
                app.status(&said);
                true
            }
            Err(why) => {
                self.error = why;
                false
            }
        }
    }

    /// One frame of the dialog. `true` when it is done with: printed, or closed.
    pub fn show(&mut self, app: &mut MailApp, ui: &mut Ui) -> bool {
        let t = Tokens::get(ui.ctx());
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        ui.label(RichText::new(tl!("Print")).font(semibold(14.0)).color(t.text));
        ui.add(egui::Label::new(RichText::new(&self.subject).color(t.text_dim)).truncate());
        ui.add_space(4.0);
        if self.printers.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("No printer was found on this computer. Add one to the system, then print again."))
                        .font(regular(11.5))
                        .color(t.text_dim),
                )
                .wrap(),
            );
        } else {
            let two_sided_possible = self.printers.iter().any(|p| p.two_sided != Some(false));
            let marked = format!(" ({})", tl!("default"));
            let mut chosen = self.printer.clone();
            egui::ComboBox::from_id_salt("print-printer")
                .selected_text(chosen.clone().map(|n| self.shown(&n)).unwrap_or_else(|| tl!("System default").to_string()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, tl!("System default"));
                    for p in &self.printers {
                        ui.selectable_value(&mut chosen, Some(p.name.clone()), self.shown(&p.name));
                    }
                });
            // The label shown is the name plus the mark; keep the name.
            self.printer = chosen.map(|name| {
                let clean: &str = name.strip_suffix(marked.as_str()).unwrap_or(&name);
                clean.to_owned()
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Copies")).color(t.text_dim));
                ui.add(egui::Slider::new(&mut self.choices.copies, 1..=MAX_COPIES));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.choices.collate, tl!("Collate"));
                if two_sided_possible {
                    ui.checkbox(&mut self.choices.two_sided, tl!("Two sided"));
                }
                ui.checkbox(&mut self.choices.grayscale, tl!("Black and white"));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Paper")).color(t.text_dim));
                ui.selectable_value(&mut self.choices.letter, false, tl!("A4"));
                ui.selectable_value(&mut self.choices.letter, true, tl!("Letter"));
            });
        }
        if !self.error.is_empty() {
            ui.add(egui::Label::new(RichText::new(&self.error).color(t.overdue)).wrap());
        }
        ui.add_space(6.0);
        let mut done = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Cancel")).clicked() {
                done = true;
            } else if ui.button(tl!("Print")).clicked() {
                done = self.go(app);
            }
        });
        done
    }
}
