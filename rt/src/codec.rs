//! Encoding of values (`spec/IDL.md` section 3, "Encoding of values").

use core::cell::Cell;
use core::convert::Infallible;
use core::fmt;

use crate::{CapList, Capability, MAX_CAPS, MAX_DEPTH, MAX_MESSAGE};

/// Why bytes and capabilities did not decode. Every variant means the input
/// is malformed; the variants only say where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ends inside a value, or a length or count exceeds the bytes
    /// that remain.
    Truncated,
    /// A header field is invalid (zero ordinal, reserved flag, bad status,
    /// payload length not matching the message length).
    BadHeader,
    /// The message is not the one expected (wrong ordinal, request where a
    /// reply was expected or the reverse).
    Unexpected,
    /// The message is longer than [`MAX_MESSAGE`] or carries more than
    /// [`crate::MAX_CAPS`] capabilities.
    TooLarge,
    /// A `bool` byte other than 0 or 1.
    BadBool,
    /// An `option` tag other than 0 or 1.
    BadOption,
    /// A `string` that is not UTF-8.
    BadUtf8,
    /// A capability index out of encoding order or past the list.
    BadCapIndex,
    /// A capability whose reported object type is not the one the field
    /// declares (`cap<T>`), or not an `Endpoint` (`endpoint<P>`).
    CapType,
    /// A transferred capability that no field referenced.
    UnusedCap,
    /// Field lists nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A field list ended before a field that has no default.
    MissingField,
    /// A request carries fields after those of every list this provider's
    /// version knows: a newer client asked for something this version
    /// cannot honour (encoders leave out trailing fields that hold their
    /// default, so only a field that says something arrives). Answered
    /// with status `unknown_field`; never an error of a reply, whose
    /// unknown fields are skipped.
    UnknownField,
    /// Bytes after the payload's field list.
    TrailingBytes,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DecodeError::Truncated => "input ends inside a value",
            DecodeError::BadHeader => "invalid header",
            DecodeError::Unexpected => "unexpected message",
            DecodeError::TooLarge => "message exceeds the limits",
            DecodeError::BadBool => "invalid bool",
            DecodeError::BadOption => "invalid option tag",
            DecodeError::BadUtf8 => "string is not UTF-8",
            DecodeError::BadCapIndex => "capability index out of order",
            DecodeError::CapType => "capability of the wrong object type",
            DecodeError::UnusedCap => "capability not referenced",
            DecodeError::TooDeep => "field lists nested too deep",
            DecodeError::MissingField => "missing field without default",
            DecodeError::UnknownField => "field unknown to this version",
            DecodeError::TrailingBytes => "bytes after the payload",
        };
        f.write_str(s)
    }
}

/// Why a value did not encode. The transport reports each as `too_large`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// The message would exceed [`MAX_MESSAGE`] or the buffer.
    TooLarge,
    /// The message would carry more than [`crate::MAX_CAPS`] capabilities.
    TooManyCaps,
    /// Field lists nested deeper than [`MAX_DEPTH`].
    TooDeep,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EncodeError::TooLarge => "message too large",
            EncodeError::TooManyCaps => "too many capabilities",
            EncodeError::TooDeep => "field lists nested too deep",
        })
    }
}

/// Writes values into a byte buffer. A capability writes only its index
/// here; [`Encode::move_caps`] moves it into the [`CapList`] afterwards, in
/// the same order.
pub struct Encoder<'b, C> {
    buf: &'b mut [u8],
    pos: usize,
    caps: usize,
    depth: u32,
    _caps: core::marker::PhantomData<fn(C)>,
}

impl<'b, C> Encoder<'b, C> {
    /// An encoder writing from the start of `buf` (at most
    /// [`MAX_MESSAGE`] bytes of it), with capability indices from 0.
    pub fn new(buf: &'b mut [u8]) -> Self {
        let n = buf.len().min(MAX_MESSAGE);
        let (buf, _) = buf.split_at_mut(n);
        Self {
            buf,
            pos: 0,
            caps: 0,
            depth: 0,
            _caps: core::marker::PhantomData,
        }
    }

    /// Capability indices written so far.
    pub fn caps(&self) -> usize {
        self.caps
    }

    /// Bytes written so far.
    pub fn len(&self) -> usize {
        self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }

