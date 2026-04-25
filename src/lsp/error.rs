use std::fmt;
use tower_lsp::jsonrpc;

#[derive(Debug)]
pub enum LspError {
    Analysis(anyhow::Error),
    Io(std::io::Error),
    Parse(String),
    Config(String),
    Internal(String),
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LspError::Analysis(e) => write!(f, "Analysis error: {e}"),
            LspError::Io(e) => write!(f, "IO error: {e}"),
            LspError::Parse(s) => write!(f, "Parse error: {s}"),
            LspError::Config(s) => write!(f, "Config error: {s}"),
            LspError::Internal(s) => write!(f, "Internal error: {s}"),
        }
    }
}

impl std::error::Error for LspError {}

impl From<anyhow::Error> for LspError {
    fn from(e: anyhow::Error) -> Self {
        LspError::Analysis(e)
    }
}

impl From<std::io::Error> for LspError {
    fn from(e: std::io::Error) -> Self {
        LspError::Io(e)
    }
}

impl From<LspError> for jsonrpc::Error {
    fn from(e: LspError) -> Self {
        jsonrpc::Error {
            code: jsonrpc::ErrorCode::InternalError,
            message: e.to_string().into(),
            data: None,
        }
    }
}
