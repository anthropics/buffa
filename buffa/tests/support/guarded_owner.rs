//! A real single-bytes-field codec keeps wire ownership tests independent of codegen.
use buffa::bytes::Buf;
use buffa::encoding::{check_wire_type, skip_field_depth, Tag, WireType};
use buffa::{
    DecodeContext, DecodeError, DefaultInstance, EncodeSink, Message, MessageView, SizeCache,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Packet {
    pub data: Vec<u8>,
    /// A dishonest encoder-size report exercises the bounded sink.
    pub reported_len: Option<u32>,
}
impl DefaultInstance for Packet {
    fn default_instance() -> &'static Self {
        static VALUE: std::sync::OnceLock<Packet> = std::sync::OnceLock::new();
        VALUE.get_or_init(Self::default)
    }
}
impl Message for Packet {
    fn compute_size(&self, _: &mut SizeCache) -> u32 {
        self.reported_len.unwrap_or_else(|| {
            if self.data.is_empty() {
                0
            } else {
                u32::try_from(
                    buffa::types::bytes_encoded_len(&self.data)
                        .checked_add(1)
                        .unwrap(),
                )
                .unwrap()
            }
        })
    }
    fn write_to(&self, _: &mut SizeCache, sink: &mut impl EncodeSink) {
        if !self.data.is_empty() {
            Tag::new(1, WireType::LengthDelimited).encode(sink);
            buffa::types::encode_bytes(&self.data, sink);
        }
    }
    fn merge_field(
        &mut self,
        tag: Tag,
        buffer: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        if tag.field_number() == 1 {
            check_wire_type(tag, WireType::LengthDelimited)?;
            self.data = buffa::types::decode_bytes(buffer)?;
            Ok(())
        } else {
            skip_field_depth(tag, buffer, ctx.depth())
        }
    }
    fn clear(&mut self) {
        self.data.clear();
        self.reported_len = None;
    }
}
#[derive(Clone, Debug, Default)]
pub struct PacketView<'a> {
    data: &'a [u8],
}
buffa::impl_view_reborrow!(PacketView);
// SAFETY: the sole borrow is covariant in 'a and every impl is parametric in
// that lifetime. No method retains it beyond Self or exposes a 'static borrow.
buffa::unsafe_impl_view_lifetime_parametric!(PacketView);
impl<'a> MessageView<'a> for PacketView<'a> {
    type Owned = Packet;
    fn decode_view(input: &'a [u8]) -> Result<Self, DecodeError> {
        let unknown = std::cell::Cell::new(buffa::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let elements = std::cell::Cell::new(buffa::DEFAULT_ELEMENT_MEMORY_LIMIT);
        Self::decode_view_ctx(
            input,
            DecodeContext::new(buffa::RECURSION_LIMIT, &unknown).with_element_memory(&elements),
        )
    }
    fn decode_view_with_ctx(input: &'a [u8], ctx: DecodeContext<'_>) -> Result<Self, DecodeError> {
        Self::decode_view_ctx(input, ctx)
    }
    fn merge_view_field(
        &mut self,
        tag: Tag,
        mut input: &'a [u8],
        _: &'a [u8],
        ctx: DecodeContext<'_>,
    ) -> Result<&'a [u8], DecodeError> {
        if tag.field_number() == 1 {
            check_wire_type(tag, WireType::LengthDelimited)?;
            self.data = buffa::types::borrow_bytes(&mut input)?;
        } else {
            skip_field_depth(tag, &mut input, ctx.depth())?;
        }
        Ok(input)
    }
    fn to_owned_message(&self) -> Result<Packet, DecodeError> {
        Ok(Packet {
            data: self.data.to_vec(),
            reported_len: None,
        })
    }
}