    /// Appends raw bytes.
    pub fn put(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self
            .pos
            .checked_add(bytes.len())
            .filter(|&e| e <= self.buf.len())
            .ok_or(EncodeError::TooLarge)?;
        self.buf[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    /// Appends a `u32` length or count.
    pub fn put_len(&mut self, n: usize) -> Result<(), EncodeError> {
        let n = u32::try_from(n).map_err(|_| EncodeError::TooLarge)?;
        self.put(&n.to_le_bytes())
    }

    /// Starts a field list; returns the token for [`Encoder::end_list`].
    pub fn begin_list(&mut self) -> Result<usize, EncodeError> {
        if self.depth >= MAX_DEPTH {
            return Err(EncodeError::TooDeep);
        }
        let at = self.pos;
        self.put(&[0; 4])?;
        self.depth += 1;
        Ok(at)
    }

    /// Forget what was written after byte `to` of the current field list:
    /// fields at its end that hold their defaults, which a decoder gives
    /// back from their absence (`spec/IDL.md` section 3). Never anything
    /// that carries a capability (no such field has a default).
    pub fn truncate(&mut self, to: usize) {
        debug_assert!(to <= self.pos);
        self.pos = to.min(self.pos);
    }

    /// Ends the field list started at `at`, writing its body length.
    pub fn end_list(&mut self, at: usize) -> Result<(), EncodeError> {
        let body = self.pos - at - 4;
        let body = u32::try_from(body).map_err(|_| EncodeError::TooLarge)?;
        self.buf[at..at + 4].copy_from_slice(&body.to_le_bytes());
        self.depth -= 1;
        Ok(())
    }

    /// Appends the index of the next capability. The capability itself
    /// moves into the list later ([`Encode::move_caps`]).
    pub fn cap(&mut self, _cap: &C) -> Result<(), EncodeError> {
        if self.caps >= MAX_CAPS {
            return Err(EncodeError::TooManyCaps);
        }
        let index = u8::try_from(self.caps).map_err(|_| EncodeError::TooManyCaps)?;
        self.put(&[index])?;
        self.caps += 1;
        Ok(())
    }
}

/// Reads values from bytes (borrowed for `'a`) and moves capabilities out of
/// a capability list (borrowed for `'c`). Copying a decoder copies a read
/// position; it never copies data.
pub struct Decoder<'a, 'c, C> {
    buf: &'a [u8],
    pos: usize,
    end: usize,
    caps: &'c [Cell<Option<C>>],
    next_cap: usize,
    depth: u32,
    /// Refuse a field list longer than its fields ([`DecodeError::UnknownField`]):
    /// how a provider decodes a request.
    strict: bool,
}

impl<C> Clone for Decoder<'_, '_, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for Decoder<'_, '_, C> {}

impl<'a, 'c, C> Decoder<'a, 'c, C> {
    /// A decoder over all of `buf`, moving capabilities out of `caps`
    /// starting at index 0. Capabilities no value took stay in `caps`.
    pub fn new(buf: &'a [u8], caps: &'c mut CapList<C>) -> Self {
        Self::over(buf, caps.cells())
    }

    fn over(buf: &'a [u8], caps: &'c [Cell<Option<C>>]) -> Self {
        Self {
            buf,
            pos: 0,
            end: buf.len(),
            caps,
            next_cap: 0,
            depth: 0,
            strict: false,
        }
    }

    /// Bytes left in the current field list (or the input).
    pub fn remaining(&self) -> usize {
        self.end - self.pos
    }

    /// Takes the next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if n > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Takes the next `N` bytes as an array.
    pub fn take_array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    /// Reads a `u32` length or count and checks that at least that many bytes
    /// remain (every encoded value occupies at least one byte).
    pub fn take_len(&mut self) -> Result<usize, DecodeError> {
        let n = u32::from_le_bytes(self.take_array()?) as usize;
        if n > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }

    /// Starts a field list; returns the token for [`Decoder::end_list`].
    pub fn begin_list(&mut self) -> Result<usize, DecodeError> {
        if self.depth >= MAX_DEPTH {
            return Err(DecodeError::TooDeep);
        }
        let body = self.take_len()?;
        let saved = self.end;
        self.end = self.pos + body;
        self.depth += 1;
        Ok(saved)
    }

    /// Reads the next field of the current list. A list that ends exactly
    /// here gives the field's default (an older peer, `spec/IDL.md`
    /// section 4); a field without default is then [`DecodeError::MissingField`].
    pub fn field<T: Decode<'a, C>>(&mut self) -> Result<T, DecodeError> {
        if self.pos == self.end {
            T::missing().ok_or(DecodeError::MissingField)
        } else {
            T::decode(self)
        }
    }

    /// [`Decoder::field`] with an explicit decoding and default: generated
    /// code uses it for fields that carry a capability, to check its type.
    pub fn field_with<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
        missing: Option<T>,
    ) -> Result<T, DecodeError> {
        if self.pos == self.end {
            missing.ok_or(DecodeError::MissingField)
        } else {
            decode(self)
        }
    }

