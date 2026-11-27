//! The transport design of `spec/IDL_RATIONALE.md` section 5
//! (`spec/IDL.md` sections 3 and 5). Capabilities carry an object type that
//! decoding checks; they are not `Clone` and move into a call; a reply that
//! exceeds the limits becomes status 4 `reply_too_large`.
//!
//! The stand-in capability here is deliberately not `Clone`, and counts the
//! live instances, so a test sees every capability that is duplicated, lost
//! or released.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use silica_idl_rt::{
    Array, CallError, CapList, Capability, DecodeError, Header, Status, Transport, TransportError,
};

include!(concat!(env!("OUT_DIR"), "/sample_echo.rs"));
include!(concat!(env!("OUT_DIR"), "/transport/hold_holder.rs"));

use echo::{EchoCapsReply, EchoCapsRequest, EchoError, EchoReply, Record};

/// A capability stand-in: not `Clone`; it reports an object type, and
/// `live` counts the instances alive.
struct Cap {
    id: u32,
    ty: Option<&'static str>,
    live: Rc<Cell<i32>>,
}

impl Cap {
    fn new(live: &Rc<Cell<i32>>, id: u32, ty: Option<&'static str>) -> Cap {
        live.set(live.get() + 1);
        Cap {
            id,
            ty,
            live: live.clone(),
        }
    }
}

impl Drop for Cap {
    fn drop(&mut self) {
        self.live.set(self.live.get() - 1);
    }
}

impl Capability for Cap {
    fn object_type(&self) -> Option<&str> {
        self.ty
    }
}

impl PartialEq for Cap {
    fn eq(&self, other: &Cap) -> bool {
        self.id == other.id
    }
}

impl fmt::Debug for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cap({})", self.id)
    }
}

/// The sample provider: echoes, except that `swap` makes `echo_caps` return
/// the frame and the peer exchanged, and `big` makes `echo` reply with more
/// than fits in a message.
#[derive(Default)]
struct EchoServer {
    swap: bool,
    big: bool,
}

static BIG: [u8; silica_idl_rt::MAX_MESSAGE] = [0x5a; silica_idl_rt::MAX_MESSAGE];

impl echo::Server<Cap> for EchoServer {
    fn echo<'a>(&'a mut self, data: &'a [u8]) -> EchoReply<'a> {
        if self.big {
            EchoReply { data: &BIG }
        } else {
            EchoReply { data }
        }
    }

    fn echo_records<'a>(
        &'a mut self,
        records: Array<'a, Record<'a, Cap>, Cap>,
    ) -> Result<echo::EchoRecordsReply<'a, Cap>, EchoError> {
        Ok(echo::EchoRecordsReply { records })
    }

    fn echo_caps(&mut self, frame: Cap, peer: Option<Cap>, any: Cap) -> EchoCapsReply<Cap> {
        match (self.swap, peer) {
            (true, Some(peer)) => EchoCapsReply {
                frame: peer,
                peer: Some(frame),
                any,
            },
            (_, peer) => EchoCapsReply { frame, peer, any },
        }
    }
}

/// A transport that hands the request straight to `dispatch`, counting the
/// calls and the capabilities it was given.
struct Loopback<S> {
    server: S,
    calls: u32,
    caps_in: usize,
}

impl<S> Loopback<S> {
    fn new(server: S) -> Self {
        Loopback {
            server,
            calls: 0,
            caps_in: 0,
        }
    }
}

impl Transport<Cap> for Loopback<EchoServer> {
    fn call(
        &mut self,
        message: &mut [u8],
        len: usize,
        caps: CapList<Cap>,
        reply_caps: &mut CapList<Cap>,
    ) -> Result<usize, TransportError> {
        let request = message[..len].to_vec();
        let (request, reply) = (&request[..], message);
        self.calls += 1;
        self.caps_in = caps.len();
        Ok(echo::dispatch(
            &mut self.server,
            request,
            caps,
            reply,
            reply_caps,
        ))
    }
}

/// A `hold.Holder` provider that keeps the frame it is given and answers
/// with a frame of its own and `reply_len` bytes of data.
struct HoldServer {
    kept: Vec<Cap>,
    reply_frame: Option<Cap>,
    reply_len: usize,
}

