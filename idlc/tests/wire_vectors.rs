//! A06.02: the known-answer vectors of `spec/IDL.md` section 3.1
//! (`spec/adr/0008-wire-known-answer-vectors.md`). Each value encodes to
//! exactly the vector's bytes, and each vector decodes to exactly the value.
//! The bytes below are copied from the spec text; never regenerate them from
//! the encoder. Where this file and the code disagree, the code is wrong.

use silica_idl_rt::{Array, CallError, CapList, Capability, Header, Status, TransportError};

include!(concat!(env!("OUT_DIR"), "/sample_echo.rs"));

use echo::{
    EchoCapsReply, EchoCapsRequest, EchoError, EchoRecordsReply, EchoRecordsRequest, EchoReply,
    EchoRequest, Kind, Record,
};

#[derive(Clone, Debug, PartialEq)]
struct Cap(u32);

impl Capability for Cap {}

/// V1, `echo` request, `data` = "hi".
const V1: [u8; 26] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
    0x06, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x68, 0x69,
];

/// V2, `echo` reply, status `ok`, `data` empty.
const V2: [u8; 24] = [
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// V3, `echo_records` request with one `Record`.
const V3: [u8; 94] = [
    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x4e, 0x00, 0x00, 0x00,
    0x4a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x42, 0x00, 0x00, 0x00, 0x01, 0x01, 0x12, 0x56,
    0x34, 0xde, 0xbc, 0x9a, 0x78, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xfe, 0xd4, 0xfe,
    0x90, 0xee, 0xfe, 0xff, 0xfb, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02, 0x00, 0x00, 0x00,
    0xc3, 0xa9, 0x02, 0x00, 0x00, 0x00, 0x00, 0xff, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x78, 0x00, 0x00, 0x00, 0x00, 0x01, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
];

/// V4, `echo_records` reply, status `error`, `EchoError.empty`.
const V4: [u8; 24] = [
    0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
];

/// V5, `echo_caps` request, capabilities 0, present 1, 2.
const V5: [u8; 24] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x02,
];

/// V6, `echo_caps` reply, status `ok`, capability 0, absent, capability 1.
const V6: [u8; 23] = [
    0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
];

/// V7, reply to ordinal 9, status `unknown_method`.
const V7: [u8; 16] = [
    0x09, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const V3_TAGS: [&str; 2] = ["x", ""];

fn v3_record() -> Record<'static, Cap> {
    Record {
        kind: Kind::Tagged,
        flag: true,
        a: 0x12,
        b: 0x3456,
        c: 0x789a_bcde,
        d: 0x0102_0304_0506_0708,
        e: -2,
        f: -300,
        g: -70_000,
        h: -5,
        name: "\u{e9}",
        blob: &[0x00, 0xff],
        tags: Array::new(&V3_TAGS),
        parent: Some(0x1122_3344_5566_7788),
    }
}

/// Encodes with `f` and checks the bytes and the capability list.
fn encodes_to(
    name: &str,
    want: &[u8],
    want_caps: &[Cap],
    f: impl FnOnce(&mut [u8], &mut CapList<Cap>) -> usize,
) {
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let n = f(&mut buf, &mut cl);
    assert_eq!(
        &buf[..n],
        want,
        "{name}: encoded bytes differ from spec/IDL.md 3.1"
    );
    assert_eq!(
        cl.iter().cloned().collect::<Vec<_>>(),
        want_caps.to_vec(),
        "{name}: capability list"
    );
}

fn cap_list(caps: &[Cap]) -> CapList<Cap> {
    CapList::from_slice(caps).expect("at most 4 capabilities")
}

#[test]
fn known_answer_vectors() {
    let none: CapList<Cap> = CapList::new();
    let rec = [v3_record()];
    let [c0, c1, c2] = [Cap(100), Cap(101), Cap(102)];
    assert_eq!(echo::VERSION, 1, "the vectors use sender version 1");

    // V1.
    let v1 = EchoRequest { data: b"hi" };
    encodes_to("V1", &V1, &[], |b, c| {
        v1.clone().encode_message(b, c).unwrap()
    });
    assert_eq!(EchoRequest::decode_message(&V1, none.clone()), Ok(v1));

    // V2.
    let v2 = EchoReply { data: b"" };
    encodes_to("V2", &V2, &[], |b, c| {
        v2.clone().encode_message(b, c).unwrap()
    });
    assert_eq!(EchoReply::decode_message(&V2, none.clone()), Ok(v2));

    // V3.
    let v3 = EchoRecordsRequest {
        records: Array::new(&rec),
    };
    encodes_to("V3", &V3, &[], |b, c| {
        v3.clone().encode_message(b, c).unwrap()
    });
    let got = EchoRecordsRequest::decode_message(&V3, none.clone()).expect("V3 decodes");
    assert_eq!(got, v3);
    let back: Vec<Record<'_, Cap>> = got.records.iter().collect();
    assert_eq!(back, vec![v3_record()], "V3 record values");
    assert_eq!(back[0].tags.iter().collect::<Vec<_>>(), V3_TAGS.to_vec());

    // V4.
    encodes_to("V4", &V4, &[], |b, c| {
        silica_idl_rt::encode_error_reply(b, c, 2, echo::VERSION, &EchoError::Empty).unwrap()
    });
    assert_eq!(
        EchoRecordsReply::decode_message(&V4, none.clone()),
        Err(CallError::Error(EchoError::Empty))
    );

    // V5: three capabilities transferred.
    let v5 = EchoCapsRequest {
        frame: c0.clone(),
        peer: Some(c1.clone()),
        any: c2.clone(),
    };
    let v5_caps = [c0.clone(), c1.clone(), c2.clone()];
    encodes_to("V5", &V5, &v5_caps, |b, c| {
        v5.clone().encode_message(b, c).unwrap()
    });
    assert_eq!(
        EchoCapsRequest::decode_message(&V5, cap_list(&v5_caps)),
        Ok(v5)
    );

    // V6: two capabilities transferred.
    let v6 = EchoCapsReply {
        frame: c0.clone(),
        peer: None,
        any: c1.clone(),
    };
    let v6_caps = [c0.clone(), c1.clone()];
    encodes_to("V6", &V6, &v6_caps, |b, c| {
        v6.clone().encode_message(b, c).unwrap()
    });
    assert_eq!(
        EchoCapsReply::decode_message(&V6, cap_list(&v6_caps)),
        Ok(v6)
    );

    // V7.
    encodes_to("V7", &V7, &[], |b, c| {
        silica_idl_rt::encode_status_reply(b, c, 9, echo::VERSION, Status::UnknownMethod).unwrap()
    });
    let h = Header::decode(&V7).expect("V7 header decodes");
    assert_eq!(
        (h.ordinal, h.reply, h.status, h.version, h.payload_len),
        (9, true, Status::UnknownMethod, 1, 0)
    );
    // The same status on a known method's reply surfaces as a transport error.
    let mut v7_echo = V7;
    v7_echo[0] = 1;
    assert_eq!(
        EchoReply::decode_message(&v7_echo, none.clone()),
        Err(CallError::Transport(TransportError::UnknownMethod))
    );
}
