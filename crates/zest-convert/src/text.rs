//! Text/data engine: the serde ecosystem plus a hand-rolled, deliberately
//! simple text → PDF writer.
//!
//! ## Conversion matrix
//!
//! The menu (`zest_core::convert_targets`) offers the same eight leaves for
//! every `FileKind::TextData` input; the engine decides which pairs are
//! actually meaningful and rejects the rest with a reason.
//!
//! - **Structured ↔ structured** (`csv`, `json`, `xml`, `yaml`, `toml`):
//!   parsed into a `serde_json::Value` interchange, then re-emitted. Format
//!   limits still apply and fail with a clear reason rather than silently
//!   losing data: CSV input becomes an array of record objects (all fields
//!   stay strings) and CSV output demands exactly that shape back; TOML needs
//!   a top-level object and has no `null`; XML input unwraps the document's
//!   root element, attributes arrive as `@attr` keys, and a text-only element
//!   collapses to its text (mixed content keeps quick-xml's `$text` key);
//!   XML output re-wraps everything in a `<root>` element.
//! - **Anything → txt / md**: passthrough of the raw text — every input here
//!   is already a text file. The one exception is md→txt, which renders the
//!   Markdown to plain text, because that is the point of the conversion.
//! - **Anything → pdf**: the text laid out in fixed-width Courier with basic
//!   pagination (SQU-51). Markdown input goes through the plain-text render
//!   first. The writer is hand-rolled rather than pulled from a crate: the
//!   MVP asks for clean monospaced text and the base-14 fonts need no
//!   embedding, so a full PDF library would be dead weight.
//! - **txt / md → structured**: rejected — prose cannot be parsed into data.
//! - **pdf → anything**: rejected — `pdf` is classified as text data so PDFs
//!   still get a menu, but nothing here reads PDF back.

use super::{ConvertError, Job};
use serde_json::Value;
use std::path::{Path, PathBuf};
use zest_core::Settings;

/// Outputs the engine accepts — the same eight leaves the menu offers for a
/// `FileKind::TextData` selection. `is_supported` decides which are actually
/// reachable from a given input.
pub const OUTPUTS: &[&str] = &["txt", "csv", "json", "xml", "yaml", "toml", "md", "pdf"];

/// The formats that route through the `serde_json::Value` interchange.
const STRUCTURED: &[&str] = &["csv", "json", "xml", "yaml", "toml"];

// US Letter at 10pt Courier (6pt advance) with 0.75in margins: 84 × 57.
const PDF_PAGE_W: u32 = 612;
const PDF_PAGE_H: u32 = 792;
const PDF_MARGIN: u32 = 54;
const PDF_FONT_SIZE: u32 = 10;
const PDF_LINE_HEIGHT: u32 = 12;
const PDF_COLUMNS: usize = 84;
const PDF_ROWS: usize = 57;

pub async fn convert(job: &Job, _settings: &Settings) -> Result<PathBuf, ConvertError> {
    let output_ext = normalize_ext(&job.output_ext);
    if !OUTPUTS.contains(&output_ext.as_str()) {
        return Err(ConvertError::Unsupported(
            "text".to_string(),
            job.output_ext.clone(),
        ));
    }

    let input_ext = normalize_ext(
        job.input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default(),
    );

    if input_ext == "pdf" {
        return Err(ConvertError::Io(format!(
            "{} is a PDF: Zest writes PDFs from text and Markdown but cannot read them",
            job.input.display()
        )));
    }
    if !is_supported(&input_ext, &output_ext) {
        return Err(ConvertError::Unsupported(input_ext, output_ext));
    }

    // `dispatch` resolves the collision-safe sibling before delegating; without
    // it there is nowhere to write.
    let output = job.output.clone().ok_or_else(|| {
        ConvertError::Io(format!(
            "no output path resolved for {}",
            job.input.display()
        ))
    })?;

    let input = job.input.clone();
    let target = output.clone();
    // Parsing and PDF layout are CPU-bound; keep them off the async worker so
    // a large document cannot stall the event loop.
    tokio::task::spawn_blocking(move || transcode(&input, &target, &input_ext, &output_ext))
        .await
        .map_err(|e| ConvertError::Io(format!("text worker failed: {e}")))??;

    Ok(output)
}

