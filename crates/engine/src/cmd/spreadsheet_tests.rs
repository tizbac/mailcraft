// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Excel's "XML Spreadsheet" clipboard data as the table a message pastes.

use super::to_html;

const XML: &str = r#"<?xml version="1.0"?>
<?mso-application progid="Excel.Sheet"?>
<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet"
 xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
 <Worksheet ss:Name="Sheet1">
  <Table ss:ExpandedColumnCount="4" ss:ExpandedRowCount="4">
   <Row><Cell><Data ss:Type="String">Name</Data></Cell><Cell><Data ss:Type="String">Qty</Data></Cell></Row>
   <Row><Cell><Data ss:Type="String">Screws</Data></Cell><Cell><Data ss:Type="Number">42</Data></Cell></Row>
   <Row ss:Index="4"><Cell ss:Index="3"><Data ss:Type="String">late</Data></Cell></Row>
  </Table>
 </Worksheet>
</Workbook>"#;

#[test]
fn a_spreadsheet_becomes_the_table_its_cells_make() {
    let html = to_html(XML).expect("a table");
    assert!(html.starts_with("<table><tr><td>Name</td><td>Qty</td></tr>"), "{html}");
    assert!(html.contains("<tr><td>Screws</td><td>42</td></tr>"), "{html}");
    // `ss:Index` places a row and a cell where they were in the sheet; the ones skipped are empty.
    assert!(html.contains("<tr></tr><tr><td></td><td></td><td>late</td></tr></table>"), "{html}");
}

#[test]
fn cells_keep_their_spans_and_their_text_is_escaped() {
    let xml = r#"<Workbook xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet><Table>
      <Row><Cell ss:MergeAcross="1" ss:MergeDown="1"><Data>a &amp; b &lt;c&gt;</Data></Cell></Row>
      <Row><Cell><Data>next</Data></Cell></Row>
    </Table></Worksheet></Workbook>"#;
    let html = to_html(xml).expect("a table");
    assert!(html.contains("<td colspan=\"2\" rowspan=\"2\">a &amp; b &lt;c&gt;</td>"), "{html}");
}

#[test]
fn no_table_or_nonsense_is_no_html() {
    assert_eq!(to_html("<Workbook><Worksheet></Worksheet></Workbook>"), None);
    assert_eq!(to_html("<not xml"), None);
    assert_eq!(to_html(""), None);
}
