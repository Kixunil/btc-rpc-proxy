use std::fmt;

#[derive(Copy, Clone)]
pub enum InvalidType {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

pub enum BadRequest {
    MissingMethod,
    InvalidType(InvalidType),
    Json(serde_json::Error),
}

impl fmt::Display for BadRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use InvalidType::*;

        match self {
            BadRequest::MissingMethod => write!(f, "missing field: method"),
            BadRequest::InvalidType(Null) => write!(f, "invalid type: Null"),
            BadRequest::InvalidType(Bool) => write!(f, "invalid type: Bool"),
            BadRequest::InvalidType(Number) => write!(f, "invalid type: Number"),
            BadRequest::InvalidType(String) => write!(f, "invalid type: String"),
            BadRequest::InvalidType(Array) => write!(f, "invalid type: Array"),
            BadRequest::InvalidType(Object) => write!(f, "invalid type: Object"),
            BadRequest::Json(error) => write!(f, "failed to parse Json: {}", error),
        }
    }
}
