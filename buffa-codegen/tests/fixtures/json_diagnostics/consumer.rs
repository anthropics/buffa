#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text(pub String);
impl From<String> for Text {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl core::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl buffa::ProtoString for Text {
    fn from_wire(p: buffa::WirePayload<'_>) -> Result<Self, buffa::DecodeError> {
        p.to_str().map(|s| Self(s.to_owned()))
    }
}
#[derive(Debug, thiserror::Error)]
enum TextErrorReason {
    #[error("missing text encoding prefix")]
    MissingPrefix,
}
impl serde::Serialize for Text {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("prefix:{}", self.0))
    }
}
impl<'de> serde::Deserialize<'de> for Text {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(d)?;
        text.strip_prefix("prefix:")
            .map(|s| Self(s.to_owned()))
            .ok_or_else(|| serde::de::Error::custom(TextErrorReason::MissingPrefix))
    }
}
pub mod compat {
    buffa::include_proto!("compat");
}

// These collections intentionally have native Serde envelopes. Element-level
// JSON hardening must not replace a configured collection's own conversion.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EnvelopeList<T> {
    pub values: Vec<T>,
}
impl<T> Default for EnvelopeList<T> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}
impl<T> From<Vec<T>> for EnvelopeList<T> {
    fn from(values: Vec<T>) -> Self {
        Self { values }
    }
}
impl<T> FromIterator<T> for EnvelopeList<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}
impl<T> core::ops::Deref for EnvelopeList<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.values
    }
}
impl<T: Clone + PartialEq + core::fmt::Debug + Send + Sync> buffa::ProtoList<T>
    for EnvelopeList<T>
{
    fn push(&mut self, value: T) {
        self.values.push(value);
    }
    fn clear(&mut self) {
        self.values.clear();
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "K: Ord + serde::Deserialize<'de>, V: serde::Deserialize<'de>"))]
pub struct EnvelopeMap<K, V> {
    pub entries: std::collections::BTreeMap<K, V>,
}
impl<K, V> Default for EnvelopeMap<K, V> {
    fn default() -> Self {
        Self {
            entries: std::collections::BTreeMap::new(),
        }
    }
}
impl<K: Ord, V> FromIterator<(K, V)> for EnvelopeMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            entries: iter.into_iter().collect(),
        }
    }
}
impl<K: Ord, V> buffa::MapStorage for EnvelopeMap<K, V> {
    type Key = K;
    type Value = V;
    fn storage_len(&self) -> usize {
        self.entries.len()
    }
    fn storage_insert(&mut self, key: K, value: V) {
        self.entries.insert(key, value);
    }
    fn storage_clear(&mut self) {
        self.entries.clear();
    }
    fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: 'a,
        V: 'a,
    {
        self.entries.iter()
    }
}