impl holder::Server<Cap> for HoldServer {
    fn put<'a>(&'a mut self, _data: &'a [u8], frame: Cap) -> holder::PutReply<'a, Cap> {
        self.kept.push(frame);
        holder::PutReply {
            data: &BIG[..self.reply_len],
            frame: self.reply_frame.take().expect("one call"),
        }
    }
}

impl Transport<Cap> for Loopback<HoldServer> {
    fn call(
        &mut self,
        message: &mut [u8],
        len: usize,
        caps: CapList<Cap>,
        reply_caps: &mut CapList<Cap>,
    ) -> Result<usize, TransportError> {
        let request = message[..len].to_vec();
        let (request, reply) = (&request[..], message);
        self.calls += 1;
        self.caps_in = caps.len();
        Ok(holder::dispatch(
            &mut self.server,
            request,
            caps,
            reply,
            reply_caps,
        ))
    }
}

/// A transport whose provider is gone: it takes the capabilities and, as the
/// failed call ends, releases them.
struct Gone {
    caps_in: usize,
}

impl Transport<Cap> for Gone {
    fn call(
        &mut self,
        _message: &mut [u8],
        _len: usize,
        caps: CapList<Cap>,
        _reply_caps: &mut CapList<Cap>,
    ) -> Result<usize, TransportError> {
        self.caps_in = caps.len();
        Err(TransportError::PeerGone)
    }
}

/// A reported object type, or none.
type Ty = Option<&'static str>;

/// The status of the reply `dispatch` wrote.
fn reply_status(reply: &[u8]) -> Status {
    Header::decode(reply).expect("reply header").status
}

/// A capability whose object type the transport cannot tell is neither a
/// `cap<T>` nor an `endpoint<P>`: the request is malformed and its
/// capabilities are released, as for one of another type. Before, an
/// untold type passed every check (the decoder failed open), so a
/// capability the transport could not inspect reached a server as the
/// frame or endpoint it had declared.
#[test]
fn a_typed_capability_of_untold_type_is_refused() {
    let live = Rc::new(Cell::new(0));
    let cap = |id, ty| Cap::new(&live, id, ty);
    let cases: [(Ty, Option<Ty>); 3] = [
        (None, Some(Some("Endpoint"))),
        (Some("Frame"), Some(None)),
        (None, None),
    ];
    for (f, p) in cases {
        let what = format!("frame {f:?}, peer {p:?}");
        let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
        let mut cl = CapList::new();
        let req = EchoCapsRequest {
            frame: cap(1, f),
            peer: p.map(|t| cap(2, t)),
            any: cap(3, None),
        };
        let n = req.encode_message(&mut buf, &mut cl).unwrap();
        let mut server = EchoServer::default();
        let mut reply = [0u8; silica_idl_rt::MAX_MESSAGE];
        let mut rcl = CapList::new();
        let m = echo::dispatch(&mut server, &buf[..n], cl, &mut reply, &mut rcl);
        assert_eq!(
            reply_status(&reply[..m]),
            Status::MalformedRequest,
            "{what}"
        );
        assert!(rcl.is_empty(), "{what}");
        drop(rcl);
        assert_eq!(
            live.get(),
            0,
            "{what}: the request's capabilities are released"
        );
    }
}

