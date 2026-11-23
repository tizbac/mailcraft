// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A grid out of the plain text a spreadsheet's Copy leaves on the clipboard: rows of lines, cells
//! between tabs — and nothing else, so ordinary text is never turned into a table.

use super::{MAX_PASTED_COLUMNS, MAX_PASTED_ROWS, PastedTable};

#[test]
fn rows_of_tabs_become_a_grid_of_the_same_width() {
    let table = PastedTable::from_text("Part\t12,30\nGasket\t4,00\n").unwrap();
    assert_eq!((table.columns(), table.rows().len()), (2, 2));
    assert_eq!(table.plain(), "Part\t12,30\nGasket\t4,00");
    assert!(table.rows().iter().all(|row| row.iter().all(|cell| !cell.header && cell.colspan == 1 && cell.rowspan == 1)));
    // Windows ends a line with \r\n; a Mac with \r.
    let other = PastedTable::from_text("A\tB\r\nC\tD").unwrap();
    assert_eq!(other.plain(), "A\tB\nC\tD");
}

#[test]
fn an_empty_cell_is_still_a_cell() {
    let table = PastedTable::from_text("Qty\t\nGasket\t4,00").unwrap();
    assert_eq!((table.columns(), table.rows().len()), (2, 2));
    assert_eq!(table.rows()[0][1].lines[0][0].text, "");
}

#[test]
fn text_that_is_not_a_range_stays_text() {
    for text in [
        "",
        "one line with a tab\tand another cell",
        "Part\t12,30",
        "Part\t12,30\nGasket",
        "Part\t12,30\nGasket\t4,00\textra",
        "a\nb\nc",
        "\n\n",
        "one\ttwo\n\nthree\tfour",
    ] {
        assert!(PastedTable::from_text(text).is_none(), "{text:?} is not a grid");
    }
}

#[test]
fn a_range_bigger_than_a_table_holds_is_refused() {
    let too_many_rows = "\t\n".repeat(MAX_PASTED_ROWS + 1);
    assert!(PastedTable::from_text(&too_many_rows).is_none(), "more rows than a table holds");
    let wide = "a\tb\tc\t".repeat(MAX_PASTED_COLUMNS + 1);
    assert!(PastedTable::from_text(&format!("{wide}\n{wide}")).is_none(), "more columns than a table holds");
    // Nothing is read for text larger than a message's rich text can be.
    assert!(PastedTable::from_text(&"a\tb\n".repeat(1 << 21)).is_none(), "text of more than a message's worth");
}

#[test]
fn control_characters_in_a_cell_are_taken_out() {
    let table = PastedTable::from_text("a\x01b\tc\x00\nd\te").unwrap();
    assert_eq!(table.plain(), "ab\tc\nd\te");
}
