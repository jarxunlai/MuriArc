use std::path::Path;

use thiserror::Error;
use tokio::fs;

const MAX_PREVIEW_PAGES: usize = 2_000;
const MAX_IMAGE_PIXELS: u64 = 500_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentContentKind {
    Jpeg,
    Png,
    Webp,
    Gif,
    Bmp,
    Tiff,
    Pdf,
    Heic,
    Opaque,
}

impl AttachmentContentKind {
    pub const fn media_type(self) -> Option<&'static str> {
        match self {
            Self::Jpeg => Some("image/jpeg"),
            Self::Png => Some("image/png"),
            Self::Webp => Some("image/webp"),
            Self::Gif => Some("image/gif"),
            Self::Bmp => Some("image/bmp"),
            Self::Tiff => Some("image/tiff"),
            Self::Pdf => Some("application/pdf"),
            Self::Heic => Some("image/heic"),
            Self::Opaque => None,
        }
    }

    pub const fn preview_supported(self) -> bool {
        !matches!(self, Self::Opaque)
    }

    fn allowed_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Png => &["png"],
            Self::Webp => &["webp"],
            Self::Gif => &["gif"],
            Self::Bmp => &["bmp"],
            Self::Tiff => &["tif", "tiff"],
            Self::Pdf => &["pdf"],
            Self::Heic => &["heic", "heif"],
            Self::Opaque => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentInspection {
    pub kind: AttachmentContentKind,
    pub media_type: Option<String>,
    pub preview_supported: bool,
}

#[derive(Debug, Error)]
pub enum AttachmentInspectionError {
    #[error("executable or script content is not accepted as an attachment")]
    ExecutableContent,
    #[error("the file extension, declared media type and content signature do not agree")]
    SignatureMismatch,
    #[error("the image or document exceeds safe preview resource limits")]
    ResourceLimit,
    #[error("attachment inspection failed")]
    Io,
}

pub async fn inspect_attachment(
    path: &Path,
    file_name: &str,
    declared_media_type: Option<&str>,
) -> Result<AttachmentInspection, AttachmentInspectionError> {
    let bytes = fs::read(path)
        .await
        .map_err(|_| AttachmentInspectionError::Io)?;
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref().is_some_and(is_dangerous_extension) || executable_magic(&bytes) {
        return Err(AttachmentInspectionError::ExecutableContent);
    }

    let kind = detect_kind(&bytes);
    if kind != AttachmentContentKind::Opaque {
        if !extension
            .as_deref()
            .is_some_and(|value| kind.allowed_extensions().contains(&value))
        {
            return Err(AttachmentInspectionError::SignatureMismatch);
        }
        if let Some(declared) = declared_media_type
            && declared != "application/octet-stream"
            && !media_type_matches(kind, declared)
        {
            return Err(AttachmentInspectionError::SignatureMismatch);
        }
        validate_resources(kind, &bytes)?;
    } else if extension.as_deref().is_some_and(is_preview_extension)
        || declared_media_type.is_some_and(is_preview_media_type)
    {
        return Err(AttachmentInspectionError::SignatureMismatch);
    }

    let media_type = kind
        .media_type()
        .map(str::to_owned)
        .or_else(|| declared_media_type.map(str::to_owned));
    Ok(AttachmentInspection {
        kind,
        media_type,
        preview_supported: kind.preview_supported(),
    })
}

fn detect_kind(bytes: &[u8]) -> AttachmentContentKind {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        AttachmentContentKind::Jpeg
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        AttachmentContentKind::Png
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        AttachmentContentKind::Webp
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        AttachmentContentKind::Gif
    } else if bytes.starts_with(b"BM") {
        AttachmentContentKind::Bmp
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        AttachmentContentKind::Tiff
    } else if bytes.starts_with(b"%PDF-") {
        AttachmentContentKind::Pdf
    } else if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(
            &bytes[8..12],
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
        )
    {
        AttachmentContentKind::Heic
    } else {
        AttachmentContentKind::Opaque
    }
}

fn executable_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"#!")
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
}

fn is_dangerous_extension(value: &str) -> bool {
    matches!(
        value,
        "exe"
            | "dll"
            | "com"
            | "bat"
            | "cmd"
            | "ps1"
            | "sh"
            | "elf"
            | "msi"
            | "jar"
            | "apk"
            | "dmg"
            | "app"
            | "scr"
            | "cpl"
            | "sys"
    )
}

fn is_preview_extension(value: &str) -> bool {
    matches!(
        value,
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "pdf" | "heic" | "heif"
    )
}

fn is_preview_media_type(value: &str) -> bool {
    matches!(
        value.split(';').next().unwrap_or(value).trim(),
        "image/jpeg"
            | "image/png"
            | "image/webp"
            | "image/gif"
            | "image/bmp"
            | "image/tiff"
            | "image/heic"
            | "image/heif"
            | "application/pdf"
    )
}

fn media_type_matches(kind: AttachmentContentKind, value: &str) -> bool {
    let value = value.split(';').next().unwrap_or(value).trim();
    match kind {
        AttachmentContentKind::Heic => matches!(value, "image/heic" | "image/heif"),
        _ => kind.media_type() == Some(value),
    }
}

