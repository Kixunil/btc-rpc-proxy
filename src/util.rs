use hyper::body::Bytes;
use serde::{
    de::{Deserialize, Deserializer, Unexpected},
    ser::{Serialize, Serializer},
};

#[derive(Debug)]
pub struct HexBytes(Bytes);

impl std::ops::Deref for HexBytes {
    type Target = Bytes;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}


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

pub trait Apply: Sized {
    fn apply<F: FnOnce(Self) -> T, T>(self, f: F) -> T {
        f(self)
    }
}
impl<T: Sized> Apply for T {}
pub trait ApplyRef {
    fn apply_ref<F: FnOnce(&Self) -> T, T>(&self, f: F) -> T {
        f(self)
    }
}
impl<T> ApplyRef for T {}
pub trait ApplyMut {
    fn apply_mut<F: FnOnce(&mut Self) -> T, T>(&mut self, f: F) -> T {
        f(self)
    }
}
impl<T> ApplyMut for T {}

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
