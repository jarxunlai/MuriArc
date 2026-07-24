use std::{
    io::{BufReader, Cursor},
    path::Component,
};

use muriarc_core::AiConversationSourceKind;
use muriarc_importer::{TabularData, read_xlsx};
use quick_xml::{Reader as XmlReader, events::Event as XmlEvent};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zip::{CompressionMethod, ZipArchive};

pub const MAX_AI_SOURCE_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_AI_SOURCE_ROWS: usize = 200;
pub const MAX_AI_SOURCE_COLUMNS: usize = 128;
pub const MAX_AI_SOURCE_VISION_ASSETS: usize = 8;
const MAX_AI_SOURCE_DELIMITED_SCAN_BYTES: usize = MAX_AI_SOURCE_TEXT_BYTES + 64 * 1024;
const MAX_AI_SOURCE_JSON_BYTES: usize = 1024 * 1024;
const MAX_AI_SOURCE_XLSX_BYTES: usize = 32 * 1024 * 1024;
const MAX_AI_SOURCE_XLSX_ENTRIES: usize = 2_048;
const MAX_AI_SOURCE_XLSX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_AI_SOURCE_XLSX_WORKSHEET_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AI_SOURCE_XLSX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AI_SOURCE_XLSX_COMPRESSION_RATIO: u64 = 200;
const MIN_AI_SOURCE_XLSX_RATIO_CHECK_BYTES: u64 = 64 * 1024;
const MAX_AI_SOURCE_XLSX_ROWS: u32 = 100_000;
const MAX_AI_SOURCE_XLSX_COLUMNS: u32 = 4_096;
const MAX_AI_SOURCE_XLSX_RANGE_CELLS: u64 = 2_000_000;
const MAX_AI_SOURCE_XLSX_CELL_ELEMENTS: usize = 250_000;
const MAX_AI_SOURCE_PDF_BYTES: usize = 16 * 1024 * 1024;
const MAX_AI_SOURCE_PDF_PAGES: usize = 200;
const MAX_AI_SOURCE_PDF_OBJECTS: usize = 100_000;
const MAX_AI_SOURCE_VISION_BYTES: usize = 10 * 1024 * 1024;
const MAX_AI_SOURCE_VISION_PIXELS: i64 = 100_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSourceVisionAsset {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// Bounded, inert material derived from one immutable AI source.
///
/// The parser never evaluates spreadsheet formulas, follows external links or
/// executes PDF actions. Callers keep the original attachment as the source of
/// truth and use this value only as reviewable model context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiSourceMaterial {
    Text {
        text: String,
        truncated: bool,
    },
    Table {
        sheet_name: String,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        /// Exact for a fully parsed table. For a truncated delimited source,
        /// this is the observed lower bound (retained rows plus at least one).
        total_rows: usize,
        truncated: bool,
    },
    Image {
        media_type: String,
    },
    ScannedPdf {
        /// Textless PDFs must be handled by the bounded vision path. This
        /// marker prevents a text parser failure from being mistaken for an
        /// empty document.
        requires_vision: bool,
    },
}