/// Canonical spelling for extension lookups (`yml` behaves as `yaml`).
fn normalize_ext(ext: &str) -> String {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "yml" => "yaml".to_string(),
        other => other.to_string(),
    }
}

/// Whether the pair is in the matrix documented at the top of this module.
/// `input` here is never `pdf` (rejected earlier) and is always a text kind.
fn is_supported(input: &str, output: &str) -> bool {
    if STRUCTURED.contains(&output) {
        return STRUCTURED.contains(&input);
    }
    // txt, md and pdf accept every text input.
    true
}

fn transcode(
    input: &Path,
    output: &Path,
    input_ext: &str,
    output_ext: &str,
) -> Result<(), ConvertError> {
    // Guaranteed here too so this function is correct on its own rather than
    // depending on dispatch having done it.
    super::ensure_parent_dir(output)?;

    if STRUCTURED.contains(&output_ext) {
        let value = parse_structured(input, input_ext)?;
        return write_bytes(output, &serialize_structured(&value, output_ext)?);
    }

    let raw = read_text(input)?;
    match output_ext {
        // md → txt/pdf renders the markup away; an md → md leaf is a plain
        // passthrough like every other text target.
        "pdf" => {
            let plain = if input_ext == "md" {
                markdown_to_plain(&raw)
            } else {
                raw
            };
            write_bytes(output, &render_pdf(&plain))
        }
        "txt" => {
            let plain = if input_ext == "md" {
                markdown_to_plain(&raw)
            } else {
                raw
            };
            write_bytes(output, plain.as_bytes())
        }
        _ => write_bytes(output, raw.as_bytes()),
    }
}

fn read_text(path: &Path) -> Result<String, ConvertError> {
    std::fs::read_to_string(path)
        .map_err(|e| ConvertError::Io(format!("cannot read {}: {e}", path.display())))
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), ConvertError> {
    std::fs::write(path, bytes)
        .map_err(|e| ConvertError::Io(format!("cannot write {}: {e}", path.display())))
}

fn parse_structured(path: &Path, ext: &str) -> Result<Value, ConvertError> {
    match ext {
        "csv" => csv_to_value(path),
        "json" => serde_json::from_str(&read_text(path)?)
            .map_err(|e| ConvertError::Io(format!("{} is not valid json: {e}", path.display()))),
        "yaml" => serde_yaml::from_str(&read_text(path)?)
            .map_err(|e| ConvertError::Io(format!("{} is not valid yaml: {e}", path.display()))),
        "toml" => {
            let text = read_text(path)?;
            let value: toml::Value = toml::from_str(&text).map_err(|e| {
                ConvertError::Io(format!("{} is not valid toml: {e}", path.display()))
            })?;
            // Datetimes arrive as their RFC 3339 text — the interchange has
            // no richer type for them.
            serde_json::to_value(&value).map_err(|e| {
                ConvertError::Io(format!("{} could not be read as toml: {e}", path.display()))
            })
        }
        "xml" => {
            let mut value: Value = quick_xml::de::from_str(&read_text(path)?).map_err(|e| {
                ConvertError::Io(format!("{} is not valid xml: {e}", path.display()))
            })?;
            normalize_xml_value(&mut value);
            Ok(value)
        }
        other => Err(ConvertError::Unsupported(
            other.to_string(),
            "structured data".to_string(),
        )),
    }
}

