//! The M0 exit test (`spec/MILESTONES.md`): every message of the
//! sample protocol round-trips through the generated encode and decode with
//! equal values, also through the generated client, `dispatch` and server;
//! malformed input (`spec/IDL.md` section 3) is rejected, never a panic.

use std::convert::Infallible;
use std::path::Path;

use silica_idl_rt::{
    Array, CallError, CapList, Capability, DecodeError, Status, Transport, TransportError,
};

include!(concat!(env!("OUT_DIR"), "/sample_echo.rs"));

use echo::{
    EchoCapsReply, EchoCapsRequest, EchoError, EchoRecordsReply, EchoRecordsRequest, EchoReply,
    EchoRequest, Kind, Record,
};

/// Stand-in capability: the tests only need it to be cloned and compared.
#[derive(Clone, Debug, PartialEq)]
struct Cap(u32);

impl Capability for Cap {}

/// The sample provider: echoes everything; refuses an empty record list.
struct EchoServer;

impl echo::Server<Cap> for EchoServer {
    fn echo<'a>(&'a mut self, data: &'a [u8]) -> EchoReply<'a> {
        EchoReply { data }
    }

    fn echo_records<'a>(
        &'a mut self,
        records: Array<'a, Record<'a, Cap>, Cap>,
    ) -> Result<EchoRecordsReply<'a, Cap>, EchoError> {
        if records.is_empty() {
            Err(EchoError::Empty)
        } else {
            Ok(EchoRecordsReply { records })
        }
    }

    fn echo_caps(&mut self, frame: Cap, peer: Option<Cap>, any: Cap) -> EchoCapsReply<Cap> {
        EchoCapsReply { frame, peer, any }
    }
}

/// A transport that hands the request straight to `dispatch`.
struct Loopback(EchoServer);

impl Transport<Cap> for Loopback {
    fn call(
        &mut self,
        message: &mut [u8],
        len: usize,
        caps: CapList<Cap>,
        reply_caps: &mut CapList<Cap>,
    ) -> Result<usize, TransportError> {
        let request = message[..len].to_vec();
        let (request, reply) = (&request[..], message);
        Ok(echo::dispatch(
            &mut self.0,
            request,
            caps,
            reply,
            reply_caps,
        ))
    }
}

const TAGS_A: [&str; 2] = ["alpha", "beta"];
const TAGS_B: [&str; 0] = [];

fn records() -> [Record<'static, Cap>; 2] {
    [
        Record {
            kind: Kind::Tagged,
            flag: true,
            a: 0xfe,
            b: 0xbeef,
            c: 0xdead_beef,
            d: u64::MAX - 1,
            e: -2,
            f: i16::MIN,
            g: -123_456,
            h: i64::MIN + 7,
            name: "first é",
            blob: &[0, 1, 2, 255],
            tags: Array::new(&TAGS_A),
            parent: Some(42),
        },
        Record {
            kind: Kind::Unknown(9),
            flag: false,
            a: 0,
            b: 0,
            c: 0,
            d: 0,
            e: i8::MAX,
            f: 1,
            g: i32::MAX,
            h: -1,
            name: "",
            blob: &[],
            tags: Array::new(&TAGS_B),
            parent: None,
        },
    ]
}

fn caps(n: u32) -> [Cap; 3] {
    [Cap(n), Cap(n + 1), Cap(n + 2)]
}