#[test]
fn cap_type_mismatch_is_malformed() {
    let live = Rc::new(Cell::new(0));
    let cap = |id, ty| Cap::new(&live, id, ty);
    let frame = Some("Frame");
    let ep = Some("Endpoint");

    // Requests decoded by a provider: (frame, peer, any) object types.
    let cases: [(Ty, Option<Ty>, Ty, bool); 9] = [
        (frame, Some(ep), Some("Thread"), true),
        (frame, Some(ep), None, true),
        (frame, None, Some("Endpoint"), true),
        // `cap` is never checked: any type, or none told. (A typed field
        // with none told: `a_typed_capability_of_untold_type_is_refused`.)
        (frame, Some(ep), Some("Vcpu"), true),
        // `cap<Frame>` holding another object type.
        (ep, Some(ep), frame, false),
        (Some("Untyped"), None, None, false),
        // `endpoint<P>` holding something that is not an endpoint.
        (frame, Some(frame), None, false),
        (frame, Some(Some("Notification")), None, false),
        // Any name but `Frame`, even a near miss, is another type.
        (Some("frame"), None, None, false),
    ];
    for (i, (f, p, a, ok)) in cases.into_iter().enumerate() {
        let what = format!("case {i}: frame {f:?}, peer {p:?}, any {a:?}");
        let req = |base: u32| EchoCapsRequest {
            frame: cap(base, f),
            peer: p.map(|t| cap(base + 1, t)),
            any: cap(base + 2, a),
        };
        let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
        let mut cl = CapList::new();
        let n = req(10).encode_message(&mut buf, &mut cl).unwrap();
        let got = EchoCapsRequest::decode_message(&buf[..n], cl);
        if ok {
            let got = got.unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!((got.frame.id, got.any.id), (10, 12), "{what}");
            assert_eq!(got.peer.map(|c| c.id), p.map(|_| 11), "{what}");
        } else {
            assert_eq!(got.map(drop), Err(DecodeError::CapType), "{what}");
        }
        assert_eq!(
            live.get(),
            0,
            "{what}: decoding releases what it does not return"
        );

        // The provider answers a mismatch with `malformed_request`.
        let mut server = EchoServer::default();
        let mut cl = CapList::new();
        let n = req(20).encode_message(&mut buf, &mut cl).unwrap();
        let mut reply = [0u8; silica_idl_rt::MAX_MESSAGE];
        let mut rcl = CapList::new();
        let m = echo::dispatch(&mut server, &buf[..n], cl, &mut reply, &mut rcl);
        let want = if ok {
            Status::Ok
        } else {
            Status::MalformedRequest
        };
        assert_eq!(reply_status(&reply[..m]), want, "{what}");
        if !ok {
            assert!(rcl.is_empty(), "{what}");
        }
        drop(rcl);
        assert_eq!(
            live.get(),
            0,
            "{what}: the provider released the request's capabilities"
        );

        // And the caller observes it as `malformed_request`.
        let mut client: echo::Client<Loopback<EchoServer>, Cap> =
            Loopback::new(EchoServer::default()).into();
        let r = client.echo_caps(cap(30, f), p.map(|t| cap(31, t)), cap(32, a));
        if ok {
            assert!(r.is_ok(), "{what}: {r:?}");
        } else {
            assert_eq!(
                r,
                Err(CallError::Transport(TransportError::MalformedRequest)),
                "{what}"
            );
        }
        drop(r);
        assert_eq!(live.get(), 0, "{what}");
    }

    // A reply with a mismatch is `malformed_reply` to the caller: the server
    // returns the endpoint as the frame and the frame as the endpoint.
    let mut client: echo::Client<Loopback<EchoServer>, Cap> = Loopback::new(EchoServer {
        swap: true,
        big: false,
    })
    .into();
    let r = client.echo_caps(cap(1, frame), Some(cap(2, ep)), cap(3, None));
    assert_eq!(r, Err(CallError::Transport(TransportError::MalformedReply)));
    drop(r);
    // Without reported types nothing is known to be a frame or an endpoint:
    // the request is refused before the server sees it (before, untold
    // types were not checked, and the swapped reply decoded).
    let r = client.echo_caps(cap(4, None), Some(cap(5, None)), cap(6, None));
    assert_eq!(
        r,
        Err(CallError::Transport(TransportError::MalformedRequest))
    );
    drop(r);
    assert_eq!(live.get(), 0);

    // Decoding a reply directly: the same check.
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let n = EchoCapsReply {
        frame: cap(7, ep),
        peer: None,
        any: cap(8, frame),
    }
    .encode_message(&mut buf, &mut cl)
    .unwrap();
    assert_eq!(
        EchoCapsReply::decode_message(&buf[..n], cl).map(drop),
        Err(CallError::Transport(TransportError::MalformedReply))
    );
    assert_eq!(live.get(), 0);
}