fn serialize_structured(value: &Value, ext: &str) -> Result<Vec<u8>, ConvertError> {
    match ext {
        "json" => Ok(serde_json::to_string_pretty(value)
            .expect("a serde_json::Value always serializes")
            .into_bytes()),
        "yaml" => serde_yaml::to_string(value)
            .map(String::into_bytes)
            .map_err(|e| ConvertError::Io(format!("could not be written as yaml: {e}"))),
        "toml" => {
            // TOML has no root array or scalar, and no `null`. Both surfaces
            // fail with a reason instead of writing a lossy document.
            if !value.is_object() {
                return Err(ConvertError::Io(
                    "toml needs a top-level object; a bare array or scalar cannot be a toml document"
                        .to_string(),
                ));
            }
            toml::to_string_pretty(value)
                .map(String::into_bytes)
                .map_err(|e| ConvertError::Io(format!("could not be written as toml: {e}")))
        }
        "xml" => {
            // quick-xml needs a root tag; a top-level array or scalar is
            // wrapped so the output stays one well-formed document. CSV input
            // therefore lands as `<root><item>…</item>…</root>`.
            let wrapped = serde_json::json!({ "item": value });
            let document = if value.is_object() { value } else { &wrapped };
            quick_xml::se::to_string_with_root("root", document)
                .map(|body| {
                    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{body}").into_bytes()
                })
                .map_err(|e| ConvertError::Io(format!("could not be written as xml: {e}")))
        }
        "csv" => records_to_csv(value),
        other => Err(ConvertError::Unsupported(
            "structured data".to_string(),
            other.to_string(),
        )),
    }
}

/// quick-xml puts every text leaf under `$text`, so `<name>ada</name>` parses
/// as `{"name": {"$text": "ada"}}`. An element whose only content is text is
/// collapsed to the scalar — it serializes back identically — while elements
/// that also carry attributes keep `$text` alongside their `@attr` keys.
fn normalize_xml_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(text) = map.get("$text").filter(|t| !t.is_object() && !t.is_array()) {
                    *value = text.clone();
                    return;
                }
            }
            for child in map.values_mut() {
                normalize_xml_value(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_xml_value),
        _ => {}
    }
}

/// CSV is untyped: every field stays a string. Guessing at numbers would
/// silently change the data on the way back out.
fn csv_to_value(path: &Path) -> Result<Value, ConvertError> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| ConvertError::Io(format!("cannot read {}: {e}", path.display())))?;
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| {
            ConvertError::Io(format!(
                "{} has no readable csv header: {e}",
                path.display()
            ))
        })?
        .iter()
        .map(str::to_string)
        .collect();

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| {
            ConvertError::Io(format!("{} has a malformed csv row: {e}", path.display()))
        })?;
        let object: serde_json::Map<String, Value> = headers
            .iter()
            .cloned()
            .zip(record.iter().map(|field| Value::String(field.to_string())))
            .collect();
        rows.push(Value::Object(object));
    }
    Ok(Value::Array(rows))
}

/// Flat records only: a top-level array of objects whose values are scalars.
/// The header is the union of keys in first-seen order; `null` becomes an
/// empty cell and nested values fail rather than being stringified.
fn records_to_csv(value: &Value) -> Result<Vec<u8>, ConvertError> {
    let rows = value.as_array().ok_or_else(|| {
        ConvertError::Io(
            "csv needs a top-level array of records; pick a format that can hold this shape"
                .to_string(),
        )
    })?;

    let mut headers: Vec<String> = Vec::new();
    for row in rows {
        let record = row.as_object().ok_or_else(|| {
            ConvertError::Io(
                "csv needs a top-level array of records; found a bare scalar instead".to_string(),
            )
        })?;
        for key in record.keys() {
            if !headers.contains(key) {
                headers.push(key.clone());
            }
        }
    }

    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(&headers)
        .map_err(|e| ConvertError::Io(format!("could not write the csv header: {e}")))?;
    for row in rows {
        let record = row.as_object().expect("checked above");
        let fields: Vec<String> = headers
            .iter()
            .map(|key| match record.get(key) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(text)) => Ok(text.clone()),
                Some(Value::Number(number)) => Ok(number.to_string()),
                Some(Value::Bool(flag)) => Ok(flag.to_string()),
                Some(_) => Err(ConvertError::Io(format!(
                    "csv cells must be scalars; {key:?} holds a nested value"
                ))),
            })
            .collect::<Result<_, ConvertError>>()?;
        writer
            .write_record(&fields)
            .map_err(|e| ConvertError::Io(format!("could not write a csv row: {e}")))?;
    }
    writer
        .into_inner()
        .map_err(|e| ConvertError::Io(format!("could not finish the csv output: {e}")))
}