/// Every request and reply message of the protocol, encoded, with how many
/// capabilities it carries.
fn all_messages() -> Vec<(&'static str, Vec<u8>, CapList<Cap>)> {
    let recs = records();
    let [c0, c1, c2] = caps(10);
    let mut out = Vec::new();
    let mut push = |name: &'static str, f: &dyn Fn(&mut [u8], &mut CapList<Cap>) -> usize| {
        let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
        let mut cl = CapList::new();
        let n = f(&mut buf, &mut cl);
        out.push((name, buf[..n].to_vec(), cl));
    };
    push("echo request", &|b, c| {
        EchoRequest { data: b"hello" }.encode_message(b, c).unwrap()
    });
    push("echo reply", &|b, c| {
        EchoReply { data: b"" }.encode_message(b, c).unwrap()
    });
    push("echo_records request", &|b, c| {
        EchoRecordsRequest {
            records: Array::new(&recs),
        }
        .encode_message(b, c)
        .unwrap()
    });
    push("echo_records reply", &|b, c| {
        EchoRecordsReply {
            records: Array::new(&recs[..1]),
        }
        .encode_message(b, c)
        .unwrap()
    });
    push("echo_records error reply", &|b, c| {
        silica_idl_rt::encode_error_reply(b, c, 2, echo::VERSION, &EchoError::TooMany).unwrap()
    });
    push("echo_caps request", &|b, c| {
        EchoCapsRequest {
            frame: c0.clone(),
            peer: Some(c1.clone()),
            any: c2.clone(),
        }
        .encode_message(b, c)
        .unwrap()
    });
    push("echo_caps reply", &|b, c| {
        EchoCapsReply {
            frame: c0.clone(),
            peer: None,
            any: c2.clone(),
        }
        .encode_message(b, c)
        .unwrap()
    });
    out
}

/// Decodes `msg` as whichever message it is; `Ok` only if it decodes fully.
fn decode_any(name: &str, msg: &[u8], caps: &CapList<Cap>) -> Result<(), String> {
    match name {
        "echo request" => EchoRequest::decode_message(msg, caps.clone())
            .map(drop)
            .map_err(|e| e.to_string()),
        "echo reply" => EchoReply::decode_message(msg, caps.clone())
            .map(drop)
            .map_err(|e| format!("{e:?}")),
        "echo_records request" => EchoRecordsRequest::decode_message(msg, caps.clone())
            .map(drop)
            .map_err(|e| e.to_string()),
        "echo_records reply" => EchoRecordsReply::decode_message(msg, caps.clone())
            .map(drop)
            .map_err(|e| format!("{e:?}")),
        "echo_records error reply" => match EchoRecordsReply::decode_message(msg, caps.clone()) {
            Err(CallError::Error(_)) => Ok(()),
            other => Err(format!("{other:?}")),
        },
        "echo_caps request" => EchoCapsRequest::decode_message(msg, caps.clone())
            .map(drop)
            .map_err(|e| e.to_string()),
        "echo_caps reply" => EchoCapsReply::decode_message(msg, caps.clone())
            .map(drop)
            .map_err(|e| format!("{e:?}")),
        _ => unreachable!("{name}"),
    }
}

fn header_payload_len(msg: &mut [u8]) {
    let n = (msg.len() - silica_idl_rt::HEADER_LEN) as u32;
    msg[12..16].copy_from_slice(&n.to_le_bytes());
}

