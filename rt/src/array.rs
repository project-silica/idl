//! `array<T>`: a slice when built by the sender, a view of the input when
//! decoded (decoding allocates nothing).

use core::fmt;

use crate::codec::{Decode, DecodeError, Decoder, Encode, EncodeError, Encoder};

/// An `array<T>` value. Build one with [`Array::new`]; a decoded one is a
/// view that decodes its elements again when iterated (they were validated
/// when the array was decoded). Two arrays are equal when their elements are.
/// Elements carry no capabilities: generated Rust rejects an `array` whose
/// element type can carry one (`spec/adr/0010-capabilities-in-the-rust-mapping.md`).
pub struct Array<'a, T, C> {
    repr: Repr<'a, T, C>,
}

enum Repr<'a, T, C> {
    Slice(&'a [T]),
    Wire { at: Decoder<'a, 'a, C>, len: usize },
}

impl<T, C> Clone for Array<'_, T, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, C> Copy for Array<'_, T, C> {}

impl<T, C> Clone for Repr<'_, T, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, C> Copy for Repr<'_, T, C> {}

impl<'a, T, C> Array<'a, T, C> {
    /// An array of the given elements.
    pub const fn new(items: &'a [T]) -> Self {
        Self {
            repr: Repr::Slice(items),
        }
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        match self.repr {
            Repr::Slice(s) => s.len(),
            Repr::Wire { len, .. } => len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The elements, in order.
    pub fn iter(&self) -> ArrayIter<'a, T, C>
    where
        T: Decode<'a, C> + Clone,
    {
        ArrayIter {
            repr: self.repr,
            index: 0,
        }
    }
}

impl<T, C> Default for Array<'_, T, C> {
    fn default() -> Self {
        Self::new(&[])
    }
}

/// Iterator over an [`Array`]'s elements.
pub struct ArrayIter<'a, T, C> {
    repr: Repr<'a, T, C>,
    index: usize,
}

impl<'a, T: Decode<'a, C> + Clone, C> Iterator for ArrayIter<'a, T, C> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        let item = match &mut self.repr {
            Repr::Slice(s) => s.get(self.index).cloned(),
            Repr::Wire { at, len } => {
                if self.index < *len {
                    // Validated when the array was decoded, so this cannot
                    // fail; if it did, iteration would just end.
                    T::decode(at).ok()
                } else {
                    None
                }
            }
        };
        if item.is_some() {
            self.index += 1;
        }
        item
    }
}

impl<'a, T: Encode<C> + Decode<'a, C>, C> Encode<C> for Array<'a, T, C> {
    fn encode(&self, e: &mut Encoder<'_, C>) -> Result<(), EncodeError> {
        e.put_len(self.len())?;
        match self.repr {
            Repr::Slice(s) => {
                for v in s {
                    v.encode(e)?;
                }
            }
            Repr::Wire { mut at, len } => {
                for _ in 0..len {
                    // Validated when the array was decoded.
                    T::decode(&mut at)
                        .map_err(|_| EncodeError::TooLarge)?
                        .encode(e)?;
                }
            }
        }
        Ok(())
    }
}

impl<'a, T: Decode<'a, C>, C> Decode<'a, C> for Array<'a, T, C> {
    fn decode(d: &mut Decoder<'a, '_, C>) -> Result<Self, DecodeError> {
        let len = d.take_len()?;
        // Elements carry no capabilities: decode them without any, so one
        // that claims a capability is malformed.
        let start = d.bytes_only();
        let mut at = start;
        for _ in 0..len {
            T::decode(&mut at)?;
        }
        d.skip_to(&at);
        Ok(Self {
            repr: Repr::Wire {
                at: Decoder::since(start, &at),
                len,
            },
        })
    }

    fn missing() -> Option<Self> {
        Some(Self::new(&[]))
    }
}

impl<'a, T: Decode<'a, C> + Clone + PartialEq, C> PartialEq for Array<'a, T, C> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<'a, T: Decode<'a, C> + Clone + fmt::Debug, C> fmt::Debug for Array<'a, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