/// A shallow Markdown → plain text pass: headings, emphasis, code spans and
/// fences, links, images, list and quote markers. Tables and anything
/// unrecognised pass through untouched — this is a readability pass, not a
/// renderer — shared by md→txt and md→pdf.
fn markdown_to_plain(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut fenced = false;
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let mut text = trimmed.to_string();
        // ATX heading markers (`# x` → `x`).
        let hashes = text.bytes().take_while(|b| *b == b'#').count();
        if hashes > 0 && text[hashes..].starts_with(' ') {
            text = text[hashes..].trim_start().to_string();
        }
        // Blockquote markers.
        while let Some(rest) = text.strip_prefix('>') {
            text = rest.trim_start().to_string();
        }
        // `* `/`+ ` bullets are normalised to `- `.
        if let Some(rest) = text.strip_prefix("* ").or_else(|| text.strip_prefix("+ ")) {
            text = format!("- {rest}");
        }
        // Task lists keep an ASCII checkbox.
        for checked in ["- [x] ", "- [X] "] {
            if let Some(rest) = text.strip_prefix(checked) {
                text = format!("- [x] {rest}");
                break;
            }
        }
        // A horizontal rule (`---`, `***`, `___`) becomes a blank line.
        if text.len() >= 3
            && matches!(text.chars().next(), Some('-' | '*' | '_'))
            && text.chars().all(|c| c == text.chars().next().unwrap())
        {
            out.push('\n');
            continue;
        }
        out.push_str(&inline_markdown(&text));
        out.push('\n');
    }
    out
}

/// Inline markup: `[text](url)` → `text (url)`, `![alt](src)` → `alt`,
/// `` `code` `` → `code`, and `**`/`__`/`~~` pairs are dropped.
fn inline_markdown(text: &str) -> String {
    let text = rewrite_links_and_images(text)
        .replace("**", "")
        .replace("__", "")
        .replace("~~", "")
        .replace('`', "");
    // A single `*` is dropped only when exactly one side hugs non-space text
    // (`*word*` emphasis); `2 * 3` and word-internal `a*b` survive.
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == '*' {
            let hugs_left = i > 0 && !chars[i - 1].is_whitespace();
            let hugs_right = i + 1 < chars.len() && !chars[i + 1].is_whitespace();
            if hugs_left != hugs_right {
                continue;
            }
        }
        out.push(*c);
    }
    out
}

fn rewrite_links_and_images(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open + 1..].find("](").map(|i| open + 1 + i) else {
            break;
        };
        let Some(paren) = rest[close + 2..].find(')').map(|i| close + 2 + i) else {
            break;
        };
        out.push_str(&rest[..open]);
        // `![alt](src)` keeps just its alt text; `[text](url)` keeps both.
        let image = out.ends_with('!');
        if image {
            out.pop();
        }
        out.push_str(&rest[open + 1..close]);
        let url = &rest[close + 2..paren];
        if !image && !url.is_empty() {
            out.push_str(" (");
            out.push_str(url);
            out.push(')');
        }
        rest = &rest[paren + 1..];
    }
    out.push_str(rest);
    out
}

/// Word-wrap at `PDF_COLUMNS` characters, preferring the last space inside
/// the window and hard-breaking only when a word alone fills the line.
fn wrap_text(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut line: Vec<char> = raw.trim_end().chars().collect();
        while line.len() > PDF_COLUMNS {
            match line[..PDF_COLUMNS].iter().rposition(|c| *c == ' ') {
                Some(space) => {
                    lines.push(line[..space].iter().collect());
                    line = line[space + 1..].to_vec();
                }
                None => {
                    lines.push(line[..PDF_COLUMNS].iter().collect());
                    line = line[PDF_COLUMNS..].to_vec();
                }
            }
        }
        lines.push(line.iter().collect());
    }
    lines
}