#[test]
fn echo_round_trip() {
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let recs = records();
    let [c0, c1, c2] = caps(1);

    // Requests: encode, decode, equal.
    let req = EchoRequest {
        data: b"\x00bytes\xff",
    };
    let n = req.clone().encode_message(&mut buf, &mut cl).unwrap();
    assert_eq!(&buf[..4], &1u32.to_le_bytes(), "ordinal");
    assert_eq!(
        &buf[12..16],
        &((n - 16) as u32).to_le_bytes(),
        "payload length"
    );
    assert_eq!(
        EchoRequest::decode_message(&buf[..n], cl.clone()),
        Ok(req.clone())
    );

    let req = EchoRecordsRequest {
        records: Array::new(&recs),
    };
    let n = req.clone().encode_message(&mut buf, &mut cl).unwrap();
    let got = EchoRecordsRequest::decode_message(&buf[..n], cl.clone()).unwrap();
    assert_eq!(got, req);
    assert_eq!(got.records.len(), 2);
    let back: Vec<Record<'_, Cap>> = got.records.iter().collect();
    assert_eq!(back, recs.to_vec());
    assert_eq!(back[0].tags.iter().collect::<Vec<_>>(), TAGS_A.to_vec());
    assert_eq!(back[1].kind, Kind::Unknown(9));

    let req = EchoCapsRequest {
        frame: c0.clone(),
        peer: Some(c1.clone()),
        any: c2.clone(),
    };
    let n = req.clone().encode_message(&mut buf, &mut cl).unwrap();
    assert_eq!(
        cl.iter().cloned().collect::<Vec<_>>(),
        vec![c0.clone(), c1.clone(), c2.clone()]
    );
    assert_eq!(
        EchoCapsRequest::decode_message(&buf[..n], cl.clone()),
        Ok(req.clone())
    );
    let req = EchoCapsRequest {
        frame: c0.clone(),
        peer: None,
        any: c2.clone(),
    };
    let n = req.clone().encode_message(&mut buf, &mut cl).unwrap();
    assert_eq!(cl.len(), 2);
    assert_eq!(
        EchoCapsRequest::decode_message(&buf[..n], cl.clone()),
        Ok(req.clone())
    );

    // Replies: ok, error, and the two status replies.
    let rep = EchoReply { data: b"" };
    let n = rep.clone().encode_message(&mut buf, &mut cl).unwrap();
    assert_eq!(EchoReply::decode_message(&buf[..n], cl.clone()), Ok(rep));
    let rep = EchoRecordsReply {
        records: Array::new(&recs[1..]),
    };
    let n = rep.clone().encode_message(&mut buf, &mut cl).unwrap();
    assert_eq!(
        EchoRecordsReply::decode_message(&buf[..n], cl.clone()),
        Ok(rep)
    );
    let rep = EchoCapsReply {
        frame: c2.clone(),
        peer: Some(c0.clone()),
        any: c1.clone(),
    };
    let n = rep.clone().encode_message(&mut buf, &mut cl).unwrap();
    assert_eq!(
        EchoCapsReply::decode_message(&buf[..n], cl.clone()),
        Ok(rep)
    );
    for err in [EchoError::Empty, EchoError::TooMany, EchoError::Unknown(77)] {
        let n =
            silica_idl_rt::encode_error_reply(&mut buf, &mut cl, 2, echo::VERSION, &err).unwrap();
        assert_eq!(
            EchoRecordsReply::decode_message(&buf[..n], cl.clone()),
            Err(CallError::Error(err))
        );
    }
    for (status, want) in [
        (Status::UnknownMethod, TransportError::UnknownMethod),
        (Status::MalformedRequest, TransportError::MalformedRequest),
    ] {
        let n = silica_idl_rt::encode_status_reply(&mut buf, &mut cl, 1, echo::VERSION, status)
            .unwrap();
        assert_eq!(n, 16);
        assert_eq!(
            EchoReply::decode_message(&buf[..n], cl.clone()),
            Err(CallError::<Infallible>::Transport(want))
        );
    }

    // Encoding is canonical: decoding and encoding again gives the same bytes.
    for (name, msg, cl) in all_messages() {
        let mut again = [0u8; silica_idl_rt::MAX_MESSAGE];
        let mut cl2 = CapList::new();
        let n = match name {
            "echo request" => EchoRequest::decode_message(&msg, cl.clone())
                .unwrap()
                .encode_message(&mut again, &mut cl2),
            "echo_records request" => EchoRecordsRequest::decode_message(&msg, cl.clone())
                .unwrap()
                .encode_message(&mut again, &mut cl2),
            "echo_records reply" => EchoRecordsReply::decode_message(&msg, cl.clone())
                .unwrap()
                .encode_message(&mut again, &mut cl2),
            "echo_caps request" => EchoCapsRequest::decode_message(&msg, cl.clone())
                .unwrap()
                .encode_message(&mut again, &mut cl2),
            _ => continue,
        }
        .unwrap();
        assert_eq!(&again[..n], &msg[..], "{name} re-encodes differently");
        assert_eq!(cl2, cl, "{name} capabilities");
    }

    // End to end: generated client -> transport -> dispatch -> server.
    let mut client: echo::Client<Loopback, Cap> = echo::Client::from(Loopback(EchoServer));
    assert_eq!(client.echo(b"ping"), Ok(EchoReply { data: b"ping" }));
    let got = client.echo_records(Array::new(&recs)).unwrap();
    assert_eq!(got.records, Array::new(&recs));
    assert_eq!(
        client.echo_records(Array::new(&[])),
        Err(CallError::Error(EchoError::Empty))
    );
    let got = client
        .echo_caps(c0.clone(), Some(c1.clone()), c2.clone())
        .unwrap();
    assert_eq!(
        got,
        EchoCapsReply {
            frame: c0.clone(),
            peer: Some(c1),
            any: c2.clone()
        }
    );
    let got = client.echo_caps(c2.clone(), None, c0.clone()).unwrap();
    assert_eq!(
        got,
        EchoCapsReply {
            frame: c2,
            peer: None,
            any: c0
        }
    );

    // Reserved and unknown ordinals answer `unknown_method`.
    let mut server = EchoServer;
    let empty: CapList<Cap> = CapList::new();
    for ordinal in [4u32, 5, u32::MAX] {
        let mut req = [0u8; 20];
        req[..4].copy_from_slice(&ordinal.to_le_bytes());
        req[8..12].copy_from_slice(&1u32.to_le_bytes());
        req[12..16].copy_from_slice(&4u32.to_le_bytes());
        let mut rcl = CapList::new();
        let n = echo::dispatch(&mut server, &req, empty.clone(), &mut buf, &mut rcl);
        let h = silica_idl_rt::Header::decode(&buf[..n]).unwrap();
        assert_eq!(
            (h.ordinal, h.reply, h.status, h.payload_len),
            (ordinal, true, Status::UnknownMethod, 0)
        );
    }
}

