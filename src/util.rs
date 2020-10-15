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