#[derive(Debug, Error)]
pub enum AiSourceMaterialError {
    #[error("the AI source kind and file extension do not agree")]
    KindMismatch,
    #[error("the AI source text is not valid UTF-8")]
    InvalidUtf8,
    #[error("the AI source table could not be parsed")]
    InvalidTable,
    #[error("the AI source PDF could not be parsed")]
    InvalidPdf,
    #[error("the AI source JSON could not be parsed")]
    InvalidJson,
    #[error("the AI source exceeds a safe parsing limit: {0}")]
    ResourceLimitExceeded(&'static str),
}

pub fn extract_ai_source_material(
    kind: AiConversationSourceKind,
    file_name: &str,
    media_type: Option<&str>,
    bytes: &[u8],
) -> Result<AiSourceMaterial, AiSourceMaterialError> {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match kind {
        AiConversationSourceKind::Spreadsheet if extension == "xlsx" => {
            preflight_xlsx(bytes)?;
            let table =
                read_xlsx(Cursor::new(bytes)).map_err(|_| AiSourceMaterialError::InvalidTable)?;
            Ok(bounded_table(table))
        }
        AiConversationSourceKind::DelimitedText if extension == "csv" => {
            bounded_delimited(bytes, b',', "csv")
        }
        AiConversationSourceKind::DelimitedText if extension == "tsv" => {
            bounded_delimited(bytes, b'\t', "tsv")
        }
        AiConversationSourceKind::Text if matches!(extension.as_str(), "txt" | "md") => {
            bounded_text(bytes)
        }
        AiConversationSourceKind::Text if extension == "json" => bounded_json(bytes),
        AiConversationSourceKind::Pdf if extension == "pdf" => bounded_pdf(bytes),
        AiConversationSourceKind::Image
            if matches!(
                (extension.as_str(), normalized_media_type(media_type)),
                ("png", Some("image/png"))
                    | ("jpg" | "jpeg", Some("image/jpeg"))
                    | ("tif" | "tiff", Some("image/tiff"))
            ) =>
        {
            let media_type = normalized_media_type(media_type).expect("matched image media type");
            validate_image(media_type, bytes)?;
            Ok(AiSourceMaterial::Image {
                media_type: media_type.to_owned(),
            })
        }
        _ => Err(AiSourceMaterialError::KindMismatch),
    }
}

/// Returns bounded image inputs for a direct image or a textless scanned PDF.
///
/// PDF support intentionally extracts only embedded JPEG page images. It does
/// not execute page content, JavaScript, external actions or arbitrary
/// decoders. PDFs whose pages are not represented by safe bounded JPEG
/// XObjects remain reviewable sources but cannot silently enter the model.
pub fn extract_ai_source_vision_assets(
    kind: AiConversationSourceKind,
    media_type: Option<&str>,
    bytes: &[u8],
) -> Result<Vec<AiSourceVisionAsset>, AiSourceMaterialError> {
    match kind {
        AiConversationSourceKind::Image => {
            let media_type =
                normalized_media_type(media_type).ok_or(AiSourceMaterialError::KindMismatch)?;
            if !matches!(media_type, "image/png" | "image/jpeg" | "image/tiff") {
                return Err(AiSourceMaterialError::KindMismatch);
            }
            validate_image(media_type, bytes)?;
            Ok(vec![AiSourceVisionAsset {
                media_type: media_type.to_owned(),
                bytes: bytes.to_vec(),
            }])
        }
        AiConversationSourceKind::Pdf => extract_pdf_jpeg_images(bytes),
        _ => Ok(Vec::new()),
    }
}

fn preflight_xlsx(bytes: &[u8]) -> Result<(), AiSourceMaterialError> {
    if bytes.is_empty() || bytes.len() > MAX_AI_SOURCE_XLSX_BYTES {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "XLSX archive bytes",
        ));
    }
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|_| AiSourceMaterialError::InvalidTable)?;
    if archive.is_empty() || archive.len() > MAX_AI_SOURCE_XLSX_ENTRIES {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "XLSX ZIP entry count",
        ));
    }

    let mut total_size = 0_u64;
    let mut worksheet_indexes = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index_raw(index)
            .map_err(|_| AiSourceMaterialError::InvalidTable)?;
        let name = file.name();
        if file.encrypted() {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "encrypted XLSX ZIP entry",
            ));
        }
        if file.is_symlink()
            || file.enclosed_name().is_none()
            || name.contains('\\')
            || std::path::Path::new(name)
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "unsafe XLSX ZIP entry path",
            ));
        }
        if !matches!(
            file.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "unsupported XLSX ZIP compression",
            ));
        }

        let size = file.size();
        let compressed_size = file.compressed_size();
        if size > MAX_AI_SOURCE_XLSX_ENTRY_BYTES {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "XLSX ZIP entry bytes",
            ));
        }
        total_size =
            total_size
                .checked_add(size)
                .ok_or(AiSourceMaterialError::ResourceLimitExceeded(
                    "XLSX total uncompressed bytes",
                ))?;
        if total_size > MAX_AI_SOURCE_XLSX_TOTAL_BYTES {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "XLSX total uncompressed bytes",
            ));
        }
        if size >= MIN_AI_SOURCE_XLSX_RATIO_CHECK_BYTES
            && (compressed_size == 0
                || size > compressed_size.saturating_mul(MAX_AI_SOURCE_XLSX_COMPRESSION_RATIO))
        {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "XLSX ZIP compression ratio",
            ));
        }

        let normalized_name = name.to_ascii_lowercase();
        if normalized_name.ends_with("vbaproject.bin")
            || normalized_name.starts_with("xl/externallinks/")
            || normalized_name.starts_with("xl/activex/")
            || normalized_name.starts_with("xl/embeddings/")
        {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "active or externally linked XLSX content",
            ));
        }
        if normalized_name.starts_with("xl/worksheets/") && normalized_name.ends_with(".xml") {
            if size > MAX_AI_SOURCE_XLSX_WORKSHEET_BYTES {
                return Err(AiSourceMaterialError::ResourceLimitExceeded(
                    "XLSX worksheet XML bytes",
                ));
            }
            worksheet_indexes.push(index);
        }
    }

    if worksheet_indexes.is_empty() {
        return Err(AiSourceMaterialError::InvalidTable);
    }
    for index in worksheet_indexes {
        let file = archive
            .by_index(index)
            .map_err(|_| AiSourceMaterialError::InvalidTable)?;
        validate_worksheet_xml(BufReader::new(file))?;
    }
    Ok(())
}