#[test]
fn rejects_truncated() {
    let mut server = EchoServer;
    let mut reply = [0u8; silica_idl_rt::MAX_MESSAGE];
    for (name, msg, cl) in all_messages() {
        decode_any(name, &msg, &cl).unwrap_or_else(|e| panic!("{name} does not decode: {e}"));
        for k in 0..msg.len() {
            // Cut short as it arrives.
            let cut = &msg[..k];
            assert!(
                decode_any(name, cut, &cl).is_err(),
                "{name} cut to {k} bytes decodes"
            );
            // Cut short with a header claiming the shorter length.
            if k >= silica_idl_rt::HEADER_LEN {
                let mut fixed = cut.to_vec();
                header_payload_len(&mut fixed);
                assert!(
                    decode_any(name, &fixed, &cl).is_err(),
                    "{name} cut to {k} bytes with a matching header decodes"
                );
                // A provider answers a cut request with `malformed_request`.
                if name.ends_with("request") {
                    let mut rcl = CapList::new();
                    let n = echo::dispatch(&mut server, &fixed, cl.clone(), &mut reply, &mut rcl);
                    let h = silica_idl_rt::Header::decode(&reply[..n]).unwrap();
                    assert_eq!(h.status, Status::MalformedRequest, "{name} cut to {k}");
                }
            }
        }
        // One byte too many in the payload is rejected too.
        let mut long = msg.clone();
        long.push(0);
        header_payload_len(&mut long);
        assert!(
            decode_any(name, &long, &cl).is_err(),
            "{name} with a trailing byte decodes"
        );
    }
}

/// The record list request with one record, and the offset of the record's
/// `flag` byte: header 16, list length 4, array count 4, record list length
/// 4, `kind` 1.
fn one_record_request() -> (Vec<u8>, usize) {
    let recs = records();
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let n = EchoRecordsRequest {
        records: Array::new(&recs[..1]),
    }
    .encode_message(&mut buf, &mut cl)
    .unwrap();
    (buf[..n].to_vec(), 16 + 4 + 4 + 4 + 1)
}