/// Minimal PDF 1.4: Letter pages, one Courier base-14 font, one text object
/// per page. Streams are uncompressed so the document doubles as its own
/// debugging output.
fn render_pdf(text: &str) -> Vec<u8> {
    let lines = wrap_text(text);
    let mut pages: Vec<&[String]> = lines.chunks(PDF_ROWS).collect();
    if pages.is_empty() {
        pages.push(&lines[..]); // an empty input still gets one blank page
    }

    // Objects: 1 = catalog, 2 = pages, 3 = font, then per page a page object
    // at 4+2i and its content stream at 5+2i.
    let object_count = 3 + pages.len() * 2;
    let mut offsets = vec![0usize; object_count + 1];
    let mut doc: Vec<u8> = b"%PDF-1.4\n".to_vec();

    push_object(
        &mut doc,
        &mut offsets,
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    );
    let kids: String = (0..pages.len())
        .map(|i| format!("{} 0 R ", 4 + i * 2))
        .collect();
    push_object(
        &mut doc,
        &mut offsets,
        2,
        format!("<< /Type /Pages /Kids [{kids}]/Count {} >>", pages.len()).as_bytes(),
    );
    push_object(
        &mut doc,
        &mut offsets,
        3,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>",
    );

    for (i, page) in pages.iter().enumerate() {
        let page_id = 4 + i * 2;
        let content_id = page_id + 1;
        push_object(
            &mut doc,
            &mut offsets,
            page_id,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PDF_PAGE_W} {PDF_PAGE_H}] \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .as_bytes(),
        );

        let mut content = format!(
            "BT /F1 {PDF_FONT_SIZE} Tf {PDF_LINE_HEIGHT} TL {PDF_MARGIN} {} Td\n",
            PDF_PAGE_H - PDF_MARGIN
        )
        .into_bytes();
        for (n, line) in page.iter().enumerate() {
            if n > 0 {
                content.extend_from_slice(b"T* ");
            }
            content.push(b'(');
            content.extend_from_slice(&pdf_escape(line));
            content.extend_from_slice(b") Tj\n");
        }
        content.extend_from_slice(b"ET\n");

        let mut body = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        body.extend_from_slice(&content);
        body.extend_from_slice(b"endstream");
        push_object(&mut doc, &mut offsets, content_id, &body);
    }

    let xref_pos = doc.len();
    doc.extend_from_slice(format!("xref\n0 {}\n", object_count + 1).as_bytes());
    doc.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        doc.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    doc.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            object_count + 1
        )
        .as_bytes(),
    );
    doc
}

fn push_object(doc: &mut Vec<u8>, offsets: &mut [usize], id: usize, body: &[u8]) {
    offsets[id] = doc.len();
    doc.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
    doc.extend_from_slice(body);
    doc.extend_from_slice(b"\nendobj\n");
}