#[derive(Default)]
struct WorksheetBounds {
    min_row: Option<u32>,
    max_row: Option<u32>,
    min_column: Option<u32>,
    max_column: Option<u32>,
}

impl WorksheetBounds {
    fn observe(&mut self, row: u32, column: u32) -> Result<(), AiSourceMaterialError> {
        validate_cell_position(row, column)?;
        self.min_row = Some(self.min_row.map_or(row, |current| current.min(row)));
        self.max_row = Some(self.max_row.map_or(row, |current| current.max(row)));
        self.min_column = Some(
            self.min_column
                .map_or(column, |current| current.min(column)),
        );
        self.max_column = Some(
            self.max_column
                .map_or(column, |current| current.max(column)),
        );
        let rows = u64::from(
            self.max_row
                .expect("row maximum exists")
                .saturating_sub(self.min_row.expect("row minimum exists"))
                + 1,
        );
        let columns = u64::from(
            self.max_column
                .expect("column maximum exists")
                .saturating_sub(self.min_column.expect("column minimum exists"))
                + 1,
        );
        if rows.saturating_mul(columns) > MAX_AI_SOURCE_XLSX_RANGE_CELLS {
            return Err(AiSourceMaterialError::ResourceLimitExceeded(
                "XLSX sparse worksheet range",
            ));
        }
        Ok(())
    }
}

fn validate_worksheet_xml<R: std::io::BufRead>(reader: R) -> Result<(), AiSourceMaterialError> {
    let mut xml = XmlReader::from_reader(reader);
    let mut buffer = Vec::with_capacity(8 * 1024);
    let mut bounds = WorksheetBounds::default();
    let mut cell_elements = 0_usize;
    loop {
        buffer.clear();
        match xml
            .read_event_into(&mut buffer)
            .map_err(|_| AiSourceMaterialError::InvalidTable)?
        {
            XmlEvent::Start(element) | XmlEvent::Empty(element) => {
                validate_worksheet_element(&element, &mut bounds, &mut cell_elements)?;
            }
            XmlEvent::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn validate_worksheet_element(
    element: &quick_xml::events::BytesStart<'_>,
    bounds: &mut WorksheetBounds,
    cell_elements: &mut usize,
) -> Result<(), AiSourceMaterialError> {
    match element.local_name().as_ref() {
        b"dimension" => {
            let reference =
                xml_attribute(element, b"ref")?.ok_or(AiSourceMaterialError::InvalidTable)?;
            validate_cell_range(&reference)?;
        }
        b"row" => {
            if let Some(row) = xml_attribute(element, b"r")? {
                let row = parse_positive_u32(&row)?;
                if row > MAX_AI_SOURCE_XLSX_ROWS {
                    return Err(AiSourceMaterialError::ResourceLimitExceeded(
                        "XLSX worksheet row index",
                    ));
                }
            }
        }
        b"c" => {
            *cell_elements = cell_elements.saturating_add(1);
            if *cell_elements > MAX_AI_SOURCE_XLSX_CELL_ELEMENTS {
                return Err(AiSourceMaterialError::ResourceLimitExceeded(
                    "XLSX worksheet cell count",
                ));
            }
            if let Some(reference) = xml_attribute(element, b"r")? {
                let (row, column) = parse_cell_reference(&reference)?;
                bounds.observe(row, column)?;
            }
        }
        b"col" => {
            for key in [b"min".as_slice(), b"max".as_slice()] {
                if let Some(value) = xml_attribute(element, key)? {
                    let column = parse_positive_u32(&value)?;
                    if column > MAX_AI_SOURCE_XLSX_COLUMNS {
                        return Err(AiSourceMaterialError::ResourceLimitExceeded(
                            "XLSX worksheet column index",
                        ));
                    }
                }
            }
        }
        b"mergeCell" => {
            let reference =
                xml_attribute(element, b"ref")?.ok_or(AiSourceMaterialError::InvalidTable)?;
            validate_cell_range(&reference)?;
        }
        _ => {}
    }
    Ok(())
}

fn xml_attribute(
    element: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
) -> Result<Option<Vec<u8>>, AiSourceMaterialError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|_| AiSourceMaterialError::InvalidTable)?;
        if attribute.key.local_name().as_ref() == key {
            return Ok(Some(attribute.value.into_owned()));
        }
    }
    Ok(None)
}