#[test]
fn rejects_malformed() {
    let none: CapList<Cap> = CapList::new();
    let (good, flag) = one_record_request();
    let decode = |m: &[u8]| EchoRecordsRequest::decode_message(m, none.clone()).map(drop);
    decode(&good).unwrap();
    let with = |at: usize, v: u8| {
        let mut m = good.clone();
        m[at] = v;
        m
    };

    // Header.
    assert_eq!(
        decode(&with(0, 1)),
        Err(DecodeError::Unexpected),
        "ordinal 1 is another method"
    );
    let mut zero = good.clone();
    zero[..4].copy_from_slice(&[0; 4]);
    assert_eq!(decode(&zero), Err(DecodeError::BadHeader), "ordinal 0");
    assert_eq!(
        decode(&with(4, 2)),
        Err(DecodeError::BadHeader),
        "reserved flag bit"
    );
    assert_eq!(
        decode(&with(5, 0x80)),
        Err(DecodeError::BadHeader),
        "reserved flag bit"
    );
    assert_eq!(
        decode(&with(6, 1)),
        Err(DecodeError::BadHeader),
        "status in a request"
    );
    assert_eq!(
        decode(&with(4, 1)),
        Err(DecodeError::Unexpected),
        "a reply is not a request"
    );
    assert_eq!(
        decode(&with(12, good[12] + 1)),
        Err(DecodeError::BadHeader),
        "payload length"
    );
    // The sender's version is informational.
    assert_eq!(decode(&with(8, 99)), Ok(()));
    let mut reply = with(4, 1);
    reply[6] = 4;
    assert!(
        EchoRecordsReply::decode_message(&reply, none.clone()).is_err(),
        "status 4 (reply_too_large) with a payload"
    );
    reply[6] = 5;
    assert!(
        EchoRecordsReply::decode_message(&reply, none.clone()).is_err(),
        "status 5 (unknown_field) with a payload"
    );
    reply[6] = 6;
    assert!(
        EchoRecordsReply::decode_message(&reply, none.clone()).is_err(),
        "unknown status"
    );
    let big = vec![0u8; silica_idl_rt::MAX_MESSAGE + 1];
    assert_eq!(decode(&big), Err(DecodeError::TooLarge));

    // Values.
    assert_eq!(decode(&with(flag, 2)), Err(DecodeError::BadBool));
    let name_at = flag + 1 + 1 + 2 + 4 + 8 + 1 + 2 + 4 + 8 + 4;
    assert_eq!(good[name_at], b'f', "name starts here");
    assert_eq!(decode(&with(name_at, 0xff)), Err(DecodeError::BadUtf8));
    let parent_tag = good.len() - 9;
    assert_eq!(good[parent_tag], 1, "parent is present");
    assert_eq!(decode(&with(parent_tag, 2)), Err(DecodeError::BadOption));
    // An array count larger than the bytes that remain, before any loop.
    let mut count = good.clone();
    count[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(decode(&count), Err(DecodeError::Truncated));
    // A field list longer than its parent.
    let mut list = good.clone();
    list[16..20].copy_from_slice(&0x1000u32.to_le_bytes());
    assert_eq!(decode(&list), Err(DecodeError::Truncated));

    // Capabilities: in encoding order, each referenced exactly once, at most 4.
    let [c0, c1, c2] = caps(1);
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let n = EchoCapsRequest {
        frame: c0.clone(),
        peer: Some(c1.clone()),
        any: c2.clone(),
    }
    .encode_message(&mut buf, &mut cl)
    .unwrap();
    let msg = buf[..n].to_vec();
    // Payload: list length 4, frame index, option tag, peer index, any index.
    assert_eq!(&msg[20..], &[0, 1, 1, 2]);
    let decode_caps =
        |m: &[u8], c: &CapList<Cap>| EchoCapsRequest::decode_message(m, c.clone()).map(drop);
    decode_caps(&msg, &cl).unwrap();
    let mut swapped = msg.clone();
    swapped[20] = 1;
    swapped[22] = 0;
    assert_eq!(decode_caps(&swapped, &cl), Err(DecodeError::BadCapIndex));
    let mut past = msg.clone();
    past[23] = 3;
    assert_eq!(decode_caps(&past, &cl), Err(DecodeError::BadCapIndex));
    let mut extra = cl.clone();
    extra.push(Cap(99)).unwrap();
    assert_eq!(decode_caps(&msg, &extra), Err(DecodeError::UnusedCap));
    let short = CapList::from_slice(&[c0.clone(), c1.clone()]).unwrap();
    assert_eq!(decode_caps(&msg, &short), Err(DecodeError::BadCapIndex));
    let mut absent = msg.clone();
    absent[21] = 0;
    absent.remove(22);
    header_payload_len(&mut absent);
    absent[16..20].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(
        decode_caps(&absent, &cl),
        Err(DecodeError::BadCapIndex),
        "indices skip 1"
    );
    assert!(
        CapList::from_slice(&[c0, c1, c2, Cap(4), Cap(5)]).is_none(),
        "at most 4"
    );

    // A provider answers garbage with `malformed_request`, never panics.
    let mut server = EchoServer;
    let mut out = [0u8; silica_idl_rt::MAX_MESSAGE];
    for bad in [
        &with(flag, 2)[..],
        &with(4, 2)[..],
        &[][..],
        &[7, 0, 0][..],
        &swapped[..],
    ] {
        let mut rcl = CapList::new();
        let n = echo::dispatch(&mut server, bad, cl.clone(), &mut out, &mut rcl);
        let h = silica_idl_rt::Header::decode(&out[..n]).unwrap_or_else(|_| {
            // Too short to carry an ordinal: the reply's ordinal is 0.
            assert!(bad.len() < 4);
            silica_idl_rt::Header {
                ordinal: 0,
                reply: true,
                status: Status::MalformedRequest,
                version: echo::VERSION,
                payload_len: 0,
            }
        });
        assert_eq!(h.status, Status::MalformedRequest);
    }
}

#[test]
fn too_large_is_detected_before_sending() {
    let big = vec![0u8; silica_idl_rt::MAX_MESSAGE];
    let mut client: echo::Client<Loopback, Cap> = echo::Client::from(Loopback(EchoServer));
    assert_eq!(
        client.echo(&big),
        Err(CallError::Transport(TransportError::TooLarge))
    );
    let fits = vec![7u8; silica_idl_rt::MAX_MESSAGE - 16 - 4 - 4];
    assert_eq!(client.echo(&fits).map(|r| r.data.len()), Ok(fits.len()));
}

#[test]
fn generation_is_deterministic() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(idlc::PROTOCOL_DIR)
        .join("sample")
        .join("echo.idl");
    let ps = idlc::load(std::slice::from_ref(&path)).unwrap();
    let a = idlc::rustgen::rust(&ps[0]).unwrap();
    let ps = idlc::load(std::slice::from_ref(&path)).unwrap();
    assert_eq!(a, idlc::rustgen::rust(&ps[0]).unwrap());
    assert_eq!(idlc::rustgen::file_name(&ps[0]), "sample_echo.rs");
}

