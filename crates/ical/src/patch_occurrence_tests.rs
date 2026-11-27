// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! One day of a series changed or skipped: the exception's `RECURRENCE-ID`, the master kept, and
//! a skipped day's `EXDATE`.

use mailcraft_model::Event;
use mailcraft_time::{Date, DateTime};

use super::*;

const SERIES: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s1\r\nDTSTAMP:20261101T000000Z\r\nDTSTART:20261123T100000Z\r\nDTEND:20261123T110000Z\r\nRRULE:FREQ=DAILY;COUNT=5\r\nSUMMARY:Serie\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn day(day: u8, hour: u8) -> DateTime {
    DateTime::new(Date::new(2026, 11, day).unwrap(), hour, 0, 0).expect("a time")
}

fn occurrence(day_of: u8, hour: u8, title: &str) -> Event {
    let mut e = Event { title: title.into(), start: day(day_of, hour), end: day(day_of, hour + 1), ..Default::default() };
    if hour == 10 {
        e.end = day(day_of, 11);
    }
    e
}

fn opts() -> WriteOptions {
    WriteOptions { method: String::new(), now: 0 }
}

fn patched(raw: &str, one: Option<&Event>) -> String {
    let was = occurrence(24, 10, "Serie");
    patch_occurrence(raw, &was, one, &|_| 0, &opts(), None).expect("patched")
}

#[test]
fn a_changed_day_becomes_an_exception_of_the_series_own_uid() {
    let one = occurrence(24, 12, "Verschoben");
    let doc = patched(SERIES, Some(&one));
    // The master stays, and the day is an event of the series' UID with its original start.
    assert!(doc.contains("RRULE:FREQ=DAILY;COUNT=5"), "{doc}");
    assert!(doc.contains("RECURRENCE-ID:20261124T100000Z"), "{doc}");
    assert!(doc.contains("SUMMARY:Verschoben"), "{doc}");
    assert!(doc.contains("DTSTART:20261124T120000Z"), "the change's own time:\n{doc}");
    assert_eq!(doc.matches("UID:s1").count(), 2, "the master and its exception");
    assert!(doc.trim_end().ends_with("END:VCALENDAR"));
}

#[test]
fn a_day_changed_twice_replaces_its_exception_and_keeps_one() {
    let first = patched(SERIES, Some(&occurrence(24, 12, "Einmal")));
    let second = patched(&first, Some(&occurrence(24, 14, "Zweimal")));
    assert_eq!(second.matches("RECURRENCE-ID:").count(), 1, "one exception:\n{second}");
    assert!(!second.contains("SUMMARY:Einmal"), "{second}");
    assert!(second.contains("SUMMARY:Zweimal"), "{second}");
    assert_eq!(second.matches("SUMMARY:Serie").count(), 1, "the master stays");
}

#[test]
fn a_skipped_day_is_an_exdate_on_the_master_and_its_exception_goes() {
    let changed = patched(SERIES, Some(&occurrence(24, 12, "Einmal")));
    let skipped = patched(&changed, None);
    assert!(skipped.contains("EXDATE:20261124T100000Z"), "{skipped}");
    assert!(!skipped.contains("RECURRENCE-ID:"), "the exception goes:\n{skipped}");
    assert!(!skipped.contains("SUMMARY:Einmal"));
    assert_eq!(skipped.matches("SUMMARY:Serie").count(), 1);
}

#[test]
fn a_second_skipped_day_joins_the_exdate_already_there() {
    let once = patched(SERIES, None);
    let twice = patched(&once, None);
    assert_eq!(twice.matches("EXDATE:").count(), 1, "one line, two days:\n{twice}");
    assert!(twice.contains("EXDATE:20261124T100000Z,20261124T100000Z"), "{twice}");
}

#[test]
fn an_all_day_series_names_the_day_as_a_date() {
    let series =
        SERIES.replace("DTSTART:20261123T100000Z", "DTSTART;VALUE=DATE:20261123").replace("DTEND:20261123T110000Z", "DTEND;VALUE=DATE:20261124");
    let mut was = Event { start: day(24, 0), end: day(25, 0), all_day: true, ..Default::default() };
    was.title = "Serie".into();
    let mut one = was.clone();
    one.title = "Frei".into();
    let doc = patch_occurrence(&series, &was, Some(&one), &|_| 0, &opts(), None).expect("patched");
    assert!(doc.contains("RECURRENCE-ID;VALUE=DATE:20261124"), "{doc}");
    assert!(doc.contains("SUMMARY:Frei"), "{doc}");
}

#[test]
fn what_is_not_one_series_is_not_changed() {
    assert!(
        patch_occurrence("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", &occurrence(24, 10, "x"), Some(&occurrence(24, 12, "y")), &|_| 0, &opts(), None)
            .is_none()
    );
    let was = occurrence(24, 10, "x");
    assert!(patch_occurrence("not a document", &was, Some(&was), &|_| 0, &opts(), None).is_none());
}