#[test]
fn caps_move_into_call() {
    let live = Rc::new(Cell::new(0));
    let cap = |id, ty| Cap::new(&live, id, ty);

    // Through the generated client, a transport and a provider and back: the
    // capabilities move, none is duplicated or lost on the way.
    let mut client: echo::Client<Loopback<EchoServer>, Cap> =
        Loopback::new(EchoServer::default()).into();
    let (c0, c1, c2) = (
        cap(0, Some("Frame")),
        cap(1, Some("Endpoint")),
        cap(2, None),
    );
    assert_eq!(live.get(), 3);
    let got = client.echo_caps(c0, Some(c1), c2).unwrap();
    assert_eq!(live.get(), 3, "no capability duplicated or released");
    assert_eq!(
        (got.frame.id, got.peer.as_ref().map(|c| c.id), got.any.id),
        (0, Some(1), 2)
    );
    drop(got);
    assert_eq!(
        client.core.transport.caps_in, 3,
        "the transport got all three"
    );
    assert_eq!(live.get(), 0);

    // Encoding moves the capabilities into the list, in encoding order.
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let n = EchoCapsRequest {
        frame: cap(5, Some("Frame")),
        peer: None,
        any: cap(6, None),
    }
    .encode_message(&mut buf, &mut cl)
    .unwrap();
    assert_eq!(cl.iter().map(|c| c.id).collect::<Vec<_>>(), vec![5, 6]);
    assert_eq!(live.get(), 2, "moved, not copied");
    // Decoding moves them out again; the list is consumed.
    let req = EchoCapsRequest::decode_message(&buf[..n], cl).unwrap();
    assert_eq!((req.frame.id, req.any.id), (5, 6));
    assert_eq!(live.get(), 2);
    drop(req);
    assert_eq!(live.get(), 0);

    // A transport whose call fails owns the capabilities and releases them.
    let mut client: echo::Client<Gone, Cap> = Gone { caps_in: 0 }.into();
    let r = client.echo_caps(cap(7, None), None, cap(8, None));
    assert_eq!(r, Err(CallError::Transport(TransportError::PeerGone)));
    assert_eq!(client.core.transport.caps_in, 2);
    assert_eq!(live.get(), 0, "released by the failed call");

    // A request that is too large is detected before sending; its
    // capability is dropped, and the transport is never called.
    let big = vec![1u8; silica_idl_rt::MAX_MESSAGE];
    let mut client: holder::Client<Loopback<HoldServer>, Cap> = Loopback::new(HoldServer {
        kept: Vec::new(),
        reply_frame: Some(cap(9, Some("Frame"))),
        reply_len: 0,
    })
    .into();
    assert_eq!(live.get(), 1);
    let r = client.put(&big, cap(10, Some("Frame")));
    assert_eq!(
        r.map(drop),
        Err(CallError::Transport(TransportError::TooLarge))
    );
    assert_eq!(client.core.transport.calls, 0);
    assert_eq!(
        live.get(),
        1,
        "the too-large request's capability is released"
    );

    // The provider keeps what it was given; the caller gets the provider's.
    let r = client.put(b"x", cap(11, Some("Frame"))).unwrap();
    assert_eq!(r.frame.id, 9);
    drop(r);
    assert_eq!(
        client
            .core
            .transport
            .server
            .kept
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![11]
    );
    assert_eq!(live.get(), 1, "only the kept frame is alive");
    drop(client);
    assert_eq!(live.get(), 0);
}