#[test]
fn generator_rejects_reserved_names() {
    let src = "protocol x.Y version 1 {\n  struct Client { a: u8 }\n  enum E : u8 { unknown = 0 }\n  ping @1 () -> ();\n  struct PingReply {}\n}\n";
    let ps = idlc::compile(&[("x/y.idl".into(), src.into())]).unwrap();
    let errs = idlc::rustgen::rust(&ps[0]).unwrap_err();
    let text: Vec<String> = errs.iter().map(ToString::to_string).collect();
    assert_eq!(errs.len(), 3, "{text:?}");
    assert!(
        text[0].contains("x/y.idl:2:10") && text[0].contains("`Client` is reserved"),
        "{text:?}"
    );
    assert!(text.iter().any(|t| t.contains("`Unknown`")), "{text:?}");
    assert!(text.iter().any(|t| t.contains("`PingReply`")), "{text:?}");
}

/// A request too short to carry an ordinal, or carrying 0, is still answered,
/// with `malformed_request` and ordinal 0 (`spec/IDL.md` section 4): a reply
/// the caller can decode. Ordinal 0 is valid in no other message.
#[test]
fn answers_a_request_without_an_ordinal() {
    let mut server = EchoServer;
    let mut reply = [0u8; silica_idl_rt::MAX_MESSAGE];
    for req in [&[][..], &[1, 0][..], &[0u8; 16][..]] {
        let mut rcl = CapList::new();
        let n = echo::dispatch(
            &mut server,
            req,
            CapList::<Cap>::new(),
            &mut reply,
            &mut rcl,
        );
        let h = silica_idl_rt::Header::decode(&reply[..n]).expect("the reply decodes");
        assert_eq!(
            (h.ordinal, h.reply, h.status),
            (0, true, Status::MalformedRequest),
            "request {req:?}"
        );
    }
    // Ordinal 0 with any other status, or in a request, is malformed.
    let mut m = reply;
    m[6] = Status::UnknownMethod as u8;
    assert!(silica_idl_rt::Header::decode(&m[..16]).is_err());
    let mut q = [0u8; 16];
    q[6] = Status::MalformedRequest as u8;
    assert!(silica_idl_rt::Header::decode(&q).is_err());
}