/// Escape a text line into a PDF literal string. The base-14 fonts use
/// WinAnsi encoding: Latin-1 passes through, anything further degrades to
/// '?' rather than corrupting the stream.
fn pdf_escape(line: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\\' => out.extend_from_slice(b"\\\\"),
            '(' => out.extend_from_slice(b"\\("),
            ')' => out.extend_from_slice(b"\\)"),
            c if (c as u32) <= 0xFF => out.push(c as u8),
            _ => out.push(b'?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Job;
    use std::path::PathBuf;

    /// A scratch directory that removes itself, unique per test and per
    /// process so the parallel test runner cannot collide.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("zest-text-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create scratch dir");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn run(input: &Path, ext: &str) -> Result<PathBuf, ConvertError> {
        let mut job = Job::new(input, ext);
        crate::dispatch(&mut job, &Settings::default()).await
    }

    #[tokio::test]
    async fn json_yaml_toml_json_round_trips_without_loss() {
        let scratch = Scratch::new("round-trip");
        let source = scratch.join("config.json");
        std::fs::write(
            &source,
            r#"{"name":"zest","version":1,"debug":true,"ratio":1.5,"tags":["a","b"],"nested":{"key":"value"}}"#,
        )
        .expect("write json source");
        let original: Value =
            serde_json::from_str(&std::fs::read_to_string(&source).unwrap()).unwrap();

        let yaml = run(&source, "yaml").await.expect("json -> yaml");
        assert_eq!(yaml.extension().unwrap(), "yaml");
        let toml = run(&yaml, "toml").await.expect("yaml -> toml");
        let back = run(&toml, "json").await.expect("toml -> json");

        // The original config.json still exists, so the round-trip lands at
        // the collision-suffixed path rather than overwriting it.
        assert_eq!(back, scratch.join("config (1).json"));
        let round_tripped: Value =
            serde_json::from_str(&std::fs::read_to_string(&back).unwrap()).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[tokio::test]
    async fn csv_round_trips_through_json_as_flat_records() {
        let scratch = Scratch::new("csv-json");
        let source = scratch.join("people.csv");
        std::fs::write(&source, "name,age\nada,36\ngrace,85\n").expect("write csv");

        let json = run(&source, "json").await.expect("csv -> json");
        let records: Value =
            serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(
            records,
            serde_json::json!([
                {"name": "ada", "age": "36"},
                {"name": "grace", "age": "85"},
            ]),
            "csv fields stay strings"
        );

        let csv_out = run(&json, "csv").await.expect("json -> csv");
        // serde_json::Value sorts object keys, so the header comes back in
        // sorted order even though the source listed name first.
        assert_eq!(
            std::fs::read_to_string(&csv_out)
                .unwrap()
                .replace("\r\n", "\n"),
            "age,name\n36,ada\n85,grace\n"
        );
    }

    #[tokio::test]
    async fn xml_maps_to_json_and_back_under_a_root() {
        let scratch = Scratch::new("xml");
        let source = scratch.join("user.xml");
        std::fs::write(&source, "<user><name>ada</name><age>36</age></user>").expect("write xml");

        let json = run(&source, "json").await.expect("xml -> json");
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        // quick-xml unwraps the document's single root element, so `<user>`
        // itself becomes the top-level object.
        assert_eq!(value["name"], "ada");
        assert_eq!(value["age"], "36");

        let xml = run(&json, "xml").await.expect("json -> xml");
        let body = std::fs::read_to_string(&xml).unwrap();
        assert!(body.starts_with("<?xml"), "{body}");
        assert!(body.contains("<name>ada</name>"), "{body}");
    }

    #[tokio::test]
    async fn txt_and_md_pass_through_and_markdown_strips_to_txt() {
        let scratch = Scratch::new("passthrough");
        let txt = scratch.join("notes.txt");
        std::fs::write(&txt, "plain words\n").expect("write txt");

        // txt -> md is a passthrough: the bytes are already valid markdown.
        let md = run(&txt, "md").await.expect("txt -> md");
        assert_eq!(std::fs::read(&md).unwrap(), b"plain words\n");

        // md -> txt is the one text pair that rewrites: it renders the
        // markdown so the .txt reads cleanly.
        std::fs::write(
            &md,
            "# Title\n\n**bold** and `code` plus a [link](https://e)\n",
        )
        .expect("write md");
        let back = run(&md, "txt").await.expect("md -> txt");
        let plain = std::fs::read_to_string(&back).unwrap();
        assert!(plain.contains("Title"), "{plain}");
        assert!(plain.contains("bold and code"), "{plain}");
        assert!(plain.contains("link (https://e)"), "{plain}");
        assert!(!plain.contains("**"), "{plain}");

        // md -> md is a passthrough: the markup must survive untouched.
        let copy = run(&md, "md").await.expect("md -> md");
        assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&md).unwrap());
    }

    #[tokio::test]
    async fn markdown_to_pdf_starts_with_the_pdf_magic_and_paginates() {
        let scratch = Scratch::new("md-pdf");
        let source = scratch.join("readme.md");
        let long = (1..=200)
            .map(|i| format!("line {i} of a long document"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&source, format!("# Title\n\n{long}")).expect("write md");

        let output = run(&source, "pdf").await.expect("md -> pdf");
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"), "output is not a PDF");

        // 201 lines at 57 rows/page must span at least four page objects.
        let needle = b"/Type /Page ";
        let page_objects = pdf
            .windows(needle.len())
            .filter(|window| *window == needle.as_slice())
            .count();
        assert!(
            page_objects >= 4,
            "long input did not paginate: {page_objects} page objects"
        );
    }

    #[tokio::test]
    async fn a_wrapped_word_is_not_split_in_the_pdf() {
        let line = "word ".repeat(20) + "tail";
        let wrapped = wrap_text(&line);
        assert!(wrapped.len() > 1, "line should wrap");
        assert!(wrapped.iter().all(|l| l.len() <= PDF_COLUMNS));
        assert!(wrapped.last().unwrap().ends_with("tail"));
    }

    #[tokio::test]
    async fn unstructured_input_cannot_become_structured() {
        let scratch = Scratch::new("unsupported");
        let prose = scratch.join("notes.txt");
        std::fs::write(&prose, "just some words\n").expect("write txt");

        for ext in ["json", "yaml", "toml", "xml", "csv"] {
            let error = run(&prose, ext)
                .await
                .expect_err("prose cannot become structured data");
            let message = error.to_string();
            assert!(
                message.contains("txt") && message.contains(ext),
                "unclear error for txt -> {ext}: {message}"
            );
        }
    }

    #[tokio::test]
    async fn toml_output_needs_a_top_level_object() {
        let scratch = Scratch::new("toml-array");
        let source = scratch.join("list.json");
        std::fs::write(&source, "[1,2,3]").expect("write json");

        let error = run(&source, "toml")
            .await
            .expect_err("a bare array cannot be a toml document");
        assert!(error.to_string().contains("top-level"), "{error}");
    }

    #[tokio::test]
    async fn csv_output_rejects_nested_records() {
        let scratch = Scratch::new("csv-nested");
        let source = scratch.join("nested.json");
        std::fs::write(&source, r#"[{"a":{"deep":1}}]"#).expect("write json");

        let error = run(&source, "csv")
            .await
            .expect_err("nested values do not fit a csv cell");
        assert!(error.to_string().contains("scalars"), "{error}");
    }

    #[tokio::test]
    async fn pdf_input_is_rejected_with_the_reason() {
        let scratch = Scratch::new("pdf-in");
        let source = scratch.join("doc.pdf");
        std::fs::write(&source, b"%PDF-1.4 fake").expect("write pdf");

        let error = run(&source, "txt")
            .await
            .expect_err("the engine only writes pdf");
        assert!(error.to_string().contains("cannot read"), "{error}");
        assert!(!scratch.join("doc.txt").exists());
    }

    #[tokio::test]
    async fn invalid_input_names_the_file_in_the_error() {
        let scratch = Scratch::new("invalid");
        let source = scratch.join("broken.json");
        std::fs::write(&source, "{not json").expect("write json");

        let error = run(&source, "yaml")
            .await
            .expect_err("malformed json must fail");
        assert!(error.to_string().contains("broken.json"), "{error}");
        assert!(!scratch.join("broken.yaml").exists());
    }

    #[tokio::test]
    async fn yml_input_is_treated_as_yaml() {
        let scratch = Scratch::new("yml");
        let source = scratch.join("data.yml");
        std::fs::write(&source, "key: value\n").expect("write yml");

        let out = run(&source, "json").await.expect("yml -> json");
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(value["key"], "value");
    }
}