fn validate_cell_range(value: &[u8]) -> Result<(), AiSourceMaterialError> {
    let value = std::str::from_utf8(value).map_err(|_| AiSourceMaterialError::InvalidTable)?;
    let mut parts = value.split(':');
    let start = parse_cell_reference(
        parts
            .next()
            .ok_or(AiSourceMaterialError::InvalidTable)?
            .as_bytes(),
    )?;
    let end = parts
        .next()
        .map(|part| parse_cell_reference(part.as_bytes()))
        .transpose()?
        .unwrap_or(start);
    if parts.next().is_some() || end.0 < start.0 || end.1 < start.1 {
        return Err(AiSourceMaterialError::InvalidTable);
    }
    validate_cell_position(start.0, start.1)?;
    validate_cell_position(end.0, end.1)?;
    let rows = u64::from(end.0 - start.0 + 1);
    let columns = u64::from(end.1 - start.1 + 1);
    if rows.saturating_mul(columns) > MAX_AI_SOURCE_XLSX_RANGE_CELLS {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "XLSX declared worksheet range",
        ));
    }
    Ok(())
}

fn parse_cell_reference(value: &[u8]) -> Result<(u32, u32), AiSourceMaterialError> {
    let value = std::str::from_utf8(value).map_err(|_| AiSourceMaterialError::InvalidTable)?;
    let value = value.as_bytes();
    let mut index = usize::from(value.first() == Some(&b'$'));
    let mut column = 0_u32;
    let mut letters = 0_usize;
    while let Some(character) = value.get(index).copied() {
        if !character.is_ascii_alphabetic() {
            break;
        }
        column = column
            .checked_mul(26)
            .and_then(|current| {
                current.checked_add(u32::from(character.to_ascii_uppercase() - b'A') + 1)
            })
            .ok_or(AiSourceMaterialError::InvalidTable)?;
        index += 1;
        letters += 1;
    }
    if letters == 0 {
        return Err(AiSourceMaterialError::InvalidTable);
    }
    if value.get(index) == Some(&b'$') {
        index += 1;
    }
    let row = parse_positive_u32(&value[index..])?;
    if index == value.len() || !value[index..].iter().all(u8::is_ascii_digit) {
        return Err(AiSourceMaterialError::InvalidTable);
    }
    Ok((row, column))
}

fn parse_positive_u32(value: &[u8]) -> Result<u32, AiSourceMaterialError> {
    let value = std::str::from_utf8(value).map_err(|_| AiSourceMaterialError::InvalidTable)?;
    let parsed = value
        .parse::<u32>()
        .map_err(|_| AiSourceMaterialError::InvalidTable)?;
    if parsed == 0 {
        return Err(AiSourceMaterialError::InvalidTable);
    }
    Ok(parsed)
}

fn validate_cell_position(row: u32, column: u32) -> Result<(), AiSourceMaterialError> {
    if row > MAX_AI_SOURCE_XLSX_ROWS || column > MAX_AI_SOURCE_XLSX_COLUMNS {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "XLSX worksheet cell position",
        ));
    }
    Ok(())
}

fn bounded_text(bytes: &[u8]) -> Result<AiSourceMaterial, AiSourceMaterialError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AiSourceMaterialError::InvalidUtf8)?;
    let (text, truncated) = sanitize_and_truncate(text, MAX_AI_SOURCE_TEXT_BYTES);
    Ok(AiSourceMaterial::Text { text, truncated })
}

fn bounded_json(bytes: &[u8]) -> Result<AiSourceMaterial, AiSourceMaterialError> {
    if bytes.len() > MAX_AI_SOURCE_JSON_BYTES {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "JSON input bytes",
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| AiSourceMaterialError::InvalidJson)?;
    let text =
        serde_json::to_string_pretty(&value).map_err(|_| AiSourceMaterialError::InvalidJson)?;
    let (text, truncated) = truncate_utf8(&text, MAX_AI_SOURCE_TEXT_BYTES);
    Ok(AiSourceMaterial::Text { text, truncated })
}

