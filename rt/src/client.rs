//! The transport trait and the client half of generated code
//! (`spec/IDL.md` section 5).

use core::fmt;
use core::marker::PhantomData;

use crate::codec::{Decode, Encode};
use crate::message::{decode_reply, encode_request};
use crate::{CapList, Capability, MAX_MESSAGE};

/// What a call can fail with besides the method's own error, uniformly for
/// every protocol (`spec/IDL.md` section 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// The caller's endpoint capability was revoked (also mid-call) or was
    /// never valid.
    Revoked,
    /// The provider ended or restarted while the call was pending.
    PeerGone,
    /// The caller cancelled the pending call; a later reply is discarded.
    Cancelled,
    /// The request exceeds the limits of `spec/IDL.md` section 3; detected
    /// before sending.
    TooLarge,
    /// The reply failed to decode.
    MalformedReply,
    /// The provider does not implement the method.
    UnknownMethod,
    /// The provider rejected the request as malformed.
    MalformedRequest,
    /// The provider's reply exceeded the limits of `spec/IDL.md` section 3;
    /// it sent status `reply_too_large` instead.
    ReplyTooLarge,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TransportError::Revoked => "revoked",
            TransportError::PeerGone => "peer_gone",
            TransportError::Cancelled => "cancelled",
            TransportError::TooLarge => "too_large",
            TransportError::MalformedReply => "malformed_reply",
            TransportError::UnknownMethod => "unknown_method",
            TransportError::MalformedRequest => "malformed_request",
            TransportError::ReplyTooLarge => "reply_too_large",
        })
    }
}

/// The result of a generated client call that did not return the reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError<E> {
    /// The method's declared error. For methods without one, `E` is
    /// [`core::convert::Infallible`].
    Error(E),
    Transport(TransportError),
}

/// Moves one request to a provider and its reply back. On Silica the
/// runtime's `silica_rt::ipc::Channel` implements it over the kernel's
/// call/reply IPC; host tests implement it with a direct call of a
/// generated `dispatch`.
pub trait Transport<C> {
    /// Sends `request` with the capabilities `caps`, waits for the reply,
    /// writes the reply into `reply` and its capabilities into
    /// `reply_caps` (empty on entry), and returns the reply length.
    /// From this call on the transport owns `caps`: it transfers them, or
    /// releases them when the call fails.
    /// Fails only with `Revoked`, `PeerGone` or `Cancelled`; generated code
    /// detects the other [`TransportError`]s itself.
    fn call(
        &mut self,
        request: &[u8],
        caps: CapList<C>,
        reply: &mut [u8],
        reply_caps: &mut CapList<C>,
    ) -> Result<usize, TransportError>;
}

/// A transport with its request and reply buffers. Generated clients wrap
/// one; a reply borrows the client until it is dropped.
pub struct ClientCore<T, C> {
    pub transport: T,
    request: [u8; MAX_MESSAGE],
    reply: [u8; MAX_MESSAGE],
    _caps: PhantomData<fn(C)>,
}

impl<T, C> ClientCore<T, C> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            request: [0; MAX_MESSAGE],
            reply: [0; MAX_MESSAGE],
            _caps: PhantomData,
        }
    }
}

impl<T: Transport<C>, C: Capability> ClientCore<T, C> {
    /// Calls method `ordinal` with `params`; returns the decoded results,
    /// the method's error `E`, or a transport error. `params` and its
    /// capabilities move into the call: a request that is `too_large` drops
    /// them, and the transport owns them once it is called.
    pub fn call<'s, Q: Encode<C>, R: Decode<'s, C>, E: Decode<'s, C>>(
        &'s mut self,
        ordinal: u32,
        version: u32,
        params: Q,
    ) -> Result<R, CallError<E>> {
        let mut caps = CapList::new();
        let n = encode_request(&mut self.request, &mut caps, ordinal, version, params)
            .map_err(|_| CallError::Transport(TransportError::TooLarge))?;
        let mut reply_caps = CapList::new();
        let m = self
            .transport
            .call(&self.request[..n], caps, &mut self.reply, &mut reply_caps)
            .map_err(CallError::Transport)?;
        let this: &'s Self = self;
        let msg = this
            .reply
            .get(..m)
            .ok_or(CallError::Transport(TransportError::MalformedReply))?;
        decode_reply(msg, reply_caps, ordinal)
    }
}
