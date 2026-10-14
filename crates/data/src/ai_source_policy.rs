//! Upload policy for private AI conversation sources, shared by Server and Desktop.
//!
//! The extension whitelist, declared media type agreement and content-signature
//! checks form one security boundary; both runtimes must reject exactly the
//! same files.

use std::path::Path;

use muriarc_core::AiConversationSourceKind;
use thiserror::Error;

use crate::{AttachmentContentKind, AttachmentInspection};

/// Private AI sources expire after this many days of inactivity.
pub const AI_SOURCE_RETENTION_DAYS: i64 = 30;
pub const MAX_AI_SOURCE_FILE_NAME_BYTES: usize = 255;
pub const MAX_AI_SOURCE_MEDIA_TYPE_BYTES: usize = 127;

const XLSX_MEDIA_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const OCTET_STREAM: &str = "application/octet-stream";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AiSourcePolicyError {
    #[error("AI sources accept XLSX, CSV, TSV, TXT, MD, JSON, PDF, PNG, JPEG, and TIFF only")]
    Unsupported,
    #[error("the declared media type does not match the AI source file extension")]
    MediaTypeMismatch,
    #[error("AI source file name is invalid")]
    InvalidFileName,
    #[error("AI source media type is invalid")]
    InvalidMediaType,
}

/// Trims and validates a plain file name; path separators and control
/// characters are rejected so the name can never address another object.
pub fn normalize_ai_source_file_name(value: String) -> Result<String, AiSourcePolicyError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_AI_SOURCE_FILE_NAME_BYTES
        || matches!(value.as_str(), "." | "..")
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        return Err(AiSourcePolicyError::InvalidFileName);
    }
    Ok(value)
}

/// Trims an optional declared media type; blank values mean "not declared".
pub fn normalize_ai_source_media_type(
    value: Option<String>,
) -> Result<Option<String>, AiSourcePolicyError> {
    let value = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if value.as_ref().is_some_and(|value| {
        value.len() > MAX_AI_SOURCE_MEDIA_TYPE_BYTES
            || !value.is_ascii()
            || value.chars().any(char::is_control)
    }) {
        return Err(AiSourcePolicyError::InvalidMediaType);
    }
    Ok(value)
}

/// Classifies an already inspected upload, returning its source kind and the
/// media type to persist. `content` only needs to contain the leading bytes
/// (the XLSX container signature); passing the full file is fine.
pub fn classify_ai_source(
    file_name: &str,
    declared_media_type: Option<&str>,
    inspection: AttachmentInspection,
    content: &[u8],
) -> Result<(AiConversationSourceKind, String), AiSourcePolicyError> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(AiSourcePolicyError::Unsupported)?;
    let opaque = inspection.kind == AttachmentContentKind::Opaque;
    let (kind, canonical_media_type, allowed_media_types): (
        AiConversationSourceKind,
        &'static str,
        &'static [&'static str],
    ) = match extension.as_str() {
        "xlsx" if opaque && has_zip_signature(content) => (
            AiConversationSourceKind::Spreadsheet,
            XLSX_MEDIA_TYPE,
            &[XLSX_MEDIA_TYPE, "application/zip"],
        ),
        "csv" if opaque => (
            AiConversationSourceKind::DelimitedText,
            "text/csv",
            &["text/csv", "application/csv", "text/plain"],
        ),
        "tsv" if opaque => (
            AiConversationSourceKind::DelimitedText,
            "text/tab-separated-values",
            &["text/tab-separated-values", "text/plain"],
        ),
        "txt" if opaque => (
            AiConversationSourceKind::Text,
            "text/plain",
            &["text/plain"],
        ),
        "md" if opaque => (
            AiConversationSourceKind::Text,
            "text/markdown",
            &["text/markdown", "text/plain"],
        ),
        "json" if opaque => (
            AiConversationSourceKind::Text,
            "application/json",
            &["application/json", "text/json", "text/plain"],
        ),
        "pdf" if inspection.kind == AttachmentContentKind::Pdf => (
            AiConversationSourceKind::Pdf,
            "application/pdf",
            &["application/pdf"],
        ),
        "png" if inspection.kind == AttachmentContentKind::Png => {
            (AiConversationSourceKind::Image, "image/png", &["image/png"])
        }
        "jpg" | "jpeg" if inspection.kind == AttachmentContentKind::Jpeg => (
            AiConversationSourceKind::Image,
            "image/jpeg",
            &["image/jpeg"],
        ),
        "tif" | "tiff" if inspection.kind == AttachmentContentKind::Tiff => (
            AiConversationSourceKind::Image,
            "image/tiff",
            &["image/tiff"],
        ),
        _ => return Err(AiSourcePolicyError::Unsupported),
    };
    let declared = declared_media_type
        .map(|value| value.split(';').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty());
    if declared.is_some_and(|value| value != OCTET_STREAM && !allowed_media_types.contains(&value))
    {
        return Err(AiSourcePolicyError::MediaTypeMismatch);
    }
    Ok((
        kind,
        inspection
            .media_type
            .filter(|value| value != OCTET_STREAM)
            .unwrap_or_else(|| canonical_media_type.to_owned()),
    ))
}