fn bounded_pdf(bytes: &[u8]) -> Result<AiSourceMaterial, AiSourceMaterialError> {
    preflight_pdf(bytes)?;
    let text =
        pdf_extract::extract_text_from_mem(bytes).map_err(|_| AiSourceMaterialError::InvalidPdf)?;
    let (text, truncated) = sanitize_and_truncate(&text, MAX_AI_SOURCE_TEXT_BYTES);
    if text.trim().is_empty() {
        return Ok(AiSourceMaterial::ScannedPdf {
            requires_vision: true,
        });
    }
    Ok(AiSourceMaterial::Text { text, truncated })
}

fn extract_pdf_jpeg_images(
    bytes: &[u8],
) -> Result<Vec<AiSourceVisionAsset>, AiSourceMaterialError> {
    let document = preflight_pdf(bytes)?;
    let mut assets = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for page_id in document.get_pages().into_values() {
        let images = document
            .get_page_images(page_id)
            .map_err(|_| AiSourceMaterialError::InvalidPdf)?;
        for image in images {
            if assets.len() == MAX_AI_SOURCE_VISION_ASSETS {
                return Ok(assets);
            }
            if !seen.insert(image.id)
                || image.width <= 0
                || image.height <= 0
                || image
                    .width
                    .checked_mul(image.height)
                    .is_none_or(|pixels| pixels > MAX_AI_SOURCE_VISION_PIXELS)
                || image.content.is_empty()
                || image.content.len() > MAX_AI_SOURCE_VISION_BYTES
                || !image
                    .filters
                    .as_ref()
                    .is_some_and(|filters| filters.len() == 1 && filters[0] == "DCTDecode")
                || !image.content.starts_with(&[0xff, 0xd8, 0xff])
            {
                continue;
            }
            assets.push(AiSourceVisionAsset {
                media_type: "image/jpeg".to_owned(),
                bytes: image.content.to_vec(),
            });
        }
    }
    Ok(assets)
}

fn preflight_pdf(bytes: &[u8]) -> Result<lopdf::Document, AiSourceMaterialError> {
    if bytes.is_empty() || bytes.len() > MAX_AI_SOURCE_PDF_BYTES {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "PDF input bytes",
        ));
    }
    if !bytes
        .get(..bytes.len().min(1_024))
        .is_some_and(|prefix| prefix.windows(5).any(|window| window == b"%PDF-"))
    {
        return Err(AiSourceMaterialError::InvalidPdf);
    }
    let document =
        lopdf::Document::load_mem(bytes).map_err(|_| AiSourceMaterialError::InvalidPdf)?;
    if document.is_encrypted() {
        return Err(AiSourceMaterialError::InvalidPdf);
    }
    if document.objects.len() > MAX_AI_SOURCE_PDF_OBJECTS {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "PDF object count",
        ));
    }
    if document.get_pages().len() > MAX_AI_SOURCE_PDF_PAGES {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "PDF page count",
        ));
    }
    Ok(document)
}

fn validate_image(media_type: &str, bytes: &[u8]) -> Result<(), AiSourceMaterialError> {
    if bytes.is_empty() || bytes.len() > MAX_AI_SOURCE_VISION_BYTES {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "image input bytes",
        ));
    }
    let (width, height) = match media_type {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" => jpeg_dimensions(bytes),
        "image/tiff" => tiff_dimensions(bytes),
        _ => None,
    }
    .ok_or(AiSourceMaterialError::KindMismatch)?;
    if width == 0
        || height == 0
        || i64::from(width)
            .checked_mul(i64::from(height))
            .is_none_or(|pixels| pixels > MAX_AI_SOURCE_VISION_PIXELS)
    {
        return Err(AiSourceMaterialError::ResourceLimitExceeded(
            "image pixel count",
        ));
    }
    Ok(())
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24
        || bytes[..8] != [137, 80, 78, 71, 13, 10, 26, 10]
        || &bytes[12..16] != b"IHDR"
    {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut index = 2_usize;
    while index < bytes.len() {
        while bytes.get(index) == Some(&0xff) {
            index += 1;
        }
        let marker = *bytes.get(index)?;
        index += 1;
        if matches!(marker, 0x01 | 0xd0..=0xd9) {
            continue;
        }
        let segment_length = usize::from(u16::from_be_bytes([
            *bytes.get(index)?,
            *bytes.get(index + 1)?,
        ]));
        if segment_length < 2 || index.checked_add(segment_length)? > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if segment_length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([
                *bytes.get(index + 3)?,
                *bytes.get(index + 4)?,
            ]));
            let width = u32::from(u16::from_be_bytes([
                *bytes.get(index + 5)?,
                *bytes.get(index + 6)?,
            ]));
            return Some((width, height));
        }
        index += segment_length;
    }
    None
}

