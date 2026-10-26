use super::*;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;

// JSONC lexical handling keeps URLs and comment-like string contents intact.
// Deserialize recursively with duplicate detection, including nested objects.
pub(super) fn parse_json(text: &str) -> Result<Value, Error> {
    let bytes = text.as_bytes();
    let mut clean = bytes.to_vec();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    clean[i] = b' ';
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                clean[i] = b' ';
                clean[i + 1] = b' ';
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                    clean[i] = b' ';
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    return Err(Error::msg("unterminated JSON comment"));
                }
                clean[i] = b' ';
                clean[i + 1] = b' ';
                i += 2;
            }
            _ => i += 1,
        }
    }
    i = 0;
    while i < clean.len() {
        if clean[i] == b'"' {
            i += 1;
            while i < clean.len() {
                if clean[i] == b'\\' {
                    i += 2;
                } else if clean[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else {
            if clean[i] == b',' {
                let mut next = i + 1;
                while next < clean.len() && clean[next].is_ascii_whitespace() {
                    next += 1;
                }
                if matches!(clean.get(next), Some(b'}' | b']')) {
                    clean[i] = b' ';
                }
            }
            i += 1;
        }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(&clean);
    let value = Unique::deserialize(&mut deserializer)
        .map_err(Error::msg)?
        .0;
    deserializer.end().map_err(Error::msg)?;
    Ok(value)
}
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, Unique>()? {
                    if values.insert(key.clone(), value.0).is_some() {
                        return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
                    }
                }
                Ok(Unique(Value::Object(values)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Unique, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(value)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        deserializer.deserialize_any(V)
    }
}

pub(super) enum Native {
    Json(Value),
    Toml(Box<toml_edit::DocumentMut>),
}
impl Native {
    pub fn read(path: &Path) -> Result<Self, Error> {
        reject_symlink(path)?;
        if path.exists() {
            file_record(path)?;
        }
        if path.extension().is_some_and(|s| s == "toml") {
            let text = if path.exists() {
                fs::read_to_string(path)?
            } else {
                String::new()
            };
            Ok(Self::Toml(Box::new(text.parse().map_err(Error::msg)?)))
        } else {
            let value = load_json(path, json!({}))?;
            if !value.is_object() {
                return Err(Error::msg(format!(
                    "expected an object in {}",
                    path.display()
                )));
            }
            Ok(Self::Json(value))
        }
    }
    pub fn servers(&self, key: &str) -> Result<Value, Error> {
        let doc = match self {
            Self::Json(doc) => doc.clone(),
            Self::Toml(doc) => {
                toml_edit::de::from_str::<Value>(&doc.to_string()).map_err(Error::msg)?
            }
        };
        let servers = doc.get(key).cloned().unwrap_or_else(|| json!({}));
        if !servers.is_object() {
            return Err(Error::msg("invalid MCP section: expected a map"));
        }
        Ok(servers)
    }
    /// Replace the MCP section; an empty one is removed.
    pub fn set_servers(&mut self, key: &str, servers: Value) -> Result<(), Error> {
        if servers
            .as_object()
            .is_some_and(|servers| servers.is_empty())
        {
            match self {
                Self::Json(doc) => {
                    doc.as_object_mut().unwrap().remove(key);
                }
                Self::Toml(doc) => {
                    doc.remove(key);
                }
            }
            return Ok(());
        }
        match self {
            Self::Json(doc) => doc[key] = servers,
            Self::Toml(doc) => {
                let replacement =
                    toml_edit::ser::to_document(&json!({key:servers})).map_err(Error::msg)?;
                doc[key] = replacement[key].clone();
            }
        }
        Ok(())
    }
    /// Nothing left to keep, not even comments.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Json(doc) => doc.as_object().is_some_and(|doc| doc.is_empty()),
            Self::Toml(doc) => doc.to_string().trim().is_empty(),
        }
    }
    pub fn bytes(&self) -> Result<Vec<u8>, Error> {
        match self {
            Self::Json(doc) => json_bytes(doc),
            Self::Toml(doc) => Ok(doc.to_string().into_bytes()),
        }
    }
}
