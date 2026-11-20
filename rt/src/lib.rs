//! Runtime of the code `silica-idlc` generates (`spec/IDL.md` sections 3, 5 and 6).
//!
//! `no_std`, no `alloc`, no `unsafe`. It holds the message header, the encoder
//! and decoder of the wire format, the errors, the transport trait, and the
//! client half every generated client wraps.
//!
//! Capabilities are a type parameter `C` (any type implementing
//! [`Capability`]); they travel in a [`CapList`] next to the bytes, never as
//! bytes. They move: encoding a value moves its capabilities into the list,
//! decoding moves them out of it, and nothing clones them
//! (`spec/IDL_RATIONALE.md` section 5). Host tests use plain
//! stand-in values.
//!
//! Decoding is total and allocates nothing: strings, bytes and arrays decode
//! to views of the input.

#![no_std]

mod array;
mod client;
mod codec;
mod message;

pub use array::{Array, ArrayIter};
pub use client::{CallError, ClientCore, Transport, TransportError};
pub use codec::{Decode, DecodeError, Decoder, Encode, EncodeError, Encoder, One, missing};
pub use message::{
    Header, Status, decode_reply, decode_request, encode_error_reply, encode_message, encode_reply,
    encode_request, encode_status_reply, request_ordinal, status_reply,
};

use core::cell::Cell;

/// Header length in bytes.
pub const HEADER_LEN: usize = 16;
/// Largest message, header included: what one kernel message carries
/// (500 words; `kernel/abi`). Anything larger travels in a shared Frame.
pub const MAX_MESSAGE: usize = 4000;
/// Most capabilities one message carries.
pub const MAX_CAPS: usize = 4;
/// Deepest nesting of field lists (the payload's own list is depth 1).
pub const MAX_DEPTH: u32 = 32;
/// Header flag bit 0: the message is a reply.
pub const FLAG_REPLY: u16 = 1;

/// A capability as the transport hands it to generated code. The kernel's
/// capability handle implements this; host tests use any stand-in. It need
/// not be `Clone`: capabilities move (`spec/IDL.md` section 3).
pub trait Capability {
    /// The kernel object type name the transport learned on receipt
    /// (`"Frame"`, `"Endpoint"`, ...), or `None` when the kernel did not
    /// report one. Generated decoding rejects a `cap<T>` reporting another
    /// name than `T`, and an `endpoint<P>` reporting another name than
    /// `"Endpoint"`; `None` is never rejected.
    fn object_type(&self) -> Option<&str> {
        None
    }
}

/// The capabilities of one message, in transfer order (at most
/// [`MAX_CAPS`]). Index `i` of the list is the capability whose one-byte
/// index in the payload is `i`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapList<C> {
    slots: [Option<C>; MAX_CAPS],
    len: usize,
}

impl<C> Default for CapList<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> CapList<C> {
    /// An empty list.
    pub const fn new() -> Self {
        Self {
            slots: [None, None, None, None],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Drops every capability in the list.
    pub fn clear(&mut self) {
        for s in &mut self.slots {
            *s = None;
        }
        self.len = 0;
    }

    /// Appends `cap`; gives it back when the list is full.
    pub fn push(&mut self, cap: C) -> Result<(), C> {
        match self.slots.get_mut(self.len) {
            Some(slot) => {
                *slot = Some(cap);
                self.len += 1;
                Ok(())
            }
            None => Err(cap),
        }
    }

    /// The capability at `index`.
    pub fn get(&self, index: usize) -> Option<&C> {
        self.slots.get(index).and_then(Option::as_ref)
    }

    /// The capabilities in transfer order.
    pub fn iter(&self) -> impl Iterator<Item = &C> {
        self.slots.iter().filter_map(Option::as_ref)
    }

    /// Moves the capability at `index` out, leaving the slot empty (the
    /// length is unchanged).
    pub fn take(&mut self, index: usize) -> Option<C> {
        self.slots.get_mut(index).and_then(Option::take)
    }

    /// The slots in use, each of which decoding may empty.
    pub(crate) fn cells(&mut self) -> &[Cell<Option<C>>] {
        Cell::from_mut(&mut self.slots[..self.len]).as_slice_of_cells()
    }
}

impl<C> IntoIterator for CapList<C> {
    type Item = C;
    type IntoIter = core::iter::Flatten<core::array::IntoIter<Option<C>, MAX_CAPS>>;

    /// The capabilities in transfer order, by value.
    fn into_iter(self) -> Self::IntoIter {
        self.slots.into_iter().flatten()
    }
}

impl<C: Clone> CapList<C> {
    /// A list holding clones of `caps`, or `None` if there are more than
    /// [`MAX_CAPS`].
    pub fn from_slice(caps: &[C]) -> Option<Self> {
        let mut l = Self::new();
        for c in caps {
            l.push(c.clone()).ok()?;
        }
        Some(l)
    }
}