fn tiff_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 8 {
        return None;
    }
    let little_endian = match &bytes[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if read_tiff_u16(bytes, 2, little_endian)? != 42 {
        return None;
    }
    let ifd_offset = usize::try_from(read_tiff_u32(bytes, 4, little_endian)?).ok()?;
    let entry_count = usize::from(read_tiff_u16(bytes, ifd_offset, little_endian)?);
    if entry_count > 4_096 {
        return None;
    }
    let mut width = None;
    let mut height = None;
    for index in 0..entry_count {
        let offset = ifd_offset
            .checked_add(2)?
            .checked_add(index.checked_mul(12)?)?;
        let tag = read_tiff_u16(bytes, offset, little_endian)?;
        if !matches!(tag, 256 | 257) {
            continue;
        }
        let value_type = read_tiff_u16(bytes, offset + 2, little_endian)?;
        let count = read_tiff_u32(bytes, offset + 4, little_endian)?;
        if count != 1 {
            return None;
        }
        let value = match value_type {
            3 => u32::from(read_tiff_u16(bytes, offset + 8, little_endian)?),
            4 => read_tiff_u32(bytes, offset + 8, little_endian)?,
            _ => return None,
        };
        match tag {
            256 => width = Some(value),
            257 => height = Some(value),
            _ => unreachable!(),
        }
    }
    Some((width?, height?))
}

fn read_tiff_u16(bytes: &[u8], offset: usize, little_endian: bool) -> Option<u16> {
    let value: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(if little_endian {
        u16::from_le_bytes(value)
    } else {
        u16::from_be_bytes(value)
    })
}

fn read_tiff_u32(bytes: &[u8], offset: usize, little_endian: bool) -> Option<u32> {
    let value: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(if little_endian {
        u32::from_le_bytes(value)
    } else {
        u32::from_be_bytes(value)
    })
}

fn bounded_table(table: TabularData) -> AiSourceMaterial {
    let total_rows = table.rows.len();
    let original_column_count = table.headers.len();
    let mut remaining_bytes = MAX_AI_SOURCE_TEXT_BYTES;
    let mut headers = Vec::new();
    let mut truncated = original_column_count > MAX_AI_SOURCE_COLUMNS;
    for value in table.headers.into_iter().take(MAX_AI_SOURCE_COLUMNS) {
        let (value, cell_truncated) = bounded_cell_with_flag(&value);
        truncated |= cell_truncated;
        if value.len() > remaining_bytes {
            truncated = true;
            break;
        }
        remaining_bytes -= value.len();
        headers.push(value);
    }
    let mut rows = Vec::new();
    for row in table.rows.into_iter().take(MAX_AI_SOURCE_ROWS) {
        if row.len() > headers.len() {
            truncated = true;
        }
        let mut bounded_row = Vec::with_capacity(headers.len());
        let mut fits = true;
        for index in 0..headers.len() {
            let (value, cell_truncated) =
                bounded_cell_with_flag(row.get(index).map_or("", String::as_str));
            truncated |= cell_truncated;
            if value.len() > remaining_bytes {
                truncated = true;
                fits = false;
                break;
            }
            remaining_bytes -= value.len();
            bounded_row.push(value);
        }
        if !fits {
            break;
        }
        rows.push(bounded_row);
    }
    truncated |= total_rows > rows.len() || original_column_count > headers.len();
    AiSourceMaterial::Table {
        sheet_name: bounded_cell(&table.sheet_name),
        headers,
        rows,
        total_rows,
        truncated,
    }
}