/// A request that carries a field after those the provider's version
/// knows is a newer client's, asking for something this provider cannot do:
/// answered `unknown_field` (V9 of spec/IDL.md 3.1) and not handled. Before,
/// the field was skipped and the request done as if it had not been there.
#[test]
fn a_field_unknown_to_the_provider_is_refused_not_ignored() {
    // V9: `echo` reply with status 5 `unknown_field`, empty payload.
    const V9: [u8; 16] = [
        0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    // V1 (`echo` with "hi") with one more field after `data`: a byte 7.
    const V1_AND_MORE: [u8; 27] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00,
        0x00, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x68, 0x69, 0x07,
    ];
    let mut server = EchoServer {
        swap: false,
        big: false,
    };
    let mut reply = [0xeeu8; silica_idl_rt::MAX_MESSAGE];
    let mut rcl: CapList<Cap> = CapList::new();
    let n = echo::dispatch(
        &mut server,
        &V1_AND_MORE,
        CapList::new(),
        &mut reply,
        &mut rcl,
    );
    assert_eq!(&reply[..n], &V9, "dispatch's reply bytes");
    assert!(rcl.is_empty());
    assert_eq!(TransportError::UnknownField.to_string(), "unknown_field");
    // The caller sees it as such.
    assert_eq!(
        silica_idl_rt::decode_reply::<Cap, echo::EchoReply, core::convert::Infallible>(
            &V9,
            CapList::new(),
            1
        )
        .unwrap_err(),
        CallError::Transport(TransportError::UnknownField)
    );
}

