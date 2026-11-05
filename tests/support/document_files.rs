//! Small, fictional PDF, Office and OpenDocument files built in memory for
//! attachment preview tests and the native attachment fixture.
use std::io::{Cursor, Write};

/// A PDF with one line of Helvetica text per page. `extra` is added to the
/// catalogue, for example an `/OpenAction` script.
pub fn pdf(pages: &[&str], extra: &str) -> Vec<u8> {
    let count = pages.len();
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {extra} >>"),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {count} >>",
            (0..count)
                .map(|i| format!("{} 0 R", 4 + i * 2))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    for (index, line) in pages.iter().enumerate() {
        let stream = format!("BT /F1 28 Tf 72 700 Td ({line}) Tj ET");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + index * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

/// A zip archive with the given parts, deflated like real Office files.
pub fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in parts {
        // OpenDocument requires an uncompressed `mimetype` first.
        let method = if *name == "mimetype" {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default().compression_method(method),
            )
            .expect("start fixture part");
        writer
            .write_all(body.as_bytes())
            .expect("write fixture part");
    }
    writer.finish().expect("finish fixture zip").into_inner()
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;

pub fn docx(extra: &[(&str, &str)]) -> Vec<u8> {
    let document = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Site survey</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Walls &amp; ceilings </w:t></w:r><w:r><w:t>need a second coat.</w:t></w:r></w:p><w:p><w:r><w:t>Kitchen</w:t></w:r><w:r><w:tab/><w:t>3.2 m</w:t></w:r></w:p></w:body></w:document>"#;
    let mut parts = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("word/document.xml", document),
    ];
    parts.extend_from_slice(extra);
    zip(&parts)
}

#[cfg(test)]
pub fn pptx() -> Vec<u8> {
    let slide = |title: &str, body: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p><a:p><a:r><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
        )
    };
    let first = slide("Project timeline", "Survey in March");
    let second = slide("Budget", "Within estimate");
    zip(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("ppt/presentation.xml", "<p:presentation xmlns:p=\"p\"/>"),
        // Out of name order on purpose: slide 10 must follow slide 2.
        ("ppt/slides/slide10.xml", &slide("Questions", "Any time")),
        ("ppt/slides/slide2.xml", &second),
        ("ppt/slides/slide1.xml", &first),
    ])
}

pub fn xlsx(extra: &[(&str, &str)]) -> Vec<u8> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Rooms" sheetId="1" r:id="rId1"/><sheet name="Notes" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
    let rooms = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Room</t></is></c><c r="B1" t="inlineStr"><is><t>Width</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Kitchen</t></is></c><c r="B2"><v>3.2</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>Studio</t></is></c><c r="B3"><v>4</v></c></row><row r="5000"><c r="XFD5000"><v>1</v></c></row></sheetData></worksheet>"#;
    let notes = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Paint arrives Friday</t></is></c></row></sheetData></worksheet>"#;
    let package = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let mut parts = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", package),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", rooms),
        ("xl/worksheets/sheet2.xml", notes),
    ];
    parts.extend_from_slice(extra);
    zip(&parts)
}

#[cfg(test)]
const ODF_TEXT: &str = "application/vnd.oasis.opendocument.text";

#[cfg(test)]
pub fn odt(extra: &[(&str, &str)]) -> Vec<u8> {
    let content = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:h text:outline-level="1">Meeting notes</text:h><text:p>Agreed<text:s text:c="2"/>the <text:span>new</text:span> layout.</text:p><text:p><office:annotation><text:p>Private comment</text:p></office:annotation>Next visit in May.</text:p></office:text></office:body></office:document-content>"#;
    let mut parts = vec![("mimetype", ODF_TEXT), ("content.xml", content)];
    parts.extend_from_slice(extra);
    zip(&parts)
}

#[cfg(test)]
pub fn ods() -> Vec<u8> {
    let content = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Budget"><table:table-row><table:table-cell><text:p>Item</text:p></table:table-cell><table:table-cell><text:p>Cost</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell table:number-columns-repeated="2"><text:p>Paint</text:p></table:table-cell></table:table-row><table:table-row table:number-rows-repeated="1048570"><table:table-cell table:number-columns-repeated="16384"/></table:table-row><table:table-row><table:table-cell><text:p>Total</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
    zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content),
    ])
}

/// A compound (legacy Office) file with the given streams.
#[cfg(test)]
pub fn compound(streams: &[(&str, &[u8])]) -> Vec<u8> {
    let mut file =
        cfb::CompoundFile::create(Cursor::new(Vec::new())).expect("create compound fixture");
    for (path, body) in streams {
        if let Some(parent) = std::path::Path::new(path).parent()
            && parent != std::path::Path::new("/")
        {
            file.create_storage_all(parent).expect("create storage");
        }
        let mut stream = file.create_stream(path).expect("create stream");
        stream.write_all(body).expect("write stream");
    }
    file.flush().expect("flush compound fixture");
    file.into_inner().into_inner()
}

/// BIFF8 workbook globals declaring one sheet of the given type (0 is a
/// worksheet, 1 an Excel 4.0 macro sheet).
#[cfg(test)]
pub fn biff_globals(sheet_type: u8) -> Vec<u8> {
    let mut data = Vec::new();
    // BOF
    data.extend_from_slice(&[0x09, 0x08, 0x10, 0x00]);
    data.extend_from_slice(&[0x00, 0x06, 0x05, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // BOUNDSHEET: position, visibility, type, then a short name.
    let name = b"Sheet1";
    let mut body = vec![0, 0, 0, 0, 0, sheet_type, name.len() as u8, 0];
    body.extend_from_slice(name);
    data.extend_from_slice(&[0x85, 0x00]);
    data.extend_from_slice(&(body.len() as u16).to_le_bytes());
    data.extend_from_slice(&body);
    // EOF
    data.extend_from_slice(&[0x0a, 0x00, 0x00, 0x00]);
    data
}
