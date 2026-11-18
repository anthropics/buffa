//! String and bytes representations that record the window their
//! `decode_length_delimited` override is given.

use std::cell::RefCell;

use buffa::bytes::Buf;
use buffa::{DecodeError, FieldPayload, WirePayload};

thread_local! {
    static WINDOWS: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

/// Take the windows the overrides were given on this thread since the last
/// call, in decode order.
///
/// The record is per thread, so tests running in parallel do not disturb each
/// other.
pub fn take_windows() -> Vec<Vec<u8>> {
    WINDOWS.with(|windows| core::mem::take(&mut *windows.borrow_mut()))
}

/// Copy the whole window, recording it.
fn record(payload: &mut FieldPayload<impl Buf>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.remaining());
    while payload.has_remaining() {
        let chunk = payload.chunk();
        out.extend_from_slice(chunk);
        let read = chunk.len();
        payload.advance(read);
    }
    WINDOWS.with(|windows| windows.borrow_mut().push(out.clone()));
    out
}

/// A `ProtoString` whose binary decode records its window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RecStr(pub String);

impl From<String> for RecStr {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for RecStr {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for RecStr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl buffa::ProtoString for RecStr {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(|s| Self(s.to_owned()))
    }

    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        String::from_utf8(record(payload))
            .map(Self)
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}

/// A `ProtoBytes` whose binary decode records its window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecBytes(pub Vec<u8>);

impl From<Vec<u8>> for RecBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for RecBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for RecBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl buffa::ProtoBytes for RecBytes {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(Self(payload.as_slice().to_vec()))
    }

    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        Ok(Self(record(payload)))
    }
}

buffa::include_proto!("decode_hook_frame");