fn bounded_delimited(
    bytes: &[u8],
    delimiter: u8,
    sheet_name: &str,
) -> Result<AiSourceMaterial, AiSourceMaterialError> {
    let scan_length = bytes.len().min(MAX_AI_SOURCE_DELIMITED_SCAN_BYTES);
    let input_truncated = scan_length < bytes.len();
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(&bytes[..scan_length]);
    let source_headers = reader
        .headers()
        .map_err(|_| AiSourceMaterialError::InvalidTable)?
        .clone();
    if source_headers.is_empty() {
        return Err(AiSourceMaterialError::InvalidTable);
    }

    let mut remaining_bytes = MAX_AI_SOURCE_TEXT_BYTES;
    let mut truncated = input_truncated || source_headers.len() > MAX_AI_SOURCE_COLUMNS;
    let mut headers = Vec::new();
    for value in source_headers.iter().take(MAX_AI_SOURCE_COLUMNS) {
        let (value, cell_truncated) = bounded_cell_with_flag(strip_bom(value));
        truncated |= cell_truncated;
        if value.len() > remaining_bytes {
            truncated = true;
            break;
        }
        remaining_bytes -= value.len();
        headers.push(value);
    }
    if headers.is_empty() {
        return Err(AiSourceMaterialError::InvalidTable);
    }

    let mut rows = Vec::new();
    let mut observed_rows = 0_usize;
    let mut row_count_incomplete = input_truncated;
    for record in reader.records() {
        if rows.len() == MAX_AI_SOURCE_ROWS {
            truncated = true;
            row_count_incomplete = true;
            observed_rows = observed_rows.saturating_add(1);
            break;
        }
        let record = match record {
            Ok(record) => record,
            Err(_) if input_truncated => {
                truncated = true;
                row_count_incomplete = true;
                break;
            }
            Err(_) => return Err(AiSourceMaterialError::InvalidTable),
        };
        observed_rows = observed_rows.saturating_add(1);
        if record.len() > headers.len() {
            truncated = true;
        }
        let mut row = Vec::with_capacity(headers.len());
        let mut fits = true;
        for index in 0..headers.len() {
            let (value, cell_truncated) =
                bounded_cell_with_flag(record.get(index).unwrap_or_default().trim());
            truncated |= cell_truncated;
            if value.len() > remaining_bytes {
                truncated = true;
                row_count_incomplete = true;
                fits = false;
                break;
            }
            remaining_bytes -= value.len();
            row.push(value);
        }
        if !fits {
            break;
        }
        rows.push(row);
    }

    let total_rows = if row_count_incomplete {
        observed_rows.max(rows.len().saturating_add(1))
    } else {
        observed_rows
    };
    Ok(AiSourceMaterial::Table {
        sheet_name: sheet_name.to_owned(),
        headers,
        rows,
        total_rows,
        truncated,
    })
}

fn strip_bom(value: &str) -> &str {
    value.strip_prefix('\u{feff}').unwrap_or(value)
}

fn bounded_cell(value: &str) -> String {
    bounded_cell_with_flag(value).0
}

fn bounded_cell_with_flag(value: &str) -> (String, bool) {
    const MAX_CELL_BYTES: usize = 1024;
    sanitize_and_truncate(value, MAX_CELL_BYTES)
}

