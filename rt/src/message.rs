//! Messages: the 16-byte header and a payload (`spec/IDL.md` section 3).

use crate::client::{CallError, TransportError};
use crate::codec::{Decode, DecodeError, Decoder, Encode, EncodeError, Encoder, One};
use crate::{CapList, FLAG_REPLY, HEADER_LEN, MAX_MESSAGE};

/// Reply status (header offset 6). Requests always carry [`Status::Ok`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Status {
    /// The payload is the result field list.
    Ok = 0,
    /// The payload is the method's error enum value as a one-field list.
    Error = 1,
    /// The provider does not know the method; empty payload.
    UnknownMethod = 2,
    /// The provider could not decode the request; empty payload.
    MalformedRequest = 3,
    /// The provider's reply did not fit the limits; empty payload.
    ReplyTooLarge = 4,
    /// The request carries a field the provider's version does not know
    /// (`spec/IDL.md` section 4): it is not honoured; empty payload.
    UnknownField = 5,
}

impl Status {
    pub fn from_raw(raw: u16) -> Option<Status> {
        match raw {
            0 => Some(Status::Ok),
            1 => Some(Status::Error),
            2 => Some(Status::UnknownMethod),
            3 => Some(Status::MalformedRequest),
            4 => Some(Status::ReplyTooLarge),
            5 => Some(Status::UnknownField),
            _ => None,
        }
    }
}

/// A decoded, validated message header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Method ordinal, never 0.
    pub ordinal: u32,
    /// Flag bit 0.
    pub reply: bool,
    pub status: Status,
    /// Protocol version of the sender: informational, never a reason to reject.
    pub version: u32,
    /// Payload length; the message is exactly `HEADER_LEN + payload_len` bytes.
    pub payload_len: u32,
}

fn le32(b: &[u8], at: usize) -> u32 {
    let mut a = [0; 4];
    a.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(a)
}

fn le16(b: &[u8], at: usize) -> u16 {
    let mut a = [0; 2];
    a.copy_from_slice(&b[at..at + 2]);
    u16::from_le_bytes(a)
}

impl Header {
    /// Validates and decodes the header of the complete message `msg`.
    pub fn decode(msg: &[u8]) -> Result<Header, DecodeError> {
        if msg.len() > MAX_MESSAGE {
            return Err(DecodeError::TooLarge);
        }
        if msg.len() < HEADER_LEN {
            return Err(DecodeError::Truncated);
        }
        let ordinal = le32(msg, 0);
        let flags = le16(msg, 4);
        let status = le16(msg, 6);
        let version = le32(msg, 8);
        let payload_len = le32(msg, 12);
        if flags & !FLAG_REPLY != 0 {
            return Err(DecodeError::BadHeader);
        }
        let reply = flags & FLAG_REPLY != 0;
        let status = Status::from_raw(status).ok_or(DecodeError::BadHeader)?;
        if !reply && status != Status::Ok {
            return Err(DecodeError::BadHeader);
        }
        // Ordinal 0 appears only in the answer to a request that carried none.
        if ordinal == 0 && !(reply && status == Status::MalformedRequest) {
            return Err(DecodeError::BadHeader);
        }
        if payload_len as usize != msg.len() - HEADER_LEN {
            return Err(DecodeError::BadHeader);
        }
        Ok(Header {
            ordinal,
            reply,
            status,
            version,
            payload_len,
        })
    }

    /// Writes the header into the first [`HEADER_LEN`] bytes of `out`.
    pub fn write(&self, out: &mut [u8; HEADER_LEN]) {
        let flags = if self.reply { FLAG_REPLY } else { 0 };
        out[0..4].copy_from_slice(&self.ordinal.to_le_bytes());
        out[4..6].copy_from_slice(&flags.to_le_bytes());
        out[6..8].copy_from_slice(&(self.status as u16).to_le_bytes());
        out[8..12].copy_from_slice(&self.version.to_le_bytes());
        out[12..16].copy_from_slice(&self.payload_len.to_le_bytes());
    }
}

