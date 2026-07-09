//! Shared test doubles for the crate's `#[cfg(test)]` modules.

use bytes::Buf;

use crate::error::DecodeError;
use crate::message_field::DefaultInstance;

/// Test double whose `compute_size` reports a caller-chosen value and whose
/// `write_to` writes nothing — lets over-limit encode paths be exercised
/// without materializing gigabytes. (buffa-types carries its own copy in
/// `any_ext.rs`; `#[cfg(test)]` items don't cross the crate boundary.)
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SizedMsg {
    pub(crate) reported_size: u32,
}

impl DefaultInstance for SizedMsg {
    fn default_instance() -> &'static Self {
        static INST: crate::__private::OnceBox<SizedMsg> = crate::__private::OnceBox::new();
        INST.get_or_init(|| alloc::boxed::Box::new(SizedMsg::default()))
    }
}

impl crate::Message for SizedMsg {
    fn compute_size(&self, _cache: &mut crate::SizeCache) -> u32 {
        self.reported_size
    }
    fn write_to(&self, _cache: &mut crate::SizeCache, _buf: &mut impl crate::EncodeSink) {}
    fn merge_field(
        &mut self,
        tag: crate::encoding::Tag,
        buf: &mut impl Buf,
        _ctx: crate::DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        crate::encoding::skip_field(tag, buf)?;
        Ok(())
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}