fn has_zip_signature(content: &[u8]) -> bool {
    matches!(
        content.get(..4),
        Some([b'P', b'K', 3, 4] | [b'P', b'K', 5, 6] | [b'P', b'K', 7, 8])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspection(kind: AttachmentContentKind) -> AttachmentInspection {
        AttachmentInspection {
            kind,
            media_type: kind.media_type().map(str::to_owned),
            preview_supported: kind.preview_supported(),
        }
    }

    fn classify(
        file_name: &str,
        declared: Option<&str>,
        kind: AttachmentContentKind,
        content: &[u8],
    ) -> Result<(AiConversationSourceKind, String), AiSourcePolicyError> {
        classify_ai_source(file_name, declared, inspection(kind), content)
    }

    #[test]
    fn whitelisted_text_formats_use_canonical_media_types() {
        let opaque = AttachmentContentKind::Opaque;
        for (name, kind, media_type) in [
            ("a.csv", AiConversationSourceKind::DelimitedText, "text/csv"),
            (
                "a.TSV",
                AiConversationSourceKind::DelimitedText,
                "text/tab-separated-values",
            ),
            ("a.txt", AiConversationSourceKind::Text, "text/plain"),
            ("a.md", AiConversationSourceKind::Text, "text/markdown"),
            ("a.json", AiConversationSourceKind::Text, "application/json"),
        ] {
            assert_eq!(
                classify(name, None, opaque, b"x"),
                Ok((kind, media_type.to_owned())),
                "{name}"
            );
        }
    }

    #[test]
    fn binary_formats_require_a_matching_content_signature() {
        assert_eq!(
            classify("scan.pdf", None, AttachmentContentKind::Pdf, b"%PDF"),
            Ok((AiConversationSourceKind::Pdf, "application/pdf".to_owned()))
        );
        assert_eq!(
            classify("gel.JPEG", None, AttachmentContentKind::Jpeg, b""),
            Ok((AiConversationSourceKind::Image, "image/jpeg".to_owned()))
        );
        assert_eq!(
            classify("gel.tif", None, AttachmentContentKind::Tiff, b""),
            Ok((AiConversationSourceKind::Image, "image/tiff".to_owned()))
        );
        // A PDF extension over non-PDF content, or an image renamed to text, is refused.
        for (name, kind) in [
            ("scan.pdf", AttachmentContentKind::Opaque),
            ("notes.txt", AttachmentContentKind::Png),
            ("image.png", AttachmentContentKind::Jpeg),
        ] {
            assert_eq!(
                classify(name, None, kind, b""),
                Err(AiSourcePolicyError::Unsupported),
                "{name}"
            );
        }
    }

    #[test]
    fn xlsx_requires_a_zip_container_signature() {
        let opaque = AttachmentContentKind::Opaque;
        assert_eq!(
            classify("sheet.xlsx", None, opaque, b"PK\x03\x04rest"),
            Ok((
                AiConversationSourceKind::Spreadsheet,
                XLSX_MEDIA_TYPE.to_owned()
            ))
        );
        assert_eq!(
            classify("sheet.xlsx", None, opaque, b"not a zip"),
            Err(AiSourcePolicyError::Unsupported)
        );
        assert_eq!(
            classify("sheet.xlsx", None, opaque, b"PK"),
            Err(AiSourcePolicyError::Unsupported)
        );
    }

    #[test]
    fn unlisted_or_missing_extensions_are_unsupported() {
        for name in [
            "run.exe",
            "macro.xlsm",
            "image.webp",
            "README",
            "archive.zip",
        ] {
            assert_eq!(
                classify(name, None, AttachmentContentKind::Opaque, b"PK\x03\x04"),
                Err(AiSourcePolicyError::Unsupported),
                "{name}"
            );
        }
    }

    #[test]
    fn declared_media_type_must_agree_with_the_extension() {
        let opaque = AttachmentContentKind::Opaque;
        assert!(classify("a.csv", Some("text/csv; charset=utf-8"), opaque, b"").is_ok());
        assert!(classify("a.csv", Some("application/octet-stream"), opaque, b"").is_ok());
        assert!(classify("a.csv", Some("  "), opaque, b"").is_ok());
        assert_eq!(
            classify("a.csv", Some("application/pdf"), opaque, b""),
            Err(AiSourcePolicyError::MediaTypeMismatch)
        );
    }

    #[test]
    fn file_names_must_be_plain_names() {
        assert_eq!(
            normalize_ai_source_file_name("  notes.csv ".to_owned()),
            Ok("notes.csv".to_owned())
        );
        for name in [
            "",
            "   ",
            ".",
            "..",
            "../notes.csv",
            "dir\\notes.csv",
            "nul\0.csv",
            "line\nbreak.csv",
        ] {
            assert_eq!(
                normalize_ai_source_file_name(name.to_owned()),
                Err(AiSourcePolicyError::InvalidFileName),
                "{name:?}"
            );
        }
        let longest = "a".repeat(MAX_AI_SOURCE_FILE_NAME_BYTES);
        assert!(normalize_ai_source_file_name(longest.clone()).is_ok());
        assert_eq!(
            normalize_ai_source_file_name(format!("{longest}a")),
            Err(AiSourcePolicyError::InvalidFileName)
        );
    }

    #[test]
    fn media_types_are_short_ascii_without_controls() {
        assert_eq!(normalize_ai_source_media_type(None), Ok(None));
        assert_eq!(
            normalize_ai_source_media_type(Some("  ".to_owned())),
            Ok(None)
        );
        assert_eq!(
            normalize_ai_source_media_type(Some(" text/csv ".to_owned())),
            Ok(Some("text/csv".to_owned()))
        );
        for value in [
            "text/\u{7f}csv".to_owned(),
            "text/çsv".to_owned(),
            "a".repeat(MAX_AI_SOURCE_MEDIA_TYPE_BYTES + 1),
        ] {
            assert_eq!(
                normalize_ai_source_media_type(Some(value)),
                Err(AiSourcePolicyError::InvalidMediaType)
            );
        }
    }
}