fn sanitize_and_truncate(value: &str, maximum_bytes: usize) -> (String, bool) {
    let mut sanitized = String::with_capacity(value.len().min(maximum_bytes));
    let mut truncated = false;
    for character in value.chars() {
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        if sanitized.len() + character.len_utf8() > maximum_bytes {
            truncated = true;
            break;
        }
        sanitized.push(character);
    }
    (sanitized, truncated)
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> (String, bool) {
    if value.len() <= maximum_bytes {
        return (value.to_owned(), false);
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

fn normalized_media_type(value: Option<&str>) -> Option<&str> {
    value.map(|value| value.split(';').next().unwrap_or(value).trim())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn zip_entry(name: &str, bytes: &[u8], compression: CompressionMethod) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let mut options = SimpleFileOptions::default().compression_method(compression);
        if compression == CompressionMethod::Deflated {
            options = options.compression_level(Some(6));
        }
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn text_is_bounded_and_control_characters_are_removed() {
        let mut text = "a".repeat(MAX_AI_SOURCE_TEXT_BYTES + 8);
        text.insert(1, '\0');
        let material = extract_ai_source_material(
            AiConversationSourceKind::Text,
            "notes.md",
            Some("text/markdown"),
            text.as_bytes(),
        )
        .unwrap();
        let AiSourceMaterial::Text { text, truncated } = material else {
            panic!("expected text")
        };
        assert!(truncated);
        assert!(!text.contains('\0'));
        assert!(text.len() <= MAX_AI_SOURCE_TEXT_BYTES);
    }

    #[test]
    fn tsv_is_parsed_as_inert_bounded_table() {
        let material = extract_ai_source_material(
            AiConversationSourceKind::DelimitedText,
            "animals.tsv",
            Some("text/tab-separated-values"),
            b"display_id\tstatus\nM-1\tactive\n",
        )
        .unwrap();
        let AiSourceMaterial::Table {
            headers,
            rows,
            total_rows,
            truncated,
            ..
        } = material
        else {
            panic!("expected table")
        };
        assert_eq!(headers, ["display_id", "status"]);
        assert_eq!(rows, [["M-1".to_owned(), "active".to_owned()]]);
        assert_eq!(total_rows, 1);
        assert!(!truncated);
    }

    #[test]
    fn oversized_csv_stops_after_the_bounded_row_window() {
        let mut csv = String::from("display_id,status\n");
        for index in 0..(MAX_AI_SOURCE_ROWS + 50) {
            csv.push_str(&format!("M-{index},active\n"));
        }
        let material = extract_ai_source_material(
            AiConversationSourceKind::DelimitedText,
            "animals.csv",
            Some("text/csv"),
            csv.as_bytes(),
        )
        .unwrap();
        let AiSourceMaterial::Table {
            rows,
            total_rows,
            truncated,
            ..
        } = material
        else {
            panic!("expected table")
        };
        assert_eq!(rows.len(), MAX_AI_SOURCE_ROWS);
        assert_eq!(total_rows, MAX_AI_SOURCE_ROWS + 1);
        assert!(truncated);
        assert_eq!(rows.last().unwrap()[0], "M-199");
    }

    #[test]
    fn xlsx_zip_bomb_metadata_is_rejected_before_decompression() {
        let repeated_xml = vec![b' '; 1024 * 1024];
        let bytes = zip_entry(
            "xl/worksheets/sheet1.xml",
            &repeated_xml,
            CompressionMethod::Deflated,
        );
        let result = extract_ai_source_material(
            AiConversationSourceKind::Spreadsheet,
            "bomb.xlsx",
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            &bytes,
        );
        assert!(
            matches!(
                result,
                Err(AiSourceMaterialError::ResourceLimitExceeded(
                    "XLSX ZIP compression ratio"
                ))
            ),
            "{result:?}"
        );
    }

    #[test]
    fn xlsx_malicious_worksheet_dimension_is_rejected_before_calamine() {
        let worksheet = br#"<?xml version="1.0" encoding="UTF-8"?>
            <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
              <dimension ref="A1:XFD1048576"/>
              <sheetData/>
            </worksheet>"#;
        let bytes = zip_entry(
            "xl/worksheets/sheet1.xml",
            worksheet,
            CompressionMethod::Stored,
        );
        assert!(matches!(
            extract_ai_source_material(
                AiConversationSourceKind::Spreadsheet,
                "sparse.xlsx",
                Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
                &bytes,
            ),
            Err(AiSourceMaterialError::ResourceLimitExceeded(
                "XLSX worksheet cell position"
            ))
        ));
    }

    #[test]
    fn ordinary_xlsx_template_survives_the_security_preflight() {
        let bytes = muriarc_importer::animal_import_template_xlsx().unwrap();
        let material = extract_ai_source_material(
            AiConversationSourceKind::Spreadsheet,
            "animals.xlsx",
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            &bytes,
        )
        .unwrap();
        assert!(matches!(material, AiSourceMaterial::Table { .. }));
    }

    #[test]
    fn oversized_pdf_is_rejected_before_pdf_parsing() {
        let mut bytes = vec![0_u8; MAX_AI_SOURCE_PDF_BYTES + 1];
        bytes[..5].copy_from_slice(b"%PDF-");
        assert!(matches!(
            extract_ai_source_material(
                AiConversationSourceKind::Pdf,
                "oversized.pdf",
                Some("application/pdf"),
                &bytes,
            ),
            Err(AiSourceMaterialError::ResourceLimitExceeded(
                "PDF input bytes"
            ))
        ));
    }

    #[test]
    fn oversized_image_dimensions_are_rejected_before_decode() {
        let mut png = vec![0_u8; 24];
        png[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&20_000_u32.to_be_bytes());
        png[20..24].copy_from_slice(&20_000_u32.to_be_bytes());
        assert!(matches!(
            extract_ai_source_material(
                AiConversationSourceKind::Image,
                "oversized.png",
                Some("image/png"),
                &png,
            ),
            Err(AiSourceMaterialError::ResourceLimitExceeded(
                "image pixel count"
            ))
        ));
    }

    #[test]
    fn json_is_validated_before_becoming_model_context() {
        assert!(
            extract_ai_source_material(
                AiConversationSourceKind::Text,
                "unsafe.json",
                Some("application/json"),
                b"{not json}",
            )
            .is_err()
        );
    }

    #[test]
    fn image_extension_and_media_type_must_agree() {
        assert!(
            extract_ai_source_material(
                AiConversationSourceKind::Image,
                "image.png",
                Some("image/jpeg"),
                b"ignored by the material classifier",
            )
            .is_err()
        );
    }
}