    /// An `option` whose value `decode` reads.
    pub fn option<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        match self.take_array()? {
            [0] => Ok(None),
            [1] => decode(self).map(Some),
            _ => Err(DecodeError::BadOption),
        }
    }

    /// Ends the current field list, skipping fields this version does not
    /// know (a newer peer); in a request ([`Decoder::refuse_unknown`]) they
    /// are [`DecodeError::UnknownField`].
    pub fn end_list(&mut self, saved: usize) -> Result<(), DecodeError> {
        if self.strict && self.pos != self.end {
            return Err(DecodeError::UnknownField);
        }
        self.pos = self.end;
        self.end = saved;
        self.depth -= 1;
        Ok(())
    }

    /// Refuse fields this version does not know from here on: a provider
    /// decoding a request does, so a newer client's field is never ignored
    /// (`spec/IDL.md` section 4).
    pub fn refuse_unknown(&mut self) {
        self.strict = true;
    }

    /// Reads a capability index, which must be the next index in encoding
    /// order, and moves that capability out of the list. With
    /// `object_type` = `Some(t)`, a capability reporting another object
    /// type is [`DecodeError::CapType`] (and stays in the list).
    pub fn cap(&mut self, object_type: Option<&str>) -> Result<C, DecodeError>
    where
        C: Capability,
    {
        let [index] = self.take_array()?;
        if usize::from(index) != self.next_cap {
            return Err(DecodeError::BadCapIndex);
        }
        let slot = self
            .caps
            .get(self.next_cap)
            .ok_or(DecodeError::BadCapIndex)?;
        let cap = slot.take().ok_or(DecodeError::BadCapIndex)?;
        if let (Some(want), Some(got)) = (object_type, cap.object_type())
            && want != got
        {
            slot.set(Some(cap));
            return Err(DecodeError::CapType);
        }
        self.next_cap += 1;
        Ok(cap)
    }

    /// Checks that the input is fully read and every capability referenced.
    pub fn finish(&self) -> Result<(), DecodeError> {
        if self.pos != self.end {
            return Err(DecodeError::TrailingBytes);
        }
        if self.next_cap != self.caps.len() {
            return Err(DecodeError::UnusedCap);
        }
        Ok(())
    }

    /// This decoder without capabilities: a capability index read through
    /// it is [`DecodeError::BadCapIndex`]. Array elements are decoded through
    /// one, because they carry no capabilities
    /// (`spec/IDL_RATIONALE.md` section 6).
    pub(crate) fn bytes_only(&self) -> Decoder<'a, 'a, C> {
        Decoder {
            buf: self.buf,
            pos: self.pos,
            end: self.end,
            caps: &[],
            next_cap: 0,
            depth: self.depth,
            strict: self.strict,
        }
    }

    /// Moves the read position to that of `other`, a [`Decoder::bytes_only`]
    /// copy that read on.
    pub(crate) fn skip_to(&mut self, other: &Decoder<'a, '_, C>) {
        self.pos = other.pos;
    }

    /// Limits this decoder to the bytes read since `start` (a copy taken
    /// earlier), positioned at `start`.
    pub(crate) fn since(start: Self, now: &Self) -> Self {
        Self {
            end: now.pos,
            ..start
        }
    }
}

/// A value that encodes into the wire format, in two steps: [`Encode::encode`]
/// writes the bytes (a capability only its index), then
/// [`Encode::move_caps`] moves the capabilities into the list in the same
/// order.
pub trait Encode<C> {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError>;

    /// Whether this is the value an absent field takes ([`Decode::missing`]):
    /// a field list leaves such fields out at its end. `false` for a type
    /// without a default.
    fn is_default(&self) -> bool {
        false
    }

