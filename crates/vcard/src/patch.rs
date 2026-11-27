// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A contact changed here, written into the card an address book server sent (#209): only the
//! properties MailCraft changes are replaced; every other line of the card (its photo, its keys,
//! another program's own properties, what vCard 4.0 added) is kept as it came, folded as it came
//! (RFC 6350 section 3.2).
//!
//! A card is one resource, so a document that holds more than one card is not changed here:
//! [`patch`] answers `None` for it, and the host writes nothing.

use std::collections::BTreeSet;

use mailcraft_model::Contact;
use mailcraft_model::contact_sync::ContactField;

/// Most lines a card may have to be changed here (a card is at most a megabyte).
const MAX_LINES: usize = 100_000;

/// One content line with the lines folded into it, as they came: its group (`item1.`), its
/// property name in upper case.
struct Group<'a> {
    lines: Vec<&'a str>,
    group: String,
    name: String,
}

/// Splits a card into its content lines. `None` for something that isn't one card.
fn groups(doc: &str) -> Option<Vec<Group<'_>>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    for (n, line) in doc.split('\n').enumerate() {
        if n >= MAX_LINES {
            return None;
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with([' ', '\t']) {
            out.last_mut()?.lines.push(line);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        // `item1.TEL;TYPE=CELL:…`: the group is what the property is grouped under.
        let prefix = line.split([':', ';']).next().unwrap_or_default();
        let (group, property) = match prefix.rsplit_once('.') {
            Some((group, property)) => (group.to_string(), property),
            None => (String::new(), prefix),
        };
        out.push(Group { lines: vec![line], group, name: property.trim().to_ascii_uppercase() });
    }
    Some(out)
}

/// The value of a group's first line after its name and parameters (`BEGIN:VCARD` → `VCARD`).
fn value<'a>(g: &Group<'a>) -> &'a str {
    g.lines.first().and_then(|l| l.split_once(':')).map(|(_, v)| v.trim()).unwrap_or_default()
}

fn cards(groups: &[Group<'_>]) -> usize {
    groups.iter().filter(|g| g.name == "BEGIN" && value(g).eq_ignore_ascii_case("VCARD")).count()
}

/// The properties of the card that are written anew for these changed fields.
fn replaced(fields: &BTreeSet<ContactField>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut add = |name: &'static str| {
        if !out.contains(&name) {
            out.push(name);
        }
    };
    let named = fields.contains(&ContactField::First)
        || fields.contains(&ContactField::Middle)
        || fields.contains(&ContactField::Last)
        || fields.contains(&ContactField::DisplayName);
    if named {
        // (`FN` is the name the lists show, `N` the parts it is made of: both follow a name.)
        add("N");
        add("FN");
    }
    if fields.contains(&ContactField::Nickname) {
        add("NICKNAME");
    }
    if fields.contains(&ContactField::Company) || fields.contains(&ContactField::Department) {
        add("ORG");
    }
    if fields.contains(&ContactField::Title) {
        add("TITLE");
    }
    if fields.contains(&ContactField::Emails) {
        add("EMAIL");
    }
    if fields.contains(&ContactField::Phones) {
        add("TEL");
    }
    if fields.contains(&ContactField::Addresses) {
        add("ADR");
    }
    if fields.contains(&ContactField::Websites) {
        add("URL");
    }
    if fields.contains(&ContactField::Birthday) {
        add("BDAY");
    }
    if fields.contains(&ContactField::Notes) {
        add("NOTE");
    }
    if fields.contains(&ContactField::Categories) {
        add("CATEGORIES");
    }
    out
}

/// The card `doc` with the properties of `changed` written from `contact`, every other line as it
/// came. `None` when the document isn't one card that can be written back (no `END:VCARD`, more
/// than one card, a line folded with nothing before it).
pub fn patch(doc: &str, contact: &Contact, changed: &BTreeSet<ContactField>) -> Option<String> {
    let as_sent = groups(doc)?;
    if cards(&as_sent) != 1 || !as_sent.iter().any(|g| g.name == "END" && value(g).eq_ignore_ascii_case("VCARD")) {
        return None;
    }
    let replaced = replaced(changed);
    // What the changed fields make of a card, by property: what goes in where the old lines were.
    let fresh = crate::write(&[contact]);
    let fresh = groups(&fresh)?;
    let wanted = |name: &str| replaced.iter().any(|r| r.eq_ignore_ascii_case(name));
    // A group's label goes with the property it labels, in the card as it was and in the new one.
    let mut dropped_groups: Vec<&str> = Vec::new();
    for g in &as_sent {
        if g.name != "X-ABLABEL" && wanted(&g.name) && !g.group.is_empty() {
            dropped_groups.push(&g.group);
        }
    }
    let fresh_groups: Vec<&str> =
        fresh.iter().filter(|g| g.name != "X-ABLABEL" && wanted(&g.name) && !g.group.is_empty()).map(|g| g.group.as_str()).collect();
    let mut fresh_lines: Vec<&str> = fresh
        .iter()
        .filter(|g| g.name != "BEGIN" && g.name != "END" && g.name != "VERSION")
        .filter(|g| wanted(&g.name) || (g.name == "X-ABLABEL" && fresh_groups.contains(&g.group.as_str())))
        .flat_map(|g| g.lines.iter().copied())
        .collect();
    let mut out = String::with_capacity(doc.len());
    let mut inserted = false;
    for g in &as_sent {
        let dropped = wanted(&g.name) || (g.name == "X-ABLABEL" && dropped_groups.contains(&g.group.as_str()));
        if dropped {
            continue;
        }
        if g.name == "END" && value(g).eq_ignore_ascii_case("VCARD") && !inserted {
            for line in fresh_lines.drain(..) {
                out.push_str(line);
                out.push_str("\r\n");
            }
            inserted = true;
        }
        for line in &g.lines {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    inserted.then_some(out)
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
