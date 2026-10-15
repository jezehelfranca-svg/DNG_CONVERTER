use std::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum DngError {
    Io(std::io::Error),
    Image(image::ImageError),
    InvalidDng(String),
    DecompressionBomb(String),
    UnsupportedFormat(String),
    ExternalDelegateFailed(String),
    Cancelled,
}

impl fmt::Display for DngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DngError::Io(e) => write!(f, "I/O error: {}", e),
            DngError::Image(e) => write!(f, "Image decode error: {}", e),
            DngError::InvalidDng(s) => write!(f, "Invalid DNG structure: {}", s),
            DngError::DecompressionBomb(s) => write!(f, "Decompression limit exceeded: {}", s),
            DngError::UnsupportedFormat(s) => write!(f, "Unsupported DNG format: {}", s),
            DngError::ExternalDelegateFailed(s) => write!(f, "External delegate failed: {}", s),
            DngError::Cancelled => write!(f, "Operation was cancelled"),
        }
    }
}

impl std::error::Error for DngError {}

impl From<std::io::Error> for DngError {
    fn from(e: std::io::Error) -> Self {
        DngError::Io(e)
    }
}

impl From<image::ImageError> for DngError {
    fn from(e: image::ImageError) -> Self {
        DngError::Image(e)
    }
}