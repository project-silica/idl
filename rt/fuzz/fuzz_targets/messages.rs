//! Every message a provider or a client receives crosses a trust boundary:
//! its bytes and capabilities come from another program. Given any bytes and
//! up to four capabilities (`spec/IDL.md` section 3):
//!
//! - decoding as any message of the sample protocol never panics;
//! - whatever decodes re-encodes, and the re-encoding decodes to an equal
//!   value (a decoder never accepts what the encoder cannot say);
//! - a provider's `dispatch` answers anything with a well-formed reply and
//!   never panics.
//!
//! The first input byte picks how many capabilities arrive with the message
//! and what the transport reports of each one's object type.
#![no_main]

use libfuzzer_sys::fuzz_target;
use silica_idl_rt::{Array, CapList, Capability, Header, MAX_CAPS, MAX_MESSAGE};

include!(concat!(env!("OUT_DIR"), "/sample_echo.rs"));

use echo::{
    EchoCapsReply, EchoCapsRequest, EchoError, EchoRecordsReply, EchoRecordsRequest, EchoReply,
    EchoRequest, Record,
};

/// A stand-in capability and the object type its transport reports.
#[derive(Clone, Debug, PartialEq)]
struct Cap(u32, Option<&'static str>);

impl Capability for Cap {
    fn object_type(&self) -> Option<&str> {
        self.1
    }
}

/// What a transport may report of a capability: the types the sample
/// protocol's typed fields declare, another, or nothing.
const TYPES: [Option<&str>; 4] = [Some("Frame"), Some("Endpoint"), Some("Thread"), None];

/// The sample provider: echoes everything; refuses an empty record list.
struct Server;

impl echo::Server<Cap> for Server {
    fn echo<'a>(&'a mut self, data: &'a [u8]) -> EchoReply<'a> {
        EchoReply { data }
    }
    fn echo_records<'a>(
        &'a mut self,
        records: Array<'a, Record<'a, Cap>, Cap>,
    ) -> Result<EchoRecordsReply<'a, Cap>, EchoError> {
        if records.is_empty() { Err(EchoError::Empty) } else { Ok(EchoRecordsReply { records }) }
    }
    fn echo_caps(&mut self, frame: Cap, peer: Option<Cap>, any: Cap) -> EchoCapsReply<Cap> {
        EchoCapsReply { frame, peer, any }
    }
}

macro_rules! round_trip {
    ($t:ty, $msg:expr, $caps:expr) => {
        if let Ok(v) = <$t>::decode_message($msg, $caps.clone()) {
            let mut buf = [0u8; MAX_MESSAGE];
            let mut cl: CapList<Cap> = CapList::new();
            let n = v.clone().encode_message(&mut buf, &mut cl).expect(concat!(stringify!($t), ": a decoded message re-encodes"));
            let w = <$t>::decode_message(&buf[..n], cl).expect(concat!(stringify!($t), ": the re-encoding decodes"));
            assert_eq!(v, w, concat!(stringify!($t), ": the re-encoding decodes to an equal value"));
        }
    };
}

fuzz_target!(|data: &[u8]| {
    let Some((&sel, msg)) = data.split_first() else { return };
    let mut caps = CapList::new();
    // The first byte's remainder picks how many capabilities arrive, the
    // rest what the transport reports of each.
    for i in 0..(sel as usize % (MAX_CAPS + 1)) {
        let _ = caps.push(Cap(i as u32, TYPES[(sel as usize / (MAX_CAPS + 1) + i) % TYPES.len()]));
    }
    round_trip!(EchoRequest, msg, caps);
    round_trip!(EchoReply, msg, caps);
    round_trip!(EchoRecordsRequest<Cap>, msg, caps);
    round_trip!(EchoRecordsReply<Cap>, msg, caps);
    round_trip!(EchoCapsRequest<Cap>, msg, caps);
    round_trip!(EchoCapsReply<Cap>, msg, caps);

    let mut reply = [0u8; MAX_MESSAGE];
    let mut reply_caps = CapList::new();
    let n = echo::dispatch(&mut Server, msg, caps, &mut reply, &mut reply_caps);
    assert!(n <= MAX_MESSAGE, "dispatch reports more bytes than its buffer");
    Header::decode(&reply[..n]).expect("dispatch always answers with a well-formed header");
});
