//! Attachment validation and text extraction (`docs/SPEC.md` §9).
//!
//! The file type is detected by magic bytes, the extension is only a hint.
//! Nothing is written to disk and no file content is logged.

use shared::truncate_chars;
use shared::warnings;

use crate::error::{AppError, AppResult};
use crate::limits::{MAX_EXTRACTED_TEXT_CHARS, MAX_FILE_BYTES, PDF_PARSE_TIMEOUT};

const OCR_HINT: &str = "OCR в этой версии не поддерживается";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAttachment {
    pub text: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedFileType {
    Pdf,
    Png,
    Jpeg,
    Text,
    Unknown,
}

pub fn detect_file_type(bytes: &[u8]) -> DetectedFileType {
    if bytes.starts_with(b"%PDF-") {
        return DetectedFileType::Pdf;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return DetectedFileType::Png;
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return DetectedFileType::Jpeg;
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return DetectedFileType::Text;
    }
    if !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok() {
        return DetectedFileType::Text;
    }
    // Bytes that look like a broken text file (non-UTF-8 without control
    // characters) are reported as `invalid_encoding`; arbitrary binary is
    // unsupported.
    if looks_like_text(bytes) {
        return DetectedFileType::Text;
    }
    DetectedFileType::Unknown
}

fn looks_like_text(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && !bytes.contains(&0)
        && !bytes
            .iter()
            .any(|byte| *byte < 0x20 && !matches!(*byte, b'\n' | b'\r' | b'\t'))
}

pub async fn extract_attachment(bytes: Vec<u8>) -> AppResult<ExtractedAttachment> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::code(shared::ErrorCode::FileTooLarge));
    }
    if bytes.is_empty() {
        return Err(AppError::new(
            shared::ErrorCode::TextExtractionFailed,
            "Файл пуст",
        ));
    }

    let file_type = detect_file_type(&bytes);
    let text = match file_type {
        DetectedFileType::Pdf => extract_pdf(bytes).await?,
        DetectedFileType::Png | DetectedFileType::Jpeg => {
            return Err(AppError::new(
                shared::ErrorCode::UnsupportedFileType,
                format!("Изображения не поддерживаются: {OCR_HINT}."),
            ));
        }
        DetectedFileType::Text => extract_text(bytes)?,
        DetectedFileType::Unknown => {
            return Err(AppError::new(
                shared::ErrorCode::UnsupportedFileType,
                "Поддерживаются только файлы TXT и текстовые PDF",
            ));
        }
    };

    if text.trim().is_empty() {
        return Err(AppError::new(
            shared::ErrorCode::TextExtractionFailed,
            "Не удалось извлечь текст из файла",
        ));
    }

    let (text, truncated) = truncate_chars(&text, MAX_EXTRACTED_TEXT_CHARS);
    let warnings = if truncated {
        vec![warnings::DOCUMENT_TRUNCATED.to_owned()]
    } else {
        Vec::new()
    };

    Ok(ExtractedAttachment { text, warnings })
}

fn extract_text(bytes: Vec<u8>) -> AppResult<String> {
    let bytes = bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .map(<[u8]>::to_vec)
        .unwrap_or(bytes);
    String::from_utf8(bytes).map_err(|_| {
        AppError::new(
            shared::ErrorCode::InvalidEncoding,
            "Файл TXT должен быть в кодировке UTF-8. Сохраните его в UTF-8 и попробуйте снова.",
        )
    })
}

async fn extract_pdf(bytes: Vec<u8>) -> AppResult<String> {
    let extraction =
        tokio::task::spawn_blocking(move || pdf_extract::extract_text_from_mem(&bytes));

    let text = match tokio::time::timeout(PDF_PARSE_TIMEOUT, extraction).await {
        Err(_) => {
            return Err(AppError::new(
                shared::ErrorCode::TextExtractionFailed,
                "Превышено время извлечения текста из PDF",
            ));
        }
        Ok(Err(join_error)) => {
            tracing::error!(error = %join_error, "pdf extraction task failed");
            return Err(AppError::code(shared::ErrorCode::Internal));
        }
        Ok(Ok(Err(_))) => {
            return Err(AppError::new(
                shared::ErrorCode::TextExtractionFailed,
                format!("Не удалось прочитать PDF. Похоже, это скан; {OCR_HINT}."),
            ));
        }
        Ok(Ok(Ok(text))) => text,
    };

    if text.trim().is_empty() {
        return Err(AppError::new(
            shared::ErrorCode::TextExtractionFailed,
            format!("В PDF нет текстового слоя. Похоже, это скан; {OCR_HINT}."),
        ));
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_types_by_magic_bytes_not_extension() {
        assert_eq!(detect_file_type(b"%PDF-1.7\n..."), DetectedFileType::Pdf);
        assert_eq!(
            detect_file_type(b"\x89PNG\r\n\x1a\nrest"),
            DetectedFileType::Png
        );
        assert_eq!(
            detect_file_type(b"\xff\xd8\xffrest"),
            DetectedFileType::Jpeg
        );
        assert_eq!(
            detect_file_type("просто текст".as_bytes()),
            DetectedFileType::Text
        );
        assert_eq!(detect_file_type(&[0, 1, 2, 3]), DetectedFileType::Unknown);
    }

    #[tokio::test]
    async fn text_is_extracted_and_bom_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("Привет".as_bytes());
        let extracted = extract_attachment(bytes).await.unwrap();
        assert_eq!(extracted.text, "Привет");
        assert!(extracted.warnings.is_empty());
    }

    #[tokio::test]
    async fn invalid_utf8_is_rejected() {
        let error = extract_attachment(vec![0xD0, 0x28, 0xD0])
            .await
            .unwrap_err();
        assert_eq!(error.code, shared::ErrorCode::InvalidEncoding);
    }

    #[tokio::test]
    async fn png_is_rejected_with_ocr_hint() {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0; 16]);
        let error = extract_attachment(bytes).await.unwrap_err();
        assert_eq!(error.code, shared::ErrorCode::UnsupportedFileType);
        assert!(error.message.contains("OCR"));
    }

    #[tokio::test]
    async fn long_text_is_truncated_with_warning() {
        let text = "a".repeat(MAX_EXTRACTED_TEXT_CHARS + 10);
        let extracted = extract_attachment(text.into_bytes()).await.unwrap();
        assert_eq!(extracted.text.chars().count(), MAX_EXTRACTED_TEXT_CHARS);
        assert_eq!(
            extracted.warnings,
            vec![warnings::DOCUMENT_TRUNCATED.to_owned()]
        );
    }

    #[tokio::test]
    async fn oversized_file_is_rejected() {
        let error = extract_attachment(vec![b'a'; MAX_FILE_BYTES + 1])
            .await
            .unwrap_err();
        assert_eq!(error.code, shared::ErrorCode::FileTooLarge);
    }
}