    /// Moves every capability of `self` into `caps`, in encoding order. The
    /// default is for values that carry none.
    fn move_caps(self, caps: &mut CapList<C>)
    where
        Self: Sized,
    {
        let _ = caps;
    }
}

/// A value that decodes from the wire format, borrowing from the input `'a`.
pub trait Decode<'a, C>: Sized {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError>;

    /// The value an absent appended field takes (`spec/IDL.md` section 4),
    /// or `None` if the type has no default.
    fn missing() -> Option<Self> {
        None
    }
}

/// `T::missing()`, callable with the trait's parameters spelled out.
pub fn missing<'a, C, T: Decode<'a, C>>() -> Option<T> {
    T::missing()
}

macro_rules! int {
    ($($t:ty),*) => {$(
        impl<C> Encode<C> for $t {
            fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
                e.put(&self.to_le_bytes())
            }
            fn is_default(&self) -> bool {
                *self == 0
            }
        }
        impl<'a, C> Decode<'a, C> for $t {
            fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
                Ok(<$t>::from_le_bytes(d.take_array()?))
            }
            fn missing() -> Option<Self> {
                Some(0)
            }
        }
    )*};
}

int!(u8, u16, u32, u64, i8, i16, i32, i64);

impl<C> Encode<C> for bool {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        e.put(&[u8::from(*self)])
    }
    fn is_default(&self) -> bool {
        !*self
    }
}

impl<'a, C> Decode<'a, C> for bool {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        match d.take_array()? {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => Err(DecodeError::BadBool),
        }
    }
    fn missing() -> Option<Self> {
        Some(false)
    }
}

impl<C> Encode<C> for &str {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        e.put_len(self.len())?;
        e.put(self.as_bytes())
    }
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl<'a, C> Decode<'a, C> for &'a str {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        let n = d.take_len()?;
        core::str::from_utf8(d.take(n)?).map_err(|_| DecodeError::BadUtf8)
    }
    fn missing() -> Option<Self> {
        Some("")
    }
}

impl<C> Encode<C> for &[u8] {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        e.put_len(self.len())?;
        e.put(self)
    }
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl<'a, C> Decode<'a, C> for &'a [u8] {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        let n = d.take_len()?;
        d.take(n)
    }
    fn missing() -> Option<Self> {
        Some(&[])
    }
}

impl<C, T: Encode<C>> Encode<C> for Option<T> {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        match self {
            None => e.put(&[0]),
            Some(v) => {
                e.put(&[1])?;
                v.encode(e)
            }
        }
    }

    fn move_caps(self, caps: &mut CapList<C>) {
        if let Some(v) = self {
            v.move_caps(caps);
        }
    }

    fn is_default(&self) -> bool {
        self.is_none()
    }
}

impl<'a, C, T: Decode<'a, C>> Decode<'a, C> for Option<T> {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        d.option(T::decode)
    }
    fn missing() -> Option<Self> {
        Some(None)
    }
}

impl<C: Capability> Encode<C> for C {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        e.cap(self)
    }

    fn move_caps(self, caps: &mut CapList<C>) {
        // The encoder counted at most MAX_CAPS; a full list cannot happen,
        // and the caller checks the count.
        let _ = caps.push(self);
    }
}

/// An unchecked `cap`: any object type.
impl<'a, C: Capability> Decode<'a, C> for C {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        d.cap(None)
    }
}

/// The error type of a method that declares none: it never encodes, and a
/// reply claiming one is malformed.
impl<C> Encode<C> for Infallible {
    fn encode(&self, _: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        match *self {}
    }
}

impl<'a, C> Decode<'a, C> for Infallible {
    fn decode(_: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        Err(DecodeError::Unexpected)
    }
}

/// A field list of one field: the payload of an `error` reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct One<T>(pub T);

impl<C, T: Encode<C>> Encode<C> for One<T> {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        let at = e.begin_list()?;
        self.0.encode(e)?;
        e.end_list(at)
    }

    fn move_caps(self, caps: &mut CapList<C>) {
        self.0.move_caps(caps);
    }
}

impl<'a, C, T: Decode<'a, C>> Decode<'a, C> for One<T> {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        let saved = d.begin_list()?;
        let v = d.field()?;
        d.end_list(saved)?;
        Ok(One(v))
    }
}