#[test]
fn oversized_reply_is_reply_too_large() {
    // Vector: `echo` reply with status 4 `reply_too_large`, empty payload,
    // sender version 1, 16 bytes (spec/IDL.md section 3, reply status).
    const V8: [u8; 16] = [
        0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    // V1 of spec/IDL.md 3.1: `echo` request with "hi".
    const V1: [u8; 26] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00,
        0x00, 0x06, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x68, 0x69,
    ];
    assert_eq!(Status::from_raw(4), Some(Status::ReplyTooLarge));
    assert_eq!(Status::from_raw(5), Some(Status::UnknownField));
    assert_eq!(Status::from_raw(6), None);
    assert_eq!(TransportError::ReplyTooLarge.to_string(), "reply_too_large");

    // The provider's reply does not fit: `dispatch` answers status 4.
    let mut server = EchoServer {
        swap: false,
        big: true,
    };
    let mut reply = [0xeeu8; silica_idl_rt::MAX_MESSAGE];
    let mut rcl: CapList<Cap> = CapList::new();
    let n = echo::dispatch(&mut server, &V1, CapList::new(), &mut reply, &mut rcl);
    assert_eq!(&reply[..n], &V8, "dispatch's reply bytes");
    assert!(rcl.is_empty());

    // Encoding status 4 gives the vector; the vector decodes to the
    // transport error, and its header to status 4.
    let mut buf = [0u8; 64];
    let mut cl: CapList<Cap> = CapList::new();
    let m = silica_idl_rt::encode_status_reply(
        &mut buf,
        &mut cl,
        1,
        echo::VERSION,
        Status::ReplyTooLarge,
    )
    .unwrap();
    assert_eq!(&buf[..m], &V8);
    let h = Header::decode(&V8).unwrap();
    assert_eq!(
        (h.ordinal, h.reply, h.status, h.version, h.payload_len),
        (1, true, Status::ReplyTooLarge, 1, 0)
    );
    assert_eq!(
        EchoReply::decode_message(&V8, CapList::<Cap>::new()),
        Err(CallError::Transport(TransportError::ReplyTooLarge))
    );
    // Status 4 is a reply status only, and carries nothing.
    let mut req = V8;
    req[4] = 0;
    assert_eq!(Header::decode(&req), Err(DecodeError::BadHeader));
    let mut with_payload = V8.to_vec();
    with_payload.extend_from_slice(&[0, 0, 0, 0]);
    with_payload[12] = 4;
    assert_eq!(
        EchoReply::decode_message(&with_payload, CapList::<Cap>::new()),
        Err(CallError::Transport(TransportError::MalformedReply))
    );

    // The caller observes `reply_too_large`.
    let mut client: echo::Client<Loopback<EchoServer>, Cap> = Loopback::new(EchoServer {
        swap: false,
        big: true,
    })
    .into();
    assert_eq!(
        client.echo(b"hi"),
        Err(CallError::Transport(TransportError::ReplyTooLarge))
    );
    // A reply that fits is unaffected.
    let mut client: echo::Client<Loopback<EchoServer>, Cap> =
        Loopback::new(EchoServer::default()).into();
    let fits = vec![7u8; silica_idl_rt::MAX_MESSAGE - 16 - 4 - 4];
    assert_eq!(client.echo(&fits).map(|r| r.data.len()), Ok(fits.len()));

    // An oversized reply that holds a capability: `dispatch` releases it and
    // sends none; the caller observes `reply_too_large`.
    let live = Rc::new(Cell::new(0));
    let mut client: holder::Client<Loopback<HoldServer>, Cap> = Loopback::new(HoldServer {
        kept: Vec::new(),
        reply_frame: Some(Cap::new(&live, 1, Some("Frame"))),
        reply_len: silica_idl_rt::MAX_MESSAGE,
    })
    .into();
    let r = client.put(b"x", Cap::new(&live, 2, Some("Frame")));
    assert_eq!(
        r.map(drop),
        Err(CallError::Transport(TransportError::ReplyTooLarge))
    );
    assert_eq!(
        live.get(),
        1,
        "the reply's frame was released; the kept one lives"
    );
    drop(client);
    assert_eq!(live.get(), 0);

    // The largest reply that fits is sent as is.
    let live = Rc::new(Cell::new(0));
    let fit = silica_idl_rt::MAX_MESSAGE - 16 - 4 - 4 - 1;
    let mut server = HoldServer {
        kept: Vec::new(),
        reply_frame: Some(Cap::new(&live, 3, Some("Frame"))),
        reply_len: fit,
    };
    let mut buf = [0u8; silica_idl_rt::MAX_MESSAGE];
    let mut cl = CapList::new();
    let n = holder::PutRequest {
        data: b"x",
        frame: Cap::new(&live, 4, Some("Frame")),
    }
    .encode_message(&mut buf, &mut cl)
    .unwrap();
    let mut rcl = CapList::new();
    let m = holder::dispatch(&mut server, &buf[..n], cl, &mut reply, &mut rcl);
    assert_eq!(m, silica_idl_rt::MAX_MESSAGE);
    assert_eq!(reply_status(&reply[..m]), Status::Ok);
    assert_eq!(rcl.iter().map(|c| c.id).collect::<Vec<_>>(), vec![3]);
    // One byte more does not fit.
    server.reply_frame = Some(Cap::new(&live, 5, Some("Frame")));
    server.reply_len = fit + 1;
    let mut cl = CapList::new();
    let n = holder::PutRequest {
        data: b"x",
        frame: Cap::new(&live, 6, Some("Frame")),
    }
    .encode_message(&mut buf, &mut cl)
    .unwrap();
    let m = holder::dispatch(&mut server, &buf[..n], cl, &mut reply, &mut rcl);
    assert_eq!(reply_status(&reply[..m]), Status::ReplyTooLarge);
    assert_eq!(m, 16);
    assert!(
        rcl.is_empty(),
        "the previous reply's capabilities were replaced, the new one released"
    );

    // A reply buffer shorter than a header gets no reply at all.
    let mut tiny = [0u8; 8];
    let mut rcl = CapList::new();
    let m = echo::dispatch(
        &mut EchoServer::default(),
        &V1,
        CapList::new(),
        &mut tiny,
        &mut rcl,
    );
    assert_eq!(m, 0);
}

/// Compiling refuses them, before any backend sees the protocol (the rule
/// is the language's: `parse.rs`, `an_array_element_carries_no_capability`).
#[test]
fn capabilities_in_arrays_are_refused_for_every_backend() {
    let src = "protocol x.Y version 1 {\n  struct Held { c: option<cap> }\n  a @1 (s: array<Held>) -> ();\n  b @2 () -> (e: array<endpoint<x.Y>>);\n  c @3 (n: option<array<cap<Frame>>>) -> ();\n  d @4 (ok: array<array<u8>>) -> ();\n}\n";
    let errs = idlc::compile(&[("x/y.idl".into(), src.into())]).unwrap_err();
    let text: Vec<String> = errs.iter().map(ToString::to_string).collect();
    assert_eq!(errs.len(), 3, "{text:?}");
    for (t, line) in text.iter().zip(["x/y.idl:3:", "x/y.idl:4:", "x/y.idl:5:"]) {
        assert!(
            t.contains(line) && t.contains("cannot carry a capability"),
            "{text:?}"
        );
    }
}
