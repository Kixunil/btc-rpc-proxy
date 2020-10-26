use hyper::body::Bytes;
use serde::{
    de::{Deserialize, Deserializer, Unexpected},
    ser::{Serialize, Serializer},
};

#[derive(Debug, Deref, DerefMut, From, Into)]
pub struct HexBytes(Bytes);

impl Serialize for HexBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        hex::encode(self.0.as_ref()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        hex::decode(s).map(|b| HexBytes(b.into())).map_err(|_| {
            serde::de::Error::invalid_value(Unexpected::Str(s), &"a hexadecimal string")
        })
    }
}

pub fn deserialize_parse<'de, D: Deserializer<'de>, T: std::str::FromStr>(
    deserializer: D,
) -> Result<T, D::Error> {
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse()
        .map_err(|_| serde::de::Error::invalid_value(Unexpected::Str(&s), &"a valid URI"))
}

#[cfg(feature = "compat")]
pub mod compat {
    pub trait StrCompat {
        fn strip_prefix<'a>(&'a self, prefix: &str) -> Option<&'a str>;
        fn strip_suffix<'a>(&'a self, prefix: &str) -> Option<&'a str>;
    }
    impl StrCompat for str {
        fn strip_prefix<'a>(&'a self, prefix: &str) -> Option<&'a str> {
            if let Some(s) = self.matches(prefix).next() {
                Some(&self[s.len()..])
            } else {
                None
            }
        }
        fn strip_suffix<'a>(&'a self, suffix: &str) -> Option<&'a str> {
            if let Some(s) = self.rmatches(prefix).next() {
                Some(&self[..(self.len() - s.len())])
            } else {
                None
            }
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Either<Left, Right> {
    Left(Left),
    Right(Right),
}
impl<Left, Right> Either<Left, Right> {
    pub fn as_left(&self) -> Option<&Left> {
        match self {
            Either::Left(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_right(&self) -> Option<&Right> {
        match self {
            Either::Right(a) => Some(a),
            _ => None,
        }
    }

    pub fn into_left(self) -> Option<Left> {
        match self {
            Either::Left(a) => Some(a),
            _ => None,
        }
    }

    pub fn into_right(self) -> Option<Right> {
        match self {
            Either::Right(a) => Some(a),
            _ => None,
        }
    }
}
