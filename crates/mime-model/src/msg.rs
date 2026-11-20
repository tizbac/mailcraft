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
//!
//! [`read`] reads such a file back — one written here or one Outlook wrote: the subject, the body
//! as HTML, plain text or rich text, the sender and the recipients, the date, the message id, its
//! importance and whether it was read, and every attachment, an embedded message included. The
//! file is read from memory only: nothing is fetched, nothing is opened, and a file that is not a
//! compound file, is not a message, or is bigger or fuller than the format keeps is an error.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

use cfb::CompoundFile;
use mailcraft_model::{Address, Attachment, Body, FlagStatus, Importance, Message};

/// The class of a message's compound file ([MS-OXMSG] §2.2), `{00020D0B-0000-0000-C000-000000000046}`.
const MSG_CLSID: [u8; 16] = [0x00, 0x02, 0x0d, 0x0b, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

/// Property types ([MS-OXCDATA] §2.11.1): what the entries and stream names carry.
const PT_LONG: u16 = 0x0003;
const PT_BOOLEAN: u16 = 0x000B;
const PT_OBJECT: u16 = 0x000D;
const PT_UNICODE: u16 = 0x001F;
const PT_STRING8: u16 = 0x001E;
const PT_SYSTIME: u16 = 0x0040;
const PT_BINARY: u16 = 0x0102;
/// The bit set on a type whose property holds more than one value ([MS-OXCDATA] §2.11.1).
const PT_MULTIPLE: u16 = 0x1000;

/// The properties of a note, named as [MS-OXPROPS] names them: the ids this module writes and the
/// ids it reads are the same ones, so a property is named once here and nowhere else.
const PR_MESSAGE_CLASS: u16 = 0x001A;
const PR_SUBJECT: u16 = 0x0037;
const PR_SENT_REPRESENTING_NAME: u16 = 0x0042;
const PR_SENT_REPRESENTING_EMAIL_ADDRESS: u16 = 0x0044;
const PR_CONVERSATION_TOPIC: u16 = 0x0070;
const PR_TRANSPORT_MESSAGE_HEADERS: u16 = 0x007D;
const PR_FLAG_STATUS: u16 = 0x0011;
const PR_COMPLETE: u16 = 0x0012;
const PR_IMPORTANCE: u16 = 0x0017;
const PR_SENSITIVITY: u16 = 0x0036;
const PR_SENT_TIME: u16 = 0x0039;
const PR_SENDER_NAME: u16 = 0x0C1A;
const PR_SENDER_ADDRESS_TYPE: u16 = 0x0C1E;
const PR_SENDER_EMAIL_ADDRESS: u16 = 0x0C1F;
const PR_RECIPIENT_TYPE: u16 = 0x0C15;
const PR_DISPLAY_BCC: u16 = 0x0E02;
const PR_DISPLAY_CC: u16 = 0x0E03;
const PR_DISPLAY_TO: u16 = 0x0E04;
const PR_DELIVERY_TIME: u16 = 0x0E06;
const PR_MESSAGE_FLAGS: u16 = 0x0E07;
const PR_HAS_ATTACHMENT: u16 = 0x0E1B;
const PR_NORMALIZED_SUBJECT: u16 = 0x0E1D;
const PR_ATTACH_SIZE: u16 = 0x0E20;
const PR_ATTACH_NUMBER: u16 = 0x0E21;
const PR_ACCESS: u16 = 0x0FF4;
const PR_ACCESS_TYPE: u16 = 0x0FF7;
const PR_OBJECT_TYPE: u16 = 0x0FFE;
const PR_BODY: u16 = 0x1000;
const PR_RTF_COMPRESSED: u16 = 0x1009;
const PR_HTML: u16 = 0x1013;
const PR_INTERNET_MESSAGE_ID: u16 = 0x1035;
const PR_IN_REPLY_TO_ID: u16 = 0x1042;
const PR_ROWID: u16 = 0x3000;
const PR_DISPLAY_NAME: u16 = 0x3001;
const PR_ADDRESS_TYPE: u16 = 0x3002;
const PR_EMAIL_ADDRESS: u16 = 0x3003;
const PR_CREATION_TIME: u16 = 0x3007;
const PR_LAST_MODIFIED_TIME: u16 = 0x3008;
const PR_STORE_SUPPORT_MASK: u16 = 0x340D;
const PR_ATTACH_DATA: u16 = 0x3701;
const PR_ATTACH_EXTENSION: u16 = 0x3703;
const PR_ATTACH_FILENAME: u16 = 0x3704;
const PR_ATTACH_METHOD: u16 = 0x3705;
const PR_ATTACH_LONG_FILENAME: u16 = 0x3707;
const PR_RENDERING_POSITION: u16 = 0x370B;
const PR_ATTACH_MIME_TAG: u16 = 0x370E;
const PR_ATTACH_CONTENT_ID: u16 = 0x3712;
const PR_SMTP_ADDRESS: u16 = 0x39FE;
const PR_TRANSMITABLE_DISPLAY_NAME: u16 = 0x3A20;
const PR_CODE_PAGE: u16 = 0x3FDE;
const PR_CODE_PAGE_ID: u16 = 0x3FDD;
const PR_LANGUAGE: u16 = 0x3FFD;
const PR_SENDER_SMTP_ADDRESS: u16 = 0x5D01;

/// `PidTagMessageFlags` bits ([MS-OXCMSG] §2.2.3.1).
const MSG_FLAG_READ: i32 = 0x01;
const MSG_FLAG_UNREAD: i32 = 0x02;
const MSG_FLAG_UNSENT: i32 = 0x08;
const MSG_FLAG_HAS_ATTACH: i32 = 0x10;

/// Whether a recipient is a To, a Cc or a Bcc ([MS-OXOMSG] §2.2.3.1).
const RECIPIENT_TO: u32 = 1;
const RECIPIENT_CC: u32 = 2;
const RECIPIENT_BCC: u32 = 3;

/// What a note is ([MS-OXMSG] §2.3.1), and the classes a `.msg` file holds that are not mail.
const NOTE_CLASS: &str = "IPM.Note";

/// The streams a storage of a message holds ([MS-OXMSG] §2.4): one of property entries, and one
/// for every value too long for an entry, named after the property and the type it is kept as.
const PROPERTIES_STREAM: &str = "__properties_version1.0";
const VALUE_STREAM_PREFIX: &str = "__substg1.0_";
/// The storage of named properties. This writer keeps none, but a client that looks for the
/// storage must find it.
const NAMEID_STREAM: &str = "__nameid_version1.0";
/// The three substreams of that storage, which this writer leaves empty.
const NAME_TABLES: [&str; 3] = ["00020102", "00030102", "00040102"];
/// Storages of the recipients and the attachments of a message, numbered from zero, each holding
/// its own properties ([MS-OXMSG] §2.2.2.1).
const RECIPIENT_STORAGE_PREFIX: &str = "__recip_version1.0_";
const ATTACHMENT_STORAGE_PREFIX: &str = "__attach_version1.0_";

/// The name of the storage an attachment's value occupies when it is not bytes but a storage of
/// its own — an embedded message among them ([MS-OXMSG] §2.2.2.1). It sits beside the attachment's
/// own properties, named after `PidTagAttachDataObject` written as an object.
fn attachment_storage_name() -> String {
    format!("{VALUE_STREAM_PREFIX}{PR_ATTACH_DATA:04X}{PT_OBJECT:04X}")
}

/// Why a message couldn't be written as a `.msg` file, or a `.msg` file read.
#[derive(Debug)]
pub enum MsgError {
    /// The compound file couldn't be made or read.
    Io(std::io::Error),
    /// A name, a value or a count is larger than the format keeps, or the file is bigger or
    /// fuller than MailCraft reads.
    TooLarge,
    /// Not a compound file at all, or one whose root is not a message.
    NotMessage,
    /// A compound file that holds something other than mail: an appointment, a contact, a task.
    NotMail(String),
}

impl std::fmt::Display for MsgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MsgError::Io(e) => write!(f, "the .msg file couldn't be written or read: {e}"),
            MsgError::TooLarge => f.write_str("the .msg file is larger than MailCraft reads"),
            MsgError::NotMessage => f.write_str("the file is not an Outlook message"),
            MsgError::NotMail(class) => write!(f, "the file holds a {class}, not a mail message"),
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
    // A message's compound file is marked as one at its root ([MS-OXMSG] §2.2): Outlook refuses
    // one whose root has no class.
    file.set_storage_clsid("/", uuid::Uuid::from_bytes(MSG_CLSID))?;
    // The name id streams, empty: this writer has no named properties, but a client that looks
    // for the storage must find it.
    file.create_storage(format!("/{NAMEID_STREAM}"))?;
    for name in NAME_TABLES {
        file.create_stream(format!("/{NAMEID_STREAM}/{VALUE_STREAM_PREFIX}{name}"))?;
    }
    // The folders in the order Microsoft writes them: a client that walks them sees the message's
    // own header first.
    let mut top = Properties::new();
    top.unicode(PR_MESSAGE_CLASS, NOTE_CLASS);
    top.unicode(PR_SUBJECT, &message.subject);
    top.unicode(PR_BODY, &message.body.text);
    if let Some(html) = message.body.html.as_ref().filter(|html| !html.is_empty()) {
        top.binary(PR_HTML, html.as_bytes());
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
        top.unicode(PR_TRANSPORT_MESSAGE_HEADERS, &block);
    }
    top.unicode(PR_SENDER_NAME, message.from.display());
    top.unicode(PR_SENDER_ADDRESS_TYPE, "SMTP");
    top.unicode(PR_SENDER_EMAIL_ADDRESS, &message.from.email);
    top.unicode(PR_SENDER_SMTP_ADDRESS, &message.from.email);
    if !message.to.is_empty() {
        top.unicode(PR_DISPLAY_TO, &display_line(&message.to));
    }
    if !message.cc.is_empty() {
        top.unicode(PR_DISPLAY_CC, &display_line(&message.cc));
    }
    if !message.bcc.is_empty() {
        top.unicode(PR_DISPLAY_BCC, &display_line(&message.bcc));
    }
    top.time(PR_DELIVERY_TIME, message.date);
    // The rest of what Outlook writes for a note: when it was made and changed, what it is, how
    // its text is encoded, and whether it carries files.
    top.time(PR_CREATION_TIME, message.date);
    top.time(PR_LAST_MODIFIED_TIME, message.date);
    top.long(PR_ACCESS, 2);
    top.long(PR_ACCESS_TYPE, 0);
    top.long(PR_STORE_SUPPORT_MASK, 0x0004_0E79);
    top.long(PR_CODE_PAGE, 65001);
    top.long(PR_LANGUAGE, 65001);
    top.long(PR_SENSITIVITY, 0);
    top.unicode(PR_NORMALIZED_SUBJECT, &message.subject);
    top.unicode(PR_CONVERSATION_TOPIC, &message.subject);
    if !message.message_id.is_empty() {
        top.unicode(PR_INTERNET_MESSAGE_ID, &message.message_id);
    }
    top.long(
        PR_IMPORTANCE,
        match message.importance {
            Importance::Low => 0,
            Importance::Normal => 1,
            Importance::High => 2,
        },
    );
    // A message that was read here is not marked unread there: both the read flag and the
    // message's own flags are left as Outlook would write them for a note.
    // PidTagMessageFlags is a long, not a boolean: read, unchanged, and with files.
    let files = attachments(message);
    let flags = MSG_FLAG_UNREAD | (if message.read { MSG_FLAG_READ } else { 0 }) | (if files.is_empty() { 0 } else { MSG_FLAG_HAS_ATTACH });
    top.long(PR_MESSAGE_FLAGS, flags);
    top.boolean(PR_HAS_ATTACHMENT, !files.is_empty());
    write_properties(&mut file, &format!("/{}", PROPERTIES_STREAM), &top, header(recipients(message), attachments(message)))?;

    for (index, (kind, address)) in recipients(message).into_iter().enumerate() {
        let storage = format!("/{RECIPIENT_STORAGE_PREFIX}#{index:08X}");
        file.create_storage(&storage)?;
        let mut props = Properties::new();
        props.long(PR_RECIPIENT_TYPE, i32::try_from(kind).unwrap_or(i32::MAX));
        props.long(PR_OBJECT_TYPE, 6);
        props.long(PR_ROWID, i32::try_from(index).unwrap_or(i32::MAX));
        props.unicode(PR_DISPLAY_NAME, address.display());
        props.unicode(PR_ADDRESS_TYPE, "SMTP");
        props.unicode(PR_EMAIL_ADDRESS, &address.email);
        props.unicode(PR_SMTP_ADDRESS, &address.email);
        write_properties(&mut file, &format!("{storage}/{PROPERTIES_STREAM}"), &props, vec![0; 8])?;
    }

    for (index, attachment) in attachments(message).into_iter().enumerate() {
        let storage = format!("/{ATTACHMENT_STORAGE_PREFIX}#{index:08X}");
        file.create_storage(&storage)?;
        let mut props = Properties::new();
        let name = attachment.name.as_str();
        if let Some((stem, extension)) = split_name(name) {
            props.unicode(PR_ATTACH_EXTENSION, extension);
            props.unicode(PR_ATTACH_FILENAME, stem);
        }
        props.unicode(PR_ATTACH_LONG_FILENAME, name);
        props.unicode(PR_ATTACH_MIME_TAG, &attachment.mime);
        props.long(PR_ATTACH_NUMBER, i32::try_from(index).unwrap_or(i32::MAX));
        props.long(PR_OBJECT_TYPE, 7);
        props.long(PR_ACCESS, 2);
        props.long(PR_ACCESS_TYPE, 0);
        props.long(PR_RENDERING_POSITION, -1);
        props.long(PR_ATTACH_METHOD, 1);
        props.long(PR_ATTACH_SIZE, i32::try_from(attachment.data.len()).map_err(|_| MsgError::TooLarge)?);
        if !attachment.content_id.is_empty() {
            props.unicode(PR_ATTACH_CONTENT_ID, &attachment.content_id);
        }
        props.binary(PR_ATTACH_DATA, &attachment.data);
        write_properties(&mut file, &format!("{storage}/{PROPERTIES_STREAM}"), &props, vec![0; 8])?;
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
    all.extend(message.to.iter().map(|a| (RECIPIENT_TO, a)));
    all.extend(message.cc.iter().map(|a| (RECIPIENT_CC, a)));
    all.extend(message.bcc.iter().map(|a| (RECIPIENT_BCC, a)));
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
        // A string in a property stream ends with a null ([MS-OXMSG] §2.4.2.1): the length the
        // entry gives includes it.
        let mut bytes = Vec::with_capacity(text.len().saturating_mul(2).saturating_add(2));
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
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
            let name = format!("{dir}/{VALUE_STREAM_PREFIX}{:04X}{:04X}", entry.id, entry.kind);
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

/// The size of one entry of a property stream, and the header that precedes the entries: 32 bytes
/// for the message's own stream, 8 for a recipient's or an attachment's ([MS-OXMSG] §2.4.1).
const ENTRY_SIZE: usize = 16;
const TOP_HEADER: usize = 32;
const PART_HEADER: usize = 8;

// ------------------------------------------------------------------ a `.msg` file read

/// The bytes every compound file begins with ([MS-CFB] §2.1).
const CFB_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// The most of a `.msg` file read at a time, and the most storages, properties, recipients and
/// attachments taken from one. Past these the file is not a message somebody wrote but one made
/// to make this read for ever or allocate without end, so it is refused.
const MAX_FILE: usize = 256 << 20;
const MAX_PARTS: usize = 10_000;
const MAX_PROPERTIES: usize = 4_000;
const MAX_RECIPIENTS: usize = 1_000;
const MAX_ATTACHMENTS: usize = 256;
/// How deep an embedded message is followed: a message attached to a message, and no further.
const MAX_EMBEDDED_DEPTH: usize = 2;

/// The message a `.msg` file holds. The bytes are the whole file; nothing is fetched or opened,
/// and a file that is not a compound file, is not a note, or is bigger or fuller than the format
/// keeps is an error rather than a half-read message.
pub fn read(bytes: &[u8]) -> Result<Message, MsgError> {
    if bytes.len() > MAX_FILE {
        return Err(MsgError::TooLarge);
    }
    if !bytes.starts_with(&CFB_MAGIC) {
        return Err(MsgError::NotMessage);
    }
    let mut file = File::open(bytes)?;
    read_note(&mut file, "", 0)
}

/// One part of the compound file: a stream, or a storage holding others.
#[derive(Debug)]
struct Part {
    name: String,
    stream: bool,
}

/// The compound file and everything in it, by the storage it is in — `""` is the root. Values are
/// read one at a time, so a big attachment is only in memory when something asks for it.
///
/// `left` is how many bytes may still be taken out of streams in total. A file may name the same
/// sectors over and over in its directory, so without this a small file could ask for many times
/// its own size; the bound is a few times the bytes handed in, whatever the file says inside.
struct File<'b> {
    cfb: CompoundFile<std::io::Cursor<&'b [u8]>>,
    parts: BTreeMap<String, Vec<Part>>,
    left: usize,
}

impl<'b> File<'b> {
    fn open(bytes: &'b [u8]) -> Result<File<'b>, MsgError> {
        let cfb = CompoundFile::open(std::io::Cursor::new(bytes))?;
        // The whole directory of the file first: the chains of sectors are bounded by the file's
        // own size, and this counts what is in it, so nothing here reads more than the bytes given.
        let mut parts: BTreeMap<String, Vec<Part>> = BTreeMap::new();
        let mut seen = 0usize;
        for entry in cfb.walk() {
            seen += 1;
            if seen > MAX_PARTS {
                return Err(MsgError::TooLarge);
            }
            if entry.is_root() {
                continue;
            }
            let path = entry.path().to_string_lossy().into_owned();
            let path = path.strip_prefix('/').unwrap_or(&path);
            if path.is_empty() {
                continue;
            }
            let (dir, name) = match path.rfind('/') {
                Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
                None => (String::new(), path.to_string()),
            };
            parts.entry(dir).or_default().push(Part { name, stream: entry.is_stream() });
        }
        Ok(File { cfb, parts, left: bytes.len().saturating_mul(4) })
    }

    /// The bytes of one stream, at most `cap` of them, out of what the file still has left.
    fn stream(&mut self, path: &str, cap: usize) -> Result<Vec<u8>, MsgError> {
        let take = cap.min(self.left);
        if take == 0 {
            return Err(MsgError::TooLarge);
        }
        let mut out = Vec::new();
        self.cfb.open_stream(path)?.take(take as u64).read_to_end(&mut out)?;
        self.left = self.left.saturating_sub(out.len());
        Ok(out)
    }

    fn has(&self, path: &str) -> bool {
        self.cfb.exists(path)
    }

    /// Whether the root storage is marked as a message, by its class id ([MS-OXMSG] §2.2).
    fn root_is_message(&self) -> bool {
        self.cfb.root_entry().clsid() == &uuid::Uuid::from_bytes(MSG_CLSID)
    }

    /// The names of the storages under `dir` whose name begins with `prefix`, at most `limit`.
    fn storages(&self, dir: &str, prefix: &str, limit: usize) -> Vec<String> {
        let key = dir.strip_prefix('/').unwrap_or(dir);
        let mut out = Vec::new();
        for part in self.parts.get(key).into_iter().flat_map(|v| v.iter()) {
            if !part.stream && part.name.starts_with(prefix) {
                out.push(part.name.clone());
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }
}

/// A property entry of a property stream: what the property is, and the eight bytes its entry
/// holds. For a value too long for the entry those are the length of the stream that keeps it.
#[derive(Debug)]
struct Property {
    id: u16,
    kind: u16,
    data: [u8; 8],
}

/// The property stream of a storage, as far as it is read. A storage with none keeps its values in
/// the streams beside it, which are still found below. The message's own stream begins with a
/// 32-byte header (the counts of recipients and attachments); a recipient's or an attachment's
/// begins with 8 reserved bytes ([MS-OXMSG] §2.4.1). Either way the entries are 16 bytes each,
/// and the header is whichever of the two the stream's length fits.
fn properties(file: &mut File<'_>, dir: &str) -> Result<Vec<Property>, MsgError> {
    let path = format!("{dir}/{PROPERTIES_STREAM}");
    if !file.has(&path) {
        return Ok(Vec::new());
    }
    let bytes = file.stream(&path, MAX_PROPERTIES.saturating_mul(ENTRY_SIZE).saturating_add(TOP_HEADER))?;
    let header = if bytes.len() >= TOP_HEADER && (bytes.len() - TOP_HEADER).is_multiple_of(ENTRY_SIZE) { TOP_HEADER } else { PART_HEADER };
    let count = bytes.len().saturating_sub(header) / ENTRY_SIZE;
    let mut out = Vec::with_capacity(count.min(MAX_PROPERTIES));
    for i in 0..count.min(MAX_PROPERTIES) {
        let at = header + i * ENTRY_SIZE;
        let tag = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        out.push(Property {
            id: (tag >> 16) as u16,
            kind: (tag & 0xFFFF) as u16,
            data: [bytes[at + 8], bytes[at + 9], bytes[at + 10], bytes[at + 11], bytes[at + 12], bytes[at + 13], bytes[at + 14], bytes[at + 15]],
        });
    }
    Ok(out)
}

/// A property's value held in the entry itself: an integer, a boolean, a time. A property kept as
/// text or as bytes has its value in a stream of its own, so its entry's eight bytes are only the
/// size of that stream and are not read as a value. A property that holds more than one value
/// (the `0x1000` bit of its type) is not read here either.
fn fixed(props: &[Property], id: u16) -> Option<[u8; 8]> {
    props.iter().find(|p| p.id == id && !matches!(p.kind, PT_UNICODE | PT_STRING8 | PT_BINARY) && p.kind & PT_MULTIPLE == 0).map(|p| p.data)
}

fn long(props: &[Property], id: u16) -> Option<i32> {
    let d = fixed(props, id)?;
    Some(i32::from_le_bytes([d[0], d[1], d[2], d[3]]))
}

/// A Windows `FILETIME` (100-nanosecond intervals since 1601-01-01) as Unix seconds. A time that
/// doesn't fall in the range this keeps is no time at all.
fn filetime_to_seconds(data: [u8; 8]) -> Option<i64> {
    const EPOCH: u64 = 11_644_473_600;
    let intervals = u64::from_le_bytes(data);
    let seconds = intervals / 10_000_000;
    let before = seconds.checked_sub(EPOCH)?;
    if before > i64::MAX as u64 {
        return None;
    }
    Some(before as i64)
}

fn time(props: &[Property], id: u16) -> Option<i64> {
    filetime_to_seconds(fixed(props, id)?)
}

/// The stream that keeps a property's value in a storage: the value is written in a stream named
/// after the property and the type it is kept as ([MS-OXMSG] §2.4.2).
fn value_path(dir: &str, id: u16, kind: u16) -> String {
    format!("{dir}/{VALUE_STREAM_PREFIX}{id:04X}{kind:04X}")
}

fn read_value(file: &mut File<'_>, dir: &str, id: u16, kind: u16) -> Result<Option<Vec<u8>>, MsgError> {
    let path = value_path(dir, id, kind);
    if !file.has(&path) {
        return Ok(None);
    }
    file.stream(&path, MAX_FILE).map(Some)
}

/// The one-byte character set a file's text is written in, from the code page it names
/// ([MS-OXPROPS] PidTagCodePage). A page this doesn't know reads as Windows-1252, what Outlook
/// uses when it says nothing.
fn charset_of(code_page: Option<i32>) -> &'static str {
    match code_page {
        Some(65001) => "utf-8",
        Some(1250) => "windows-1250",
        Some(1251) => "windows-1251",
        Some(1252) => "windows-1252",
        Some(1253) => "windows-1253",
        Some(1254) => "windows-1254",
        Some(1255) => "windows-1255",
        Some(1256) => "windows-1256",
        Some(1257) => "windows-1257",
        Some(1258) => "windows-1258",
        Some(874) => "windows-874",
        Some(932) => "shift_jis",
        Some(936) => "gbk",
        Some(950) => "big5",
        Some(28592) => "iso-8859-2",
        Some(28593) => "iso-8859-3",
        Some(28594) => "iso-8859-4",
        Some(28595) => "iso-8859-5",
        Some(28597) => "iso-8859-7",
        Some(28598) => "iso-8859-8",
        // Latin-1 (28591) is read as Windows-1252, its superset, as the decoder does everywhere.
        _ => "windows-1252",
    }
}

/// Text of a property, as Unicode or as the older one-byte form in the character set the file
/// names. A property with no value, or one that reads as nothing, is no text.
fn text(file: &mut File<'_>, dir: &str, id: u16, charset: &str) -> Result<Option<String>, MsgError> {
    for kind in [PT_UNICODE, PT_STRING8] {
        let Some(bytes) = read_value(file, dir, id, kind)? else { continue };
        let words = if kind == PT_UNICODE {
            let units: Vec<u16> = bytes.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else if charset == "utf-8" {
            // A file that says UTF-8 but holds bytes that aren't is read as Windows-1252, which
            // is what such bytes almost always are.
            mailcraft_charset::guess(&bytes)
        } else {
            mailcraft_charset::decode(&bytes, charset)
        };
        let words = words.trim_matches('\0');
        if !words.is_empty() {
            return Ok(Some(words.to_string()));
        }
    }
    Ok(None)
}

/// The first of several properties that has a value, as text: Outlook writes the same thing under
/// more than one id depending on the program that made the file.
fn text_in(file: &mut File<'_>, dir: &str, ids: &[u16], charset: &str) -> Result<String, MsgError> {
    for &id in ids {
        if let Some(words) = text(file, dir, id, charset)? {
            return Ok(words);
        }
    }
    Ok(String::new())
}

/// The HTML of a body, which Outlook keeps either as it is or base64-encoded. When every byte of
/// the stream is a base64 character and decoding it gives a document, that is what it is.
fn html(file: &mut File<'_>, dir: &str, charset: &str) -> Result<Option<String>, MsgError> {
    let Some(raw) = read_value(file, dir, PR_HTML, PT_BINARY)? else {
        let Some(as_text) = read_value(file, dir, PR_HTML, PT_UNICODE)? else {
            return Ok(None);
        };
        if as_text.is_empty() {
            return Ok(None);
        }
        let units: Vec<u16> = as_text.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let decoded = String::from_utf16_lossy(&units);
        return Ok((!decoded.is_empty()).then_some(decoded));
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let packed: String = raw.iter().filter(|b| !b.is_ascii_whitespace()).map(|b| (*b) as char).collect();
    if packed.len() >= 8
        && packed.len().is_multiple_of(4)
        && let Ok(decoded) = mailcraft_charset::base64::STANDARD.decode(packed)
    {
        // Everything in it is a base64 character and the bytes behind it are a document: the HTML
        // was written packed, as some programs write it.
        if decoded.contains(&b'<') {
            return Ok(Some(mailcraft_charset::decode(&decoded, charset)));
        }
    }
    Ok(Some(mailcraft_charset::decode(&raw, charset)))
}

/// The recipients of a storage, in To, Cc and Bcc lines. A recipient with neither a name nor an
/// address is left out; one whose type the file doesn't say is taken as a To.
fn read_recipients(file: &mut File<'_>, dir: &str, charset: &str) -> Result<(Vec<Address>, Vec<Address>, Vec<Address>), MsgError> {
    let mut to = Vec::new();
    let mut cc = Vec::new();
    let mut bcc = Vec::new();
    for name in file.storages(dir, RECIPIENT_STORAGE_PREFIX, MAX_RECIPIENTS) {
        let storage = format!("{dir}/{name}");
        let props = properties(file, &storage)?;
        let kind = long(&props, PR_RECIPIENT_TYPE).unwrap_or(RECIPIENT_TO as i32);
        let email = text_in(file, &storage, &[PR_SMTP_ADDRESS, PR_EMAIL_ADDRESS], charset)?;
        // The name a file keeps is often the address itself; that is not a name to show beside it.
        let shown = text_in(file, &storage, &[PR_TRANSMITABLE_DISPLAY_NAME, PR_DISPLAY_NAME], charset)?;
        let shown = if shown == email { String::new() } else { shown };
        if shown.is_empty() && email.is_empty() {
            continue;
        }
        let address = Address::new(shown, email);
        if kind == RECIPIENT_CC as i32 {
            cc.push(address);
        } else if kind == RECIPIENT_BCC as i32 {
            bcc.push(address);
        } else {
            to.push(address);
        }
    }
    Ok((to, cc, bcc))
}

/// Addresses of a display line (`"Ada <ada@example.org>; Bob <bob@example.org>"`), for a file that
/// names its recipients in the To, Cc and Bcc lines rather than in storages of their own.
fn line_addresses(line: &str) -> Vec<Address> {
    let mut out = Vec::new();
    for part in line.split([';', ',']) {
        if let Some(a) = Address::parse(part) {
            out.push(a);
        }
    }
    out
}

/// A note's own attachment storages: each one a file by value, an embedded message, or a file
/// elsewhere that only names where it is.
fn read_attachments(file: &mut File<'_>, dir: &str, depth: usize, charset: &str) -> Result<Vec<Attachment>, MsgError> {
    let mut out = Vec::new();
    for name in file.storages(dir, ATTACHMENT_STORAGE_PREFIX, MAX_ATTACHMENTS) {
        let storage = format!("{dir}/{name}");
        let shown = text_in(file, &storage, &[PR_ATTACH_LONG_FILENAME, PR_ATTACH_FILENAME, PR_DISPLAY_NAME], charset)?;
        let media = text(file, &storage, PR_ATTACH_MIME_TAG, charset)?.unwrap_or_default();
        // An attachment that is itself a message: its own storages sit beside its properties.
        let inner_name = attachment_storage_name();
        if !file.storages(&storage, &inner_name, 1).is_empty() && depth < MAX_EMBEDDED_DEPTH {
            if let Ok(inner) = read_note(file, &format!("{storage}/{inner_name}"), depth + 1) {
                let written = write(&inner).unwrap_or_default();
                if !written.is_empty() {
                    out.push(Attachment::new(attach_name(shown, ".msg"), "application/octet-stream", written));
                }
            }
            continue;
        }
        let data = read_value(file, &storage, PR_ATTACH_DATA, PT_BINARY)?.unwrap_or_default();
        if data.is_empty() && shown.is_empty() {
            continue;
        }
        let content_id = text(file, &storage, PR_ATTACH_CONTENT_ID, charset)?.unwrap_or_default();
        let inline = !content_id.is_empty() && media.starts_with("image/");
        let mime = if media.is_empty() { "application/octet-stream".to_string() } else { media };
        out.push(Attachment {
            name: attach_name(shown, &extension_of(&mime)),
            mime,
            content_id,
            inline,
            data: std::sync::Arc::new(data),
            kept: None,
        });
    }
    Ok(out)
}

/// A file's name, with the extension the message says it has when the name has none.
fn attach_name(name: String, extension: &str) -> String {
    if name.is_empty() {
        return format!("attachment{extension}");
    }
    if extension.is_empty() || name.ends_with(extension) {
        return name;
    }
    format!("{name}{extension}")
}

fn extension_of(media: &str) -> String {
    match media.to_ascii_lowercase().as_str() {
        "image/png" => ".png",
        "image/jpeg" | "image/jpg" => ".jpg",
        "image/gif" => ".gif",
        "application/pdf" => ".pdf",
        "text/plain" => ".txt",
        "text/html" => ".html",
        _ => "",
    }
    .to_string()
}

/// A message id as the mailbox keeps it: the brackets around it are not part of it.
fn clean_id(words: Option<String>) -> String {
    words.unwrap_or_default().trim_matches('<').trim_matches('>').to_string()
}

/// The header lines a file keeps with its note, as name and value pairs; `None` when it keeps none.
/// A line whose name is not a header name is left out.
fn header_lines(file: &mut File<'_>, dir: &str, charset: &str) -> Result<Option<Vec<(String, String)>>, MsgError> {
    let Some(block) = text(file, dir, PR_TRANSPORT_MESSAGE_HEADERS, charset)? else {
        return Ok(None);
    };
    let mut lines = Vec::new();
    for line in block.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                lines.push((name.to_string(), value.trim().to_string()));
            }
        }
    }
    Ok((!lines.is_empty()).then_some(lines))
}

/// A body kept only as rich text ([MS-OXRTFEX]): the HTML inside it if it has one, else the text the
/// document shows. `None` when the stored rich text can't be read.
fn rtf_body(file: &mut File<'_>, dir: &str) -> Result<Option<Body>, MsgError> {
    let Some(stored) = read_value(file, dir, PR_RTF_COMPRESSED, PT_BINARY)? else {
        return Ok(None);
    };
    let Ok(rtf) = mailcraft_pst::decompress_rtf(&stored, MAX_FILE) else {
        return Ok(None);
    };
    if let Some((inner, _)) = mailcraft_pst::rtf_html(&rtf, MAX_FILE).ok().flatten() {
        return Ok(Some(Body { text: String::new(), html: Some(inner) }));
    }
    match mailcraft_pst::rtf_text(&rtf, MAX_FILE) {
        Ok((plain, _)) => Ok(Some(Body { text: plain, html: None })),
        Err(_) => Ok(None),
    }
}

/// The body of a note: the text and the HTML it keeps. With neither, the rich text inside it; with
/// HTML but no text, the HTML's own words as its text.
fn read_body(file: &mut File<'_>, dir: &str, charset: &str) -> Result<Body, MsgError> {
    let html = html(file, dir, charset)?.filter(|h| !h.is_empty());
    let mut text = text(file, dir, PR_BODY, charset)?.unwrap_or_default();
    if text.is_empty()
        && html.is_none()
        && let Some(body) = rtf_body(file, dir)?
    {
        return Ok(body);
    }
    if text.is_empty()
        && let Some(inner) = &html
    {
        text = mailcraft_html::text_and_hidden(inner).0;
    }
    Ok(Body { text, html })
}

/// One message, as its storage holds it: the root of the file, or the storage of an embedded one.
fn read_note(file: &mut File<'_>, dir: &str, depth: usize) -> Result<Message, MsgError> {
    let props = properties(file, dir)?;
    // The character set the file's one-byte strings are in, which the code page it names says.
    let charset = charset_of(long(&props, PR_CODE_PAGE).or_else(|| long(&props, PR_CODE_PAGE_ID)));
    let class = text(file, dir, PR_MESSAGE_CLASS, charset)?.unwrap_or_default();
    if class.is_empty() && depth == 0 && !file.root_is_message() {
        // Outlook marks a message at the root of the compound file. Without the class or the mark
        // this is some other compound file.
        return Err(MsgError::NotMessage);
    }
    if !class.is_empty() && !class.starts_with(NOTE_CLASS) {
        let named: String = class.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').take(40).collect();
        return Err(MsgError::NotMail(named));
    }

    let subject = text_in(file, dir, &[PR_SUBJECT, PR_CONVERSATION_TOPIC], charset)?;
    // Message ids are kept without the brackets around them, as everywhere in the mailbox.
    let message_id = clean_id(text(file, dir, PR_INTERNET_MESSAGE_ID, charset)?);
    let in_reply_to = clean_id(text(file, dir, PR_IN_REPLY_TO_ID, charset)?);
    let from_name = text_in(file, dir, &[PR_SENDER_NAME, PR_SENT_REPRESENTING_NAME], charset)?;
    // The address a file gives for the sender is sometimes an Exchange one, `/O=…/CN=…`, which
    // is not an address anybody can write to; the SMTP id beside it is.
    let from_email = {
        let mut best = String::new();
        for id in [PR_SENDER_EMAIL_ADDRESS, PR_SENDER_SMTP_ADDRESS, PR_SENT_REPRESENTING_EMAIL_ADDRESS] {
            let candidate = text(file, dir, id, charset)?.unwrap_or_default();
            if candidate.is_empty() {
                continue;
            }
            if !candidate.starts_with('/') && candidate.contains('@') {
                best = candidate;
                break;
            }
            if best.is_empty() {
                best = candidate;
            }
        }
        best
    };
    let (to, cc, bcc) = read_recipients(file, dir, charset)?;
    let (to, cc, bcc) = if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        (
            line_addresses(&text(file, dir, PR_DISPLAY_TO, charset)?.unwrap_or_default()),
            line_addresses(&text(file, dir, PR_DISPLAY_CC, charset)?.unwrap_or_default()),
            line_addresses(&text(file, dir, PR_DISPLAY_BCC, charset)?.unwrap_or_default()),
        )
    } else {
        (to, cc, bcc)
    };

    let date = time(&props, PR_DELIVERY_TIME).or_else(|| time(&props, PR_SENT_TIME)).or_else(|| time(&props, PR_CREATION_TIME)).unwrap_or(0);
    let received = time(&props, PR_DELIVERY_TIME).or_else(|| time(&props, PR_SENT_TIME)).unwrap_or(date);

    // The header lines the message came with, so that View Source and a forward as an attachment
    // still have them.
    let (headers, header_block) = mailcraft_model::headers::split(header_lines(file, dir, charset)?.unwrap_or_default());

    let flags = long(&props, PR_MESSAGE_FLAGS).unwrap_or(0);
    let body = read_body(file, dir, charset)?;
    let attachments = read_attachments(file, dir, depth, charset)?;

    Ok(Message {
        subject,
        message_id,
        in_reply_to,
        from: Address::new(from_name, from_email),
        to,
        cc,
        bcc,
        date,
        received,
        headers,
        header_block,
        body,
        attachments,
        read: flags & MSG_FLAG_READ != 0,
        draft: flags & MSG_FLAG_UNSENT != 0,
        importance: match long(&props, PR_IMPORTANCE) {
            Some(0) => Importance::Low,
            Some(2) => Importance::High,
            _ => Importance::Normal,
        },
        flag: if fixed(&props, PR_COMPLETE).is_some_and(|b| b[0] != 0) {
            FlagStatus::Complete
        } else if long(&props, PR_FLAG_STATUS).is_some_and(|f| f != 0) {
            FlagStatus::Flagged
        } else {
            FlagStatus::None
        },
        ..Message::default()
    })
}

#[cfg(test)]
#[path = "msg_tests.rs"]
mod tests;