fn validate_resources(
    kind: AttachmentContentKind,
    bytes: &[u8],
) -> Result<(), AttachmentInspectionError> {
    if kind == AttachmentContentKind::Pdf {
        let pages = bytes
            .windows(b"/Type /Page".len())
            .enumerate()
            .filter(|(index, window)| {
                *window == b"/Type /Page"
                    && bytes
                        .get(*index + b"/Type /Page".len())
                        .is_none_or(|next| *next != b's')
            })
            .count();
        if pages > MAX_PREVIEW_PAGES {
            return Err(AttachmentInspectionError::ResourceLimit);
        }
        return Ok(());
    }

    if let Some((width, height)) = image_dimensions(kind, bytes) {
        if width == 0
            || height == 0
            || u64::from(width)
                .checked_mul(u64::from(height))
                .is_none_or(|pixels| pixels > MAX_IMAGE_PIXELS)
        {
            return Err(AttachmentInspectionError::ResourceLimit);
        }
    }
    if kind == AttachmentContentKind::Tiff {
        validate_tiff_ifd_count(bytes)?;
    }
    Ok(())
}

fn image_dimensions(kind: AttachmentContentKind, bytes: &[u8]) -> Option<(u32, u32)> {
    match kind {
        AttachmentContentKind::Png if bytes.len() >= 24 => Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        )),
        AttachmentContentKind::Gif if bytes.len() >= 10 => Some((
            u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32,
            u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32,
        )),
        AttachmentContentKind::Bmp if bytes.len() >= 26 => Some((
            u32::from_le_bytes(bytes[18..22].try_into().ok()?),
            i32::from_le_bytes(bytes[22..26].try_into().ok()?).unsigned_abs(),
        )),
        AttachmentContentKind::Webp if bytes.len() >= 30 && &bytes[12..16] == b"VP8X" => Some((
            1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]),
            1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]),
        )),
        AttachmentContentKind::Jpeg => jpeg_dimensions(bytes),
        AttachmentContentKind::Tiff => tiff_dimensions(bytes),
        _ => None,
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
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
            return Some((
                u16::from_be_bytes([bytes[offset + 7], bytes[offset + 8]]) as u32,
                u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32,
            ));
        }
        if marker == 0xd8 || marker == 0xd9 {
            offset += 2;
            continue;
        }
        let length =
            u16::from_be_bytes([*bytes.get(offset + 2)?, *bytes.get(offset + 3)?]) as usize;
        if length < 2 {
            return None;
        }
        offset = offset.checked_add(length + 2)?;
    }
    None
}

fn tiff_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let little = bytes.starts_with(b"II");
    let ifd = read_u32(bytes, 4, little)? as usize;
    let count = read_u16(bytes, ifd, little)? as usize;
    let mut width = None;
    let mut height = None;
    for index in 0..count.min(4_096) {
        let entry = ifd.checked_add(2 + index * 12)?;
        let tag = read_u16(bytes, entry, little)?;
        let field_type = read_u16(bytes, entry + 2, little)?;
        let value = if field_type == 3 {
            read_u16(bytes, entry + 8, little)? as u32
        } else {
            read_u32(bytes, entry + 8, little)?
        };
        match tag {
            256 => width = Some(value),
            257 => height = Some(value),
            _ => {}
        }
    }
    Some((width?, height?))
}

fn validate_tiff_ifd_count(bytes: &[u8]) -> Result<(), AttachmentInspectionError> {
    let little = bytes.starts_with(b"II");
    let mut offset =
        read_u32(bytes, 4, little).ok_or(AttachmentInspectionError::ResourceLimit)? as usize;
    let mut pages = 0;
    while offset != 0 {
        pages += 1;
        if pages > MAX_PREVIEW_PAGES {
            return Err(AttachmentInspectionError::ResourceLimit);
        }
        let count = read_u16(bytes, offset, little)
            .ok_or(AttachmentInspectionError::ResourceLimit)? as usize;
        let next = offset
            .checked_add(2)
            .and_then(|value| value.checked_add(count.checked_mul(12)?))
            .ok_or(AttachmentInspectionError::ResourceLimit)?;
        offset =
            read_u32(bytes, next, little).ok_or(AttachmentInspectionError::ResourceLimit)? as usize;
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize, little: bool) -> Option<u16> {
    let raw: [u8; 2] = bytes.get(offset..offset + 2)?.try_into().ok()?;
    Some(if little {
        u16::from_le_bytes(raw)
    } else {
        u16::from_be_bytes(raw)
    })
}

fn read_u32(bytes: &[u8], offset: usize, little: bool) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
    Some(if little {
        u32::from_le_bytes(raw)
    } else {
        u32::from_be_bytes(raw)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_executable_magic_and_disguised_preview_files() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("sample.jpg");
        fs::write(&executable, b"MZ executable").await.unwrap();
        assert!(matches!(
            inspect_attachment(&executable, "sample.jpg", Some("image/jpeg")).await,
            Err(AttachmentInspectionError::ExecutableContent)
        ));
        let fake = root.path().join("fake.pdf");
        fs::write(&fake, b"not a pdf").await.unwrap();
        assert!(matches!(
            inspect_attachment(&fake, "fake.pdf", Some("application/pdf")).await,
            Err(AttachmentInspectionError::SignatureMismatch)
        ));
    }

    #[tokio::test]
    async fn accepts_real_png_and_opaque_scientific_data() {
        let root = tempfile::tempdir().unwrap();
        let png = root.path().join("image.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&10_u32.to_be_bytes());
        bytes.extend_from_slice(&20_u32.to_be_bytes());
        fs::write(&png, bytes).await.unwrap();
        let inspected = inspect_attachment(&png, "image.png", Some("image/png"))
            .await
            .unwrap();
        assert_eq!(inspected.kind, AttachmentContentKind::Png);
        assert!(inspected.preview_supported);

        let data = root.path().join("counts.h5ad");
        fs::write(&data, b"opaque scientific data").await.unwrap();
        let inspected = inspect_attachment(&data, "counts.h5ad", Some("application/x-hdf5"))
            .await
            .unwrap();
        assert_eq!(inspected.kind, AttachmentContentKind::Opaque);
        assert!(!inspected.preview_supported);
    }
}
