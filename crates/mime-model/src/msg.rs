// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A message as an Outlook `.msg` file ([MS-OXMSG] over [MS-CFB], the compound file binary
//! format): the properties of the message, then one storage for each recipient and one for each
//! attachment, each with its own property stream and its variable-length values beside it.
//!
//! What is written is a note (`IPM.Note`) as Outlook reads it: the subject, the plain text and,
//! when there is one, the HTML body; the sender and the To, Cc and Bcc lines; its date, its
//! Internet message id and the header block it came with; and every attachment by value, under
//! its own name, media type and content id. Nothing here signs, encrypts or fetches anything; the
//! bytes never leave this computer, and a message only part of which is here is refused before
//! it is written (the command that calls this reads it whole first).
//!
//! The layout follows the public specification: a properties stream holds a 32-byte header (the
//! two counts of recipients and attachments) and then 16-byte entries, one per property; a
//! variable-length value is kept in a stream named `__substg1.0_<property id><type>`. Everything
//! is little-endian and the file is one compound file whose root holds those streams.

use std::io::Write as _;

use cfb::CompoundFile;
use mailcraft_model::{Address, Attachment, Importance, Message};

/// Property types ([MS-OXCDATA] §2.11.1): what the entries and stream names carry.
const PT_LONG: u16 = 0x0003;
const PT_BOOLEAN: u16 = 0x000B;
const PT_OBJECT: u16 = 0x000D;
const PT_UNICODE: u16 = 0x001F;
const PT_SYSTIME: u16 = 0x0040;
const PT_BINARY: u16 = 0x0102;

/// Why a message couldn't be written as a `.msg` file.
#[derive(Debug)]
pub enum MsgError {
    /// The compound file couldn't be made or grown: a disk, or a stream's name repeated.
    Io(std::io::Error),
    /// A name, a value or a count is larger than the format keeps.
    TooLarge,
}

impl std::fmt::Display for MsgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MsgError::Io(e) => write!(f, "the .msg file couldn't be written: {e}"),
            MsgError::TooLarge => f.write_str("the message is too large for a .msg file"),
        }
    }
}

impl std::error::Error for MsgError {}

impl From<std::io::Error> for MsgError {
    fn from(e: std::io::Error) -> Self {
        MsgError::Io(e)
    }
}

/// The message as the bytes of a `.msg` file.
pub fn write(message: &Message) -> Result<Vec<u8>, MsgError> {
    let mut file = CompoundFile::create(std::io::Cursor::new(Vec::new()))?;
    // The folders in the order Microsoft writes them: a client that walks them sees the message's
    // own header first.
    let mut top = Properties::new();
    top.unicode(0x001A, "IPM.Note");
    top.unicode(0x0037, &message.subject);
    top.unicode(0x1000, &message.body.text);
    if let Some(html) = message.body.html.as_ref().filter(|html| !html.is_empty()) {
        top.binary(0x1013, html.as_bytes());
    }
    let headers = message.header_block.lines();
    if !headers.is_empty() {
        let mut block = String::new();
        for line in &headers {
            block.push_str(&line.name);
            block.push_str(": ");
            block.push_str(&line.value);
            block.push_str("\r\n");
        }
        top.unicode(0x007D, &block);
    }
    top.unicode(0x0C1A, message.from.display());
    top.unicode(0x0C1F, &message.from.email);
    top.unicode(0x5D01, &message.from.email);
    if !message.to.is_empty() {
        top.unicode(0x0E04, &display_line(&message.to));
    }
    if !message.cc.is_empty() {
        top.unicode(0x0E03, &display_line(&message.cc));
    }
    if !message.bcc.is_empty() {
        top.unicode(0x0E02, &display_line(&message.bcc));
    }
    top.time(0x0E06, message.date);
    if !message.message_id.is_empty() {
        top.unicode(0x1035, &message.message_id);
    }
    top.long(
        0x0017,
        match message.importance {
            Importance::Low => 0,
            Importance::Normal => 1,
            Importance::High => 2,
        },
    );
    // A message that was read here is not marked unread there: both the read flag and the
    // message's own flags are left as Outlook would write them for a note.
    top.boolean(0x0E07, message.read);
    write_properties(&mut file, "/__properties_version1.0", &top, header(recipients(message), attachments(message)))?;

    for (index, (kind, address)) in recipients(message).into_iter().enumerate() {
        let storage = format!("/__recip_version1.0_#{index:08X}");
        file.create_storage(&storage)?;
        let mut props = Properties::new();
        props.long(0x0C15, i32::try_from(kind).unwrap_or(i32::MAX));
        props.unicode(0x3001, address.display());
        props.unicode(0x3003, &address.email);
        props.unicode(0x39FE, &address.email);
        props.unicode(0x39FF, "SMTP");
        write_properties(&mut file, &format!("{storage}/__properties_version1.0"), &props, vec![0; 8])?;
    }

    for (index, attachment) in attachments(message).into_iter().enumerate() {
        let storage = format!("/__attach_version1.0_#{index:08X}");
        file.create_storage(&storage)?;
        let mut props = Properties::new();
        let name = attachment.name.as_str();
        if let Some((stem, extension)) = split_name(name) {
            props.unicode(0x3703, extension);
            props.unicode(0x3704, stem);
        }
        props.unicode(0x3707, name);
        props.unicode(0x370E, &attachment.mime);
        props.long(0x3705, 1);
        props.long(0x0E20, i32::try_from(attachment.data.len()).map_err(|_| MsgError::TooLarge)?);
        if !attachment.content_id.is_empty() {
            props.unicode(0x3712, &attachment.content_id);
        }
        props.binary(0x3701, &attachment.data);
        write_properties(&mut file, &format!("{storage}/__properties_version1.0"), &props, vec![0; 8])?;
    }

    Ok(file.into_inner().into_inner())
}

