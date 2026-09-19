use std::fmt;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone)]
pub struct AppError {
    pub message: String,
    pub code: i32,
}

impl AppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }

    pub fn with_code(message: impl Into<String>, code: i32) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}
