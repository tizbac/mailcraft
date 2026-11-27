// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! One day of a series changed or skipped, written into the document a calendar server sent
//! (#209 D6): the exception is a `VEVENT` of the series' own `UID` with a `RECURRENCE-ID` of the
//! day's original start; a day the series no longer has is an `EXDATE` on the master. Every other
//! line of the document is kept as it came.

use mailcraft_model::Event;
use mailcraft_time::{Timestamp, Zone};

use crate::WriteOptions;
use crate::patch::{Group, groups, paths, value};
use crate::values::{format_date, format_local, format_utc};

/// The top-level `VEVENT` blocks of a document: their `BEGIN` and `END` group indices.
fn vevents(groups: &[Group<'_>], paths: &[Vec<String>]) -> Vec<(usize, usize)> {
    let is_endpoint = |g: &Group<'_>| (g.name == "BEGIN" || g.name == "END") && value(g).eq_ignore_ascii_case("VEVENT");
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    for (i, (g, _)) in groups.iter().zip(paths).enumerate() {
        if g.name == "BEGIN" && is_endpoint(g) {
            open = Some(i);
        } else if g.name == "END"
            && is_endpoint(g)
            && let Some(from) = open.take()
        {
            out.push((from, i));
        }
    }
    out
}

/// A property of a block, from its first line: its name's parameters and its value.
fn property<'a>(groups: &'a [Group<'a>], range: (usize, usize), name: &str) -> Option<(&'a str, &'a str)> {
    for g in groups.get(range.0..=range.1)? {
        if g.name != name {
            continue;
        }
        let line = g.lines.first()?;
        let (head, found) = line.split_once(':')?;
        return Some((head.strip_prefix(name).unwrap_or(""), found.trim()));
    }
    None
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push_str("\r\n");
}

/// The document `raw` with one occurrence of its series changed or skipped (#209 D6): `was` is the
/// occurrence as it was (its original start names it), `one` the occurrence as it is now, or `None`
/// for a day the series no longer has. A change is a `VEVENT` of the series' own `UID` with a
/// `RECURRENCE-ID` of the original start, and the fields of the occurrence; a skipped day is an
/// `EXDATE` on the master, and its exception (if one was) goes. `None` when the document isn't one
/// series that can be written back.
pub fn patch_occurrence(
    raw: &str,
    was: &Event,
    one: Option<&Event>,
    offset_at: &dyn Fn(Timestamp) -> i32,
    opts: &WriteOptions,
    zone: Option<Zone>,
) -> Option<String> {
    let sent = groups(raw)?;
    let sent_paths = paths(&sent)?;
    let blocks = vevents(&sent, &sent_paths);
    if blocks.is_empty() {
        return None;
    }
    // The series: the event of the document without a RECURRENCE-ID.
    let master = *blocks.iter().find(|range| property(&sent, **range, "RECURRENCE-ID").is_none())?;
    let uid = property(&sent, master, "UID")?.1.to_string();
    let (start_params, start_value) = property(&sent, master, "DTSTART")?;
    let all_day = start_params.to_ascii_uppercase().contains("VALUE=DATE") || start_value.len() == 8;
    let utc = start_value.ends_with('Z');
    // The day's original start, named as the master's `DTSTART` names its times.
    let (rid_params, start_text) = if all_day {
        (String::from(";VALUE=DATE"), format_date(was.start.date))
    } else if utc {
        (String::new(), format_utc(was.instants(offset_at).0))
    } else {
        (start_params.to_string(), format_local(was.start))
    };
    let rid = format!("RECURRENCE-ID{rid_params}:{start_text}");
    let exdate = format!("EXDATE{rid_params}:{start_text}");
    // The exception this day had, if any: an event of the same UID with the same RECURRENCE-ID.
    let existing = blocks.iter().copied().find(|range| {
        *range != master
            && property(&sent, *range, "RECURRENCE-ID").is_some_and(|(_, v)| v == start_text)
            && property(&sent, *range, "UID").is_none_or(|(_, u)| u == uid)
    });
    // What the change is: a new exception of the series' own UID, or nothing (a skipped day).
    let mut exception_lines: Vec<String> = Vec::new();
    if let Some(one) = one {
        let mut exception = one.clone();
        exception.uid = uid.clone();
        exception.recurrence = None;
        exception.exceptions.clear();
        let written = crate::write_in(&[&exception], &[], opts, zone);
        let written_groups = groups(&written)?;
        let written_paths = paths(&written_groups)?;
        let block = *vevents(&written_groups, &written_paths).first()?;
        let lines: Vec<String> = written_groups[block.0..=block.1].iter().flat_map(|g| g.lines.iter().map(|l| (*l).to_string())).collect();
        if lines.is_empty() {
            return None;
        }
        // The exception's times are named as the master's `DTSTART` names its own: a date, a UTC
        // time, or a time in the series' zone, so the two are one series' occurrences.
        let (from, to) = one.instants(offset_at);
        let (start_line, end_line) = if all_day {
            let last = if one.end.date > one.start.date { one.end.date } else { one.start.date.add_days(1) };
            (format!("DTSTART;VALUE=DATE:{}", format_date(one.start.date)), format!("DTEND;VALUE=DATE:{}", format_date(last)))
        } else if utc {
            (format!("DTSTART:{}", format_utc(from)), format!("DTEND:{}", format_utc(to)))
        } else {
            (format!("DTSTART{start_params}:{}", format_local(one.start)), format!("DTEND{start_params}:{}", format_local(one.end)))
        };
        let mut named: Vec<String> = Vec::with_capacity(lines.len() + 2);
        for line in lines {
            // (The times and how long it lasts follow what the master's document says; the
            // exception names the day it replaces right after its `BEGIN`.)
            if line.starts_with("DTSTART") || line.starts_with("DTEND") || line.starts_with("DURATION") {
                continue;
            }
            if line == "BEGIN:VEVENT" {
                named.push(line);
                named.push(rid.clone());
                named.push(start_line.clone());
                named.push(end_line.clone());
            } else {
                named.push(line);
            }
        }
        exception_lines = named;
    }
    let mut out = String::with_capacity(raw.len().saturating_add(exception_lines.iter().map(|l| l.len() + 2).sum::<usize>()));
    let mut exdate_written = false;
    let mut exception_written = false;
    for (i, g) in sent.iter().enumerate() {
        // The exception this day had goes: it is the one being replaced, or a skipped day's.
        if existing.is_some_and(|range| i >= range.0 && i <= range.1) {
            continue;
        }
        // A skipped day that the master already names in an `EXDATE`: the same line, one more day.
        if one.is_none() && !exdate_written && g.name == "EXDATE" && i > master.0 && i < master.1 {
            let params = g.lines.first().and_then(|l| l.split_once(':')).map(|(h, _)| h.strip_prefix("EXDATE").unwrap_or(""));
            if params == Some(rid_params.as_str()) {
                for (n, line) in g.lines.iter().enumerate() {
                    if n + 1 == g.lines.len() {
                        push_line(&mut out, &format!("{line},{start_text}"));
                    } else {
                        push_line(&mut out, line);
                    }
                }
                exdate_written = true;
                continue;
            }
        }
        if one.is_none() && !exdate_written && i == master.1 {
            push_line(&mut out, &exdate);
            exdate_written = true;
        }
        if !exception_lines.is_empty() && !exception_written && g.name == "END" && value(g).eq_ignore_ascii_case("VCALENDAR") {
            for line in &exception_lines {
                push_line(&mut out, line);
            }
            exception_written = true;
        }
        for line in &g.lines {
            push_line(&mut out, line);
        }
    }
    // (An exception goes in before the document's own end; a document with none to put it in is
    // no document to write back.)
    if !exception_lines.is_empty() && !exception_written {
        return None;
    }
    Some(out)
}

#[cfg(test)]
#[path = "patch_occurrence_tests.rs"]
mod tests;