/// Encodes one message into `buf`: the header, then the bytes `payload`
/// writes. `caps` is cleared; the caller moves the capabilities whose
/// indices `payload` wrote into it ([`Encode::move_caps`]). Returns the
/// message length and the number of capability indices written.
fn encode_bytes<C>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    reply: bool,
    status: Status,
    version: u32,
    payload: impl FnOnce(&mut Encoder<'_, C>) -> Result<(), EncodeError>,
) -> Result<(usize, usize), EncodeError> {
    caps.clear();
    let n = buf.len().min(MAX_MESSAGE);
    if n < HEADER_LEN {
        return Err(EncodeError::TooLarge);
    }
    let (head, body) = buf[..n].split_at_mut(HEADER_LEN);
    let mut e = Encoder::new(body);
    payload(&mut e)?;
    let payload_len = u32::try_from(e.len()).map_err(|_| EncodeError::TooLarge)?;
    let mut h = [0; HEADER_LEN];
    Header {
        ordinal,
        reply,
        status,
        version,
        payload_len,
    }
    .write(&mut h);
    head.copy_from_slice(&h);
    Ok((HEADER_LEN + payload_len as usize, e.caps()))
}

/// Encodes one message into `buf`: the header, then the field list
/// `value`, whose capabilities move into `caps` (cleared first). Returns the
/// message length. On failure `value` and its capabilities are dropped, and
/// `caps` is empty.
pub fn encode_message<C, V: Encode<C>>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    reply: bool,
    status: Status,
    version: u32,
    value: V,
) -> Result<usize, EncodeError> {
    let (n, count) = encode_bytes(buf, caps, ordinal, reply, status, version, |e| {
        value.encode(e)
    })?;
    value.move_caps(caps);
    if caps.len() != count {
        // A value whose two encoding steps disagree; never generated code.
        caps.clear();
        return Err(EncodeError::TooManyCaps);
    }
    Ok(n)
}

/// Encodes a request of method `ordinal` with parameters `params`, moving
/// their capabilities into `caps`.
pub fn encode_request<C, Q: Encode<C>>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    version: u32,
    params: Q,
) -> Result<usize, EncodeError> {
    encode_message(buf, caps, ordinal, false, Status::Ok, version, params)
}

/// Encodes an `ok` reply of method `ordinal` with results `results`, moving
/// their capabilities into `caps`.
pub fn encode_reply<C, R: Encode<C>>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    version: u32,
    results: R,
) -> Result<usize, EncodeError> {
    encode_message(buf, caps, ordinal, true, Status::Ok, version, results)
}

/// Encodes an `error` reply of method `ordinal` carrying `error` (an enum,
/// which carries no capabilities).
pub fn encode_error_reply<C, E: Encode<C>>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    version: u32,
    error: &E,
) -> Result<usize, EncodeError> {
    encode_bytes(buf, caps, ordinal, true, Status::Error, version, |e| {
        let at = e.begin_list()?;
        error.encode(e)?;
        e.end_list(at)
    })
    .and_then(|(n, count)| {
        if count == 0 {
            Ok(n)
        } else {
            Err(EncodeError::TooManyCaps)
        }
    })
}

/// Encodes a reply with an empty payload (`unknown_method`,
/// `malformed_request` or `reply_too_large`).
pub fn encode_status_reply<C>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    version: u32,
    status: Status,
) -> Result<usize, EncodeError> {
    encode_bytes(buf, caps, ordinal, true, status, version, |_| Ok(())).map(|(n, _)| n)
}

/// What generated `dispatch` returns for a reply it builds itself: the
/// length of a reply with `status` and an empty payload. `caps` is
/// emptied (its capabilities are released). A header-only reply fits any
/// buffer of at least [`HEADER_LEN`] bytes; a shorter `buf` gets no reply
/// (0), which the caller observes as `malformed_reply`.
pub fn status_reply<C>(
    buf: &mut [u8],
    caps: &mut CapList<C>,
    ordinal: u32,
    version: u32,
    status: Status,
) -> usize {
    encode_status_reply(buf, caps, ordinal, version, status).unwrap_or_else(|_| {
        caps.clear();
        0
    })
}