/// The 32-byte header of the top-level properties stream ([MS-OXMSG] §2.4.1): no name ids (the
/// message uses no named properties), the next ids, and the counts.
fn header(recipients: Vec<(u32, &Address)>, attachments: Vec<&Attachment>) -> Vec<u8> {
    let next_recipient = usize_to_u32(recipients.len().saturating_add(1)).unwrap_or(u32::MAX);
    let next_attachment = usize_to_u32(attachments.len().saturating_add(1)).unwrap_or(u32::MAX);
    let count_recipients = usize_to_u32(recipients.len()).unwrap_or(u32::MAX);
    let count_attachments = usize_to_u32(attachments.len()).unwrap_or(u32::MAX);
    let mut out = vec![0u8; 8];
    out.extend_from_slice(&next_recipient.to_le_bytes());
    out.extend_from_slice(&next_attachment.to_le_bytes());
    out.extend_from_slice(&count_recipients.to_le_bytes());
    out.extend_from_slice(&count_attachments.to_le_bytes());
    out.extend_from_slice(&[0u8; 8]);
    out
}

fn recipients(message: &Message) -> Vec<(u32, &Address)> {
    let mut all = Vec::new();
    all.extend(message.to.iter().map(|a| (1, a)));
    all.extend(message.cc.iter().map(|a| (2, a)));
    all.extend(message.bcc.iter().map(|a| (3, a)));
    all
}

fn attachments(message: &Message) -> Vec<&Attachment> {
    message.attachments.iter().collect()
}

fn display_line(addresses: &[Address]) -> String {
    addresses.iter().map(Address::full).collect::<Vec<_>>().join("; ")
}

/// A message's property values, waiting to be written as one properties stream and the streams
/// of the values that don't fit in an entry.
struct Properties {
    entries: Vec<Entry>,
}

struct Entry {
    id: u16,
    kind: u16,
    value: Value,
}

enum Value {
    /// A value of at most eight bytes, written in the entry itself.
    Fixed(Vec<u8>),
    /// A longer value, kept in a stream of its own.
    Variable(Vec<u8>),
}

impl Properties {
    fn new() -> Properties {
        Properties { entries: Vec::new() }
    }

    fn push(&mut self, id: u16, kind: u16, value: Value) {
        self.entries.push(Entry { id, kind, value });
    }

    fn unicode(&mut self, id: u16, text: &str) {
        let mut bytes = Vec::with_capacity(text.len().saturating_mul(2).saturating_add(2));
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        self.push(id, PT_UNICODE, Value::Variable(bytes));
    }

    fn binary(&mut self, id: u16, bytes: &[u8]) {
        self.push(id, PT_BINARY, Value::Variable(bytes.to_vec()));
    }

    fn long(&mut self, id: u16, value: i32) {
        self.push(id, PT_LONG, Value::Fixed(value.to_le_bytes().to_vec()));
    }

    fn boolean(&mut self, id: u16, value: bool) {
        self.push(id, PT_BOOLEAN, Value::Fixed(vec![u8::from(value), 0, 0, 0, 0, 0, 0, 0]));
    }

    fn time(&mut self, id: u16, at: i64) {
        self.push(id, PT_SYSTIME, Value::Fixed(filetime(at).to_le_bytes().to_vec()));
    }
}

/// Unix seconds as a Windows `FILETIME` (100-nanosecond intervals since 1601-01-01).
fn filetime(at: i64) -> u64 {
    const EPOCH: i64 = 11_644_473_600;
    let seconds = at.saturating_add(EPOCH).max(0) as u64;
    seconds.saturating_mul(10_000_000)
}

/// Writes a properties stream (`header`, then one 16-byte entry per property) and the streams the
/// values that don't fit are kept in, under `path` in `file`.
fn write_properties(file: &mut CompoundFile<std::io::Cursor<Vec<u8>>>, path: &str, props: &Properties, header: Vec<u8>) -> Result<(), MsgError> {
    let mut stream = header;
    for entry in &props.entries {
        // The property tag and its type are one 32-bit word: id in the high half.
        let tag = (u32::from(entry.id) << 16) | u32::from(entry.kind);
        stream.extend_from_slice(&tag.to_le_bytes());
        // Readable and writable, as Microsoft writes every property of a message.
        stream.extend_from_slice(&6u32.to_le_bytes());
        match &entry.value {
            Value::Fixed(bytes) => {
                stream.extend_from_slice(bytes);
                stream.resize(stream.len().saturating_add(8usize.saturating_sub(bytes.len())), 0);
            }
            Value::Variable(bytes) => {
                stream.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_le_bytes());
                stream.extend_from_slice(&[0u8; 4]);
            }
        }
    }
    let mut out = file.create_stream(path)?;
    out.write_all(&stream)?;
    drop(out);

    let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or_default();
    for entry in &props.entries {
        if let Value::Variable(bytes) = &entry.value {
            let name = format!("{dir}/__substg1.0_{:04X}{:04X}", entry.id, entry.kind);
            let mut out = file.create_stream(&name)?;
            out.write_all(bytes)?;
        }
    }
    Ok(())
}

fn split_name(name: &str) -> Option<(&str, &str)> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty() && extension.len() <= 8).then_some((stem, extension))
}

fn usize_to_u32(n: usize) -> Result<u32, MsgError> {
    u32::try_from(n).map_err(|_| MsgError::TooLarge)
}

/// The property type of an object that is not written directly (attachments and recipients are
/// storages of their own): kept here so the tags read as the specification names them.
#[allow(dead_code)]
const ATTACHMENT_OBJECT: u16 = PT_OBJECT;

#[cfg(test)]
#[path = "msg_tests.rs"]
mod tests;