/// The ordinal of a request that a provider can answer, or `Err(raw)` with
/// the raw ordinal field (0 if absent) for a request whose header is
/// malformed (answer `malformed_request`).
pub fn request_ordinal(msg: &[u8]) -> Result<u32, u32> {
    match Header::decode(msg) {
        Ok(h) if !h.reply => Ok(h.ordinal),
        _ if msg.len() >= 4 => Err(le32(msg, 0)),
        _ => Err(0),
    }
}

/// Decodes the payload of a message whose header is valid: exactly one value
/// `M`, moving every capability out of `caps`. Whatever `M` did not take is
/// dropped with `caps`.
fn decode_payload<'a, C, M: Decode<'a, C>>(
    msg: &'a [u8],
    mut caps: CapList<C>,
    request: bool,
) -> Result<M, DecodeError> {
    let mut d = Decoder::new(&msg[HEADER_LEN..], &mut caps);
    if request {
        d.refuse_unknown();
    }
    let v = M::decode(&mut d)?;
    d.finish()?;
    Ok(v)
}

/// The status a provider answers a request with that did not decode:
/// `unknown_field` for a newer client's field it does not know,
/// `malformed_request` for anything else.
pub fn refusal(e: DecodeError) -> Status {
    match e {
        DecodeError::UnknownField => Status::UnknownField,
        _ => Status::MalformedRequest,
    }
}

/// Decodes a request of method `ordinal`, moving its capabilities out of
/// `caps`. On failure the capabilities are dropped (released).
pub fn decode_request<'a, C, Q: Decode<'a, C>>(
    msg: &'a [u8],
    caps: CapList<C>,
    ordinal: u32,
) -> Result<Q, DecodeError> {
    let h = Header::decode(msg)?;
    if h.reply || h.ordinal != ordinal {
        return Err(DecodeError::Unexpected);
    }
    decode_payload(msg, caps, true)
}

/// Decodes a reply of method `ordinal` as the caller observes it
/// (`spec/IDL.md` section 5): the results, the method's error `E`, or a
/// transport error. A reply that fails to decode is `malformed_reply`.
/// Capabilities move out of `caps`; on failure they are dropped (released).
pub fn decode_reply<'a, C, R: Decode<'a, C>, E: Decode<'a, C>>(
    msg: &'a [u8],
    caps: CapList<C>,
    ordinal: u32,
) -> Result<R, CallError<E>> {
    let malformed = |_| CallError::Transport(TransportError::MalformedReply);
    let h = Header::decode(msg).map_err(malformed)?;
    if !h.reply || h.ordinal != ordinal {
        return Err(malformed(DecodeError::Unexpected));
    }
    let no_caps = caps.is_empty();
    let empty = || {
        if h.payload_len == 0 && no_caps {
            Ok(())
        } else {
            Err(CallError::Transport(TransportError::MalformedReply))
        }
    };
    match h.status {
        Status::Ok => decode_payload(msg, caps, false).map_err(malformed),
        Status::Error => match decode_payload::<C, One<E>>(msg, caps, false) {
            Ok(One(e)) => Err(CallError::Error(e)),
            Err(e) => Err(malformed(e)),
        },
        Status::UnknownMethod => {
            empty()?;
            Err(CallError::Transport(TransportError::UnknownMethod))
        }
        Status::MalformedRequest => {
            empty()?;
            Err(CallError::Transport(TransportError::MalformedRequest))
        }
        Status::ReplyTooLarge => {
            empty()?;
            Err(CallError::Transport(TransportError::ReplyTooLarge))
        }
        Status::UnknownField => {
            empty()?;
            Err(CallError::Transport(TransportError::UnknownField))
        }
    }
}
