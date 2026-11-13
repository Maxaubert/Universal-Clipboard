//! Pairing flow tests (protocol 4 and section 7).

mod common;

use std::net::IpAddr;

use common::*;
use uc_keys::DeviceKey;
use uc_pair::chain::{Change, Graph, GroupSecret};
use uc_pair::frame::{
    self, AbortReason, Decoder, DeviceRecord, Frame, Hello, State, T_ABORT, T_HELLO, T_REPLY,
    T_STATE,
};
use uc_pair::lan::Iface;
use uc_pair::pake::{Mode, PairSecret, VERSION};
use uc_pair::session::*;

const T0: u64 = 1_000_000;

fn ifaces() -> Vec<Iface> {
    vec![
        Iface {
            index: 3,
            physical: true,
            addr: "192.168.1.10".parse().unwrap(),
            prefix_len: 24,
        },
        Iface {
            index: 3,
            physical: true,
            addr: "fd00:1:2:3::10".parse().unwrap(),
            prefix_len: 64,
        },
        Iface {
            index: 9,
            physical: false,
            addr: "10.8.0.2".parse().unwrap(),
            prefix_len: 24,
        },
    ]
}

fn frame_of(bytes: &[u8]) -> Frame {
    let mut d = Decoder::new();
    d.push(bytes);
    let f = d.next_frame().unwrap().unwrap();
    assert!(d.next_frame().unwrap().is_none());
    f
}

fn abort_reason(step: &InviterStep) -> Option<AbortReason> {
    match step {
        InviterStep::Abort(Some(b)) => {
            let f = frame_of(b);
            assert_eq!(f.ty, T_ABORT);
            AbortReason::from_byte(f.payload[0])
        }
        _ => None,
    }
}

struct World {
    host: Host,
    inv: DeviceKey,
    graph: Graph,
    secret: GroupSecret,
    rng: TestRng,
}

impl World {
    fn new(tag: &str) -> World {
        let inv = DeviceKey::from_seed(&arr32(&format!("{tag}-inviter")));
        let mut rng = TestRng::new(tag);
        let (graph, secret) = Graph::create(&inv, 1, "Desk PC", 1, &mut rng).unwrap();
        World {
            host: Host::new(),
            inv,
            graph,
            secret,
            rng,
        }
    }

    fn open(&mut self, now: u64) -> Opened {
        self.host.open(&self.graph, now, &mut self.rng).unwrap()
    }

    fn transport_for(&self, joiner: &DeviceKey, ip: &str) -> (TransportInfo, TransportInfo) {
        let exp = arr32(&format!("exp-{ip}-{:?}", joiner.public()));
        let to_inviter = TransportInfo {
            peer_key: joiner.public(),
            exporter: exp,
            remote: ip.parse().unwrap(),
            scope: None,
        };
        let to_joiner = TransportInfo {
            peer_key: self.inv.public(),
            exporter: exp,
            remote: "192.168.1.10".parse().unwrap(),
            scope: None,
        };
        (to_inviter, to_joiner)
    }

    fn accept(&self, t: &TransportInfo, now: u64) -> Result<InviterRun, Refuse> {
        self.host.accept(t, &ifaces(), self.inv.public(), now)
    }

    fn ctx(&mut self) -> GroupCtx<'_> {
        GroupCtx {
            graph: &mut self.graph,
            secret: &self.secret,
            key: &self.inv,
            time: 2,
            addr_book: b"addr",
        }
    }
}

fn record() -> DeviceRecord {
    DeviceRecord::new(2, "MacBook").unwrap()
}

/// Runs one whole pairing. Returns the joiner's result and the add change.
fn full_run(
    w: &mut World,
    joiner: &DeviceKey,
    secret: &PairSecret,
    mode: Mode,
    now: u64,
) -> Result<(Box<uc_pair::session::Joined>, Option<Box<Change>>), String> {
    let (ti, tj) = w.transport_for(joiner, "192.168.1.20");
    let mut run = w.accept(&ti, now).map_err(|e| format!("{e:?}"))?;
    let inv_key = w.inv.public();
    let expected = (mode == Mode::Qr).then_some(&inv_key);
    let (mut jr, hello) = JoinerRun::start(
        secret,
        mode,
        joiner.public(),
        &tj,
        expected,
        record(),
        TestRng::new("j"),
    )
    .map_err(|e| format!("{e:?}"))?;
    let mut rng = TestRng::new("i");
    let reply = match run.on_frame(frame_of(&hello), now, &mut rng, None) {
        InviterStep::Send(b) => b,
        s => return Err(format!("hello: {s:?}")),
    };
    let confirm = match jr
        .on_frame(frame_of(&reply))
        .map_err(|e| format!("{e:?}"))?
    {
        JoinerStep::Send(b) => b,
        s => return Err(format!("{s:?}")),
    };
    let (state, added) = match run.on_frame(frame_of(&confirm), now, &mut rng, Some(w.ctx())) {
        InviterStep::SendState { frame, added } => (frame, added),
        s => return Err(format!("confirm: {s:?}")),
    };
    let joined = match jr
        .on_frame(frame_of(&state))
        .map_err(|e| format!("{e:?}"))?
    {
        JoinerStep::Joined(j) => j,
        s => return Err(format!("{s:?}")),
    };
    match run.on_frame(frame_of(&joined.done_frame), now, &mut rng, None) {
        InviterStep::Finished => {}
        s => return Err(format!("done: {s:?}")),
    }
    Ok((joined, added))
}

#[test]
fn happy_path_code() {
    let mut w = World::new("happy-code");
    let o = w.open(T0);
    let code = o.code.clone().unwrap();
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (joined, added) = full_run(&mut w, &j, &code, Mode::Code, T0 + 1000).unwrap();
    assert!(added.is_some());
    assert!(joined.graph.is_member(&j.public()));
    assert!(joined.graph.is_member(&w.inv.public()));
    assert_eq!(joined.graph.group_id(), w.graph.group_id());
    assert_eq!(joined.secret, w.secret);
    assert_eq!(joined.addr_book, b"addr");
    assert_eq!(joined.graph.device_name(&j.public()).unwrap().1, "MacBook");
    let st = w.host.status(T0 + 2000);
    assert!(st.succeeded && !st.open);
    assert!(!w.host.is_open(T0 + 2000));
}

#[test]
fn happy_path_qr() {
    let mut w = World::new("happy-qr");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (joined, _) = full_run(&mut w, &j, &PairSecret::Qr(o.qr_secret), Mode::Qr, T0).unwrap();
    assert!(joined.graph.is_member(&j.public()));
}

#[test]
fn code_is_six_digits() {
    let mut w = World::new("code6");
    for i in 0..50 {
        let o = w.open(T0 + i);
        assert_eq!(o.instance.len(), 16);
        assert!(
            o.instance
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert!(o.code.is_some());
    }
    assert!(PairSecret::code("12345").is_none());
    assert!(PairSecret::code("1234567").is_none());
    assert!(PairSecret::code("12a456").is_none());
    assert!(PairSecret::code("000000").is_some());
}

/// Sends a hello with `secret` and returns the inviter's answer and the joiner run.
fn try_once(
    w: &World,
    run: &mut InviterRun,
    j: &DeviceKey,
    secret: &PairSecret,
    mode: Mode,
    now: u64,
) -> (InviterStep, Option<JoinerRun>) {
    let (_, tj) = w.transport_for(j, "192.168.1.20");
    let ik = w.inv.public();
    let (jr, hello) = JoinerRun::start(
        secret,
        mode,
        j.public(),
        &tj,
        (mode == Mode::Qr).then_some(&ik),
        record(),
        TestRng::new("t"),
    )
    .unwrap();
    let step = run.on_frame(frame_of(&hello), now, &mut TestRng::new("x"), None);
    (step, Some(jr))
}

#[test]
fn wrong_code_joiner_sends_nothing_and_try_counts() {
    let mut w = World::new("wrong");
    let o = w.open(T0);
    let real = o.code.clone().unwrap();
    let wrong = PairSecret::code(if real == PairSecret::code("111111").unwrap() {
        "222222"
    } else {
        "111111"
    })
    .unwrap();
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let mut run = w.accept(&ti, T0).unwrap();
    let (step, jr) = try_once(&w, &mut run, &j, &wrong, Mode::Code, T0);
    let InviterStep::Send(reply) = step else {
        panic!("inviter must reply once")
    };
    // The joiner refuses conf_a and sends nothing at all.
    assert_eq!(
        jr.unwrap().on_frame(frame_of(&reply)).unwrap_err(),
        JoinError::CloseSilently
    );
    drop(run);
    let st = w.host.status(T0);
    assert_eq!(st.tries_code, 1);
    assert_eq!(st.failed_code_tries, 1);
    assert!(!st.code_dead);
}

#[test]
fn fake_inviter_gets_no_byte() {
    // A fake inviter with another password: the joiner's only output is the hello, which does
    // not depend on K.
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let fake = DeviceKey::from_seed(&arr32("fake"));
    let mut w = World::new("fake");
    w.inv = fake;
    let mut fake_graph_world = w;
    let o = fake_graph_world.open(T0);
    let _ = o;
    let typed = PairSecret::code("123456").unwrap();
    let (ti, tj) = fake_graph_world.transport_for(&j, "192.168.1.20");
    let mut run = fake_graph_world.accept(&ti, T0).unwrap();
    let (mut jr, hello) = JoinerRun::start(
        &typed,
        Mode::Code,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("a"),
    )
    .unwrap();
    let _ = run.on_frame(frame_of(&hello), T0, &mut TestRng::new("b"), None);
    // The fake inviter makes a reply with a guess of its own.
    let fake_reply = {
        let mut f2 = World::new("fake2");
        f2.inv = DeviceKey::from_seed(&arr32("fake"));
        let o2 = f2.open(T0);
        let mut r2 = f2.accept(&ti, T0).unwrap();
        let _ = o2;
        match r2.on_frame(frame_of(&hello), T0, &mut TestRng::new("c"), None) {
            InviterStep::Send(b) => b,
            s => panic!("{s:?}"),
        }
    };
    assert_eq!(
        jr.on_frame(frame_of(&fake_reply)).err(),
        Some(JoinError::CloseSilently)
    );
    // After that, any further frame is refused too: there is no path to PairConfirm.
    assert!(jr.on_frame(frame_of(&fake_reply)).is_err());
}

#[test]
fn qr_joiner_sends_nothing_to_wrong_key() {
    let w = World::new("qrkey");
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (_, tj) = w.transport_for(&j, "192.168.1.20");
    let other = DeviceKey::from_seed(&arr32("other")).public();
    let r = JoinerRun::start(
        &PairSecret::Qr([1; 16]),
        Mode::Qr,
        j.public(),
        &tj,
        Some(&other),
        record(),
        TestRng::new("q"),
    );
    assert_eq!(r.err(), Some(JoinError::WrongInviterKey));
    // QR mode without an expected key is refused too.
    let r = JoinerRun::start(
        &PairSecret::Qr([1; 16]),
        Mode::Qr,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("q"),
    );
    assert_eq!(r.err(), Some(JoinError::WrongInviterKey));
}

fn kill_code(w: &mut World, now: u64) {
    let wrong = PairSecret::code("999999").unwrap();
    let j = DeviceKey::from_seed(&arr32("guesser"));
    for _ in 0..5 {
        let (ti, _) = w.transport_for(&j, "192.168.1.30");
        let mut run = w.accept(&ti, now).unwrap();
        let (step, _) = try_once(w, &mut run, &j, &wrong, Mode::Code, now);
        // The rare case that 999999 is the real code does not matter: the joiner side is not run,
        // so the try still fails.
        assert!(matches!(step, InviterStep::Send(_)));
        run.on_close();
    }
}

#[test]
fn five_tries_then_code_dead_qr_still_valid() {
    let mut w = World::new("dead");
    let o = w.open(T0);
    kill_code(&mut w, T0);
    let st = w.host.status(T0);
    assert!(st.code_dead);
    assert_eq!(st.failed_code_tries, 5);
    // A sixth code try is refused before any reply.
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let mut run = w.accept(&ti, T0).unwrap();
    let (step, _) = try_once(&w, &mut run, &j, o.code.as_ref().unwrap(), Mode::Code, T0);
    assert_eq!(abort_reason(&step), Some(AbortReason::NotOpen));
    drop(run);
    // The QR code still works.
    let (joined, _) = full_run(&mut w, &j, &PairSecret::Qr(o.qr_secret), Mode::Qr, T0).unwrap();
    assert!(joined.graph.is_member(&j.public()));
}

#[test]
fn four_connections_at_once_never_more_than_five_replies() {
    let mut w = World::new("conc");
    let _o = w.open(T0);
    let host = w.host.clone();
    let inv = w.inv.public();
    let mut handles = Vec::new();
    for t in 0..8u64 {
        let host = host.clone();
        handles.push(std::thread::spawn(move || {
            let mut replies = 0;
            let mut busy = 0;
            for i in 0..10u64 {
                let j = DeviceKey::from_seed(&arr32(&format!("g{t}-{i}")));
                let ti = TransportInfo {
                    peer_key: j.public(),
                    exporter: [7; 32],
                    remote: "192.168.1.40".parse().unwrap(),
                    scope: None,
                };
                let mut run = match host.accept(&ti, &ifaces(), inv, T0) {
                    Ok(r) => r,
                    Err(Refuse::Busy) => {
                        busy += 1;
                        continue;
                    }
                    Err(e) => panic!("{e:?}"),
                };
                let (_, hello) = JoinerRun::start(
                    &PairSecret::code("999999").unwrap(),
                    Mode::Code,
                    j.public(),
                    &TransportInfo {
                        peer_key: inv,
                        ..ti.clone()
                    },
                    None,
                    record(),
                    TestRng::new("h"),
                )
                .unwrap();
                if let InviterStep::Send(_) =
                    run.on_frame(frame_of(&hello), T0, &mut TestRng::new("r"), None)
                {
                    replies += 1;
                }
            }
            (replies, busy)
        }));
    }
    let total: u32 = handles.into_iter().map(|h| h.join().unwrap().0).sum();
    assert_eq!(total, 5, "exactly 5 replies for one code");
    assert_eq!(w.host.status(T0).active_connections, 0);
}

#[test]
fn busy_is_not_a_try() {
    let mut w = World::new("busy");
    w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let runs: Vec<_> = (0..4).map(|_| w.accept(&ti, T0).unwrap()).collect();
    assert_eq!(w.accept(&ti, T0).err(), Some(Refuse::Busy));
    assert_eq!(
        frame_of(&frame::abort(AbortReason::Busy)).payload,
        vec![AbortReason::Busy as u8]
    );
    drop(runs);
    let st = w.host.status(T0);
    assert_eq!(st.tries_code, 0);
    assert_eq!(st.failed_code_tries, 0);
    assert!(w.accept(&ti, T0).is_ok());
}

#[test]
fn no_hello_is_not_a_try() {
    let mut w = World::new("nohello");
    w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let mut run = w.accept(&ti, T0).unwrap();
    run.on_close(); // timeout before PairHello
    drop(run);
    assert_eq!(w.host.status(T0).failed_code_tries, 0);
}

#[test]
fn dropped_after_reply_stays_counted() {
    let mut w = World::new("drop");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let mut run = w.accept(&ti, T0).unwrap();
    // Even the right code: the connection drops after PairReply.
    let (step, _) = try_once(&w, &mut run, &j, o.code.as_ref().unwrap(), Mode::Code, T0);
    assert!(matches!(step, InviterStep::Send(_)));
    drop(run);
    let st = w.host.status(T0);
    assert_eq!((st.tries_code, st.failed_code_tries), (1, 1));
}

#[test]
fn lockout_after_five_dead_codes() {
    let mut w = World::new("lockout");
    let mut now = T0;
    for _ in 0..4 {
        w.open(now);
        kill_code(&mut w, now);
        now += 1000;
    }
    assert_eq!(w.host.status(now).dead_codes_in_a_row, 4);
    assert!(!w.host.status(now).code_off);
    w.open(now);
    kill_code(&mut w, now);
    let st = w.host.status(now);
    assert!(st.code_off);
    // A new pairing shows only the QR code.
    let o = w.open(now + 1);
    assert!(o.code.is_none());
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let mut run = w.accept(&ti, now + 1).unwrap();
    let (step, _) = try_once(
        &w,
        &mut run,
        &j,
        &PairSecret::code("123456").unwrap(),
        Mode::Code,
        now + 1,
    );
    assert_eq!(abort_reason(&step), Some(AbortReason::NotOpen));
    drop(run);
    // QR still works during the lockout.
    full_run(&mut w, &j, &PairSecret::Qr(o.qr_secret), Mode::Qr, now + 2).unwrap();
    // After one hour the code is back.
    let later = now + LOCKOUT_MS + 1;
    assert!(!w.host.status(later).code_off);
    assert!(w.open(later).code.is_some());
}

#[test]
fn success_resets_dead_code_count() {
    let mut w = World::new("reset");
    let mut now = T0;
    for _ in 0..4 {
        w.open(now);
        kill_code(&mut w, now);
        now += 1000;
    }
    let o = w.open(now);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, now).unwrap();
    assert_eq!(w.host.status(now).dead_codes_in_a_row, 0);
    w.open(now);
    kill_code(&mut w, now);
    assert!(!w.host.status(now).code_off);
}

#[test]
fn expiry_at_ten_minutes() {
    let mut w = World::new("expiry");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    // A run accepted just before expiry gets NotOpen on its hello after expiry.
    let mut run = w.accept(&ti, T0 + PAIRING_LIFETIME_MS - 1).unwrap();
    let (step, _) = try_once(
        &w,
        &mut run,
        &j,
        o.code.as_ref().unwrap(),
        Mode::Code,
        T0 + PAIRING_LIFETIME_MS,
    );
    assert_eq!(abort_reason(&step), Some(AbortReason::NotOpen));
    drop(run);
    assert!(!w.host.is_open(T0 + PAIRING_LIFETIME_MS));
    assert_eq!(
        w.accept(&ti, T0 + PAIRING_LIFETIME_MS).err(),
        Some(Refuse::NotOpen)
    );
}

#[test]
fn not_local_address_refused() {
    let mut w = World::new("lan");
    w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    for ip in [
        "8.8.8.8",
        "192.168.2.20",
        "10.8.0.5",
        "100.64.1.1",
        "2001:db8::1",
        "fd00:9::1",
    ] {
        let (ti, _) = w.transport_for(&j, ip);
        assert_eq!(w.accept(&ti, T0).err(), Some(Refuse::NotLocal), "{ip}");
    }
    for ip in ["192.168.1.20", "fd00:1:2:3::99", "::ffff:192.168.1.77"] {
        let (ti, _) = w.transport_for(&j, ip);
        assert!(w.accept(&ti, T0).is_ok(), "{ip}");
    }
    // Link-local IPv6 only with the scope of a physical interface.
    let (mut ti, _) = w.transport_for(&j, "fe80::1");
    assert_eq!(w.accept(&ti, T0).err(), Some(Refuse::NotLocal));
    ti.scope = Some(9);
    assert_eq!(w.accept(&ti, T0).err(), Some(Refuse::NotLocal));
    ti.scope = Some(3);
    assert!(w.accept(&ti, T0).is_ok());
}

#[test]
fn no_alpn_when_no_pairing_open() {
    let mut w = World::new("closed");
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    assert!(!w.host.is_open(T0));
    assert_eq!(w.accept(&ti, T0).err(), Some(Refuse::NotOpen));
    w.open(T0);
    w.host.cancel();
    assert!(!w.host.is_open(T0));
    assert_eq!(w.accept(&ti, T0).err(), Some(Refuse::NotOpen));
}

#[test]
fn pairing_existing_member_signs_nothing() {
    let mut w = World::new("again");
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let o = w.open(T0);
    let (_, added) = full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, T0).unwrap();
    assert!(added.is_some());
    let n = w.graph.len();
    let o = w.open(T0 + 10);
    let (joined, added) =
        full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, T0 + 10).unwrap();
    assert!(added.is_none());
    assert_eq!(w.graph.len(), n);
    assert!(joined.graph.is_member(&j.public()));
}

#[test]
fn removed_key_and_full_group_refused() {
    let mut w = World::new("refuse");
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let o = w.open(T0);
    full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, T0).unwrap();
    let inv = DeviceKey::from_seed(&w.inv.signing_key().to_bytes());
    w.graph
        .sign_remove(&inv, j.public(), 3, &mut TestRng::new("rm"))
        .unwrap();
    let o = w.open(T0 + 1);
    let err = full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, T0 + 1).unwrap_err();
    assert!(err.ends_with("[127, 0, 0, 0, 1, 7]))"), "{err}");

    // Fill the group to 8 members: then the host refuses to open.
    let mut w = World::new("full");
    let inv = DeviceKey::from_seed(&w.inv.signing_key().to_bytes());
    for i in 0..6 {
        let k = DeviceKey::from_seed(&arr32(&format!("m{i}"))).public();
        w.graph.sign_add(&inv, k, 1, "M", 1).unwrap();
    }
    let o = w.open(T0);
    // 7 members now; an 8th joins, a 9th would be refused.
    let j8 = DeviceKey::from_seed(&arr32("j8"));
    full_run(&mut w, &j8, o.code.as_ref().unwrap(), Mode::Code, T0).unwrap();
    assert_eq!(w.graph.members().len(), 8);
    assert_eq!(
        w.host.open(&w.graph, T0, &mut TestRng::new("x")).err(),
        Some(AbortReason::GroupFull)
    );
}

#[test]
fn group_full_during_run_aborts_6() {
    let mut w = World::new("full6");
    let inv = DeviceKey::from_seed(&w.inv.signing_key().to_bytes());
    for i in 0..6 {
        let k = DeviceKey::from_seed(&arr32(&format!("m{i}"))).public();
        w.graph.sign_add(&inv, k, 1, "M", 1).unwrap();
    }
    let o = w.open(T0);
    // While the pairing is open, another device is added: the group is full.
    let k = DeviceKey::from_seed(&arr32("late")).public();
    w.graph.sign_add(&inv, k, 1, "M", 1).unwrap();
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let err = full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, T0).unwrap_err();
    assert!(err.ends_with("[127, 0, 0, 0, 1, 6]))"), "{err}");
}

#[test]
fn success_closes_other_runs() {
    let mut w = World::new("other");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let j2 = DeviceKey::from_seed(&arr32("joiner2"));
    let (ti2, tj2) = w.transport_for(&j2, "192.168.1.21");
    let mut run2 = w.accept(&ti2, T0).unwrap();
    let (mut jr2, hello2) = JoinerRun::start(
        o.code.as_ref().unwrap(),
        Mode::Code,
        j2.public(),
        &tj2,
        None,
        record(),
        TestRng::new("j2"),
    )
    .unwrap();
    let reply2 = match run2.on_frame(frame_of(&hello2), T0, &mut TestRng::new("i2"), None) {
        InviterStep::Send(b) => b,
        s => panic!("{s:?}"),
    };
    let JoinerStep::Send(confirm2) = jr2.on_frame(frame_of(&reply2)).unwrap() else {
        panic!()
    };
    full_run(&mut w, &j, o.code.as_ref().unwrap(), Mode::Code, T0).unwrap();
    let step = run2.on_frame(
        frame_of(&confirm2),
        T0,
        &mut TestRng::new("i2"),
        Some(w.ctx()),
    );
    assert_eq!(abort_reason(&step), Some(AbortReason::NotOpen));
    assert!(!w.graph.is_member(&j2.public()));
}

#[test]
fn bad_hello_messages() {
    let mut w = World::new("bad");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, tj) = w.transport_for(&j, "192.168.1.20");
    let (_, hello) = JoinerRun::start(
        o.code.as_ref().unwrap(),
        Mode::Code,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("j"),
    )
    .unwrap();
    let h = Hello::parse(&frame_of(&hello).payload).unwrap();
    let send = |w: &World, payload: Vec<u8>, ty: u8| {
        let mut run = w.accept(&ti, T0).unwrap();
        let s = run.on_frame(Frame { ty, payload }, T0, &mut TestRng::new("r"), None);
        abort_reason(&s)
    };
    // Version 2.
    let mut p = frame_of(&hello).payload;
    p[0] = 2;
    assert_eq!(send(&w, p, T_HELLO), Some(AbortReason::Version));
    // Unknown mode.
    let mut p = frame_of(&hello).payload;
    p[1] = 3;
    assert_eq!(send(&w, p, T_HELLO), Some(AbortReason::BadMessage));
    // Wrong size.
    assert_eq!(
        send(&w, vec![1, 1, 0x42], T_HELLO),
        Some(AbortReason::BadMessage)
    );
    // Out of order: a reply to the inviter.
    assert_eq!(
        send(&w, vec![0; 65], T_REPLY),
        Some(AbortReason::BadMessage)
    );
    assert_eq!(w.host.status(T0).tries_code, 0);
    // Low-order msg_b and wrong side byte: counted tries, then PairAbort(4).
    let mut low = Hello {
        msg_b: [0; 33],
        ..h.clone()
    };
    low.msg_b[0] = 0x42;
    low.msg_b[1] = 1;
    assert_eq!(
        send(&w, frame_of(&low.encode()).payload, T_HELLO),
        Some(AbortReason::BadMessage)
    );
    let mut side = h.clone();
    side.msg_b[0] = 0x41;
    assert_eq!(
        send(&w, frame_of(&side.encode()).payload, T_HELLO),
        Some(AbortReason::BadMessage)
    );
    let st = w.host.status(T0);
    assert_eq!((st.tries_code, st.failed_code_tries), (2, 2));
    assert_eq!(h.version, VERSION);
}

#[test]
fn mitm_is_detected() {
    // Each side sees another key and exporter: the joiner refuses conf_a.
    let mut w = World::new("mitm");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let m = DeviceKey::from_seed(&arr32("mitm"));
    let ti = TransportInfo {
        peer_key: m.public(),
        exporter: [1; 32],
        remote: "192.168.1.20".parse().unwrap(),
        scope: None,
    };
    let tj = TransportInfo {
        peer_key: m.public(),
        exporter: [2; 32],
        remote: "192.168.1.10".parse().unwrap(),
        scope: None,
    };
    let mut run = w.accept(&ti, T0).unwrap();
    let (mut jr, hello) = JoinerRun::start(
        o.code.as_ref().unwrap(),
        Mode::Code,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("j"),
    )
    .unwrap();
    let InviterStep::Send(reply) = run.on_frame(frame_of(&hello), T0, &mut TestRng::new("i"), None)
    else {
        panic!()
    };
    assert_eq!(
        jr.on_frame(frame_of(&reply)).unwrap_err(),
        JoinError::CloseSilently
    );
    // Same keys but different exporters (two TLS sessions).
    let (ti, mut tj) = w.transport_for(&j, "192.168.1.20");
    tj.exporter = [9; 32];
    let mut run = w.accept(&ti, T0).unwrap();
    let (mut jr, hello) = JoinerRun::start(
        o.code.as_ref().unwrap(),
        Mode::Code,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("j"),
    )
    .unwrap();
    let InviterStep::Send(reply) = run.on_frame(frame_of(&hello), T0, &mut TestRng::new("i"), None)
    else {
        panic!()
    };
    assert_eq!(
        jr.on_frame(frame_of(&reply)).unwrap_err(),
        JoinError::CloseSilently
    );
}

#[test]
fn wrong_conf_b_aborts_5_and_counts() {
    let mut w = World::new("confb");
    let o = w.open(T0);
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (ti, _) = w.transport_for(&j, "192.168.1.20");
    let mut run = w.accept(&ti, T0).unwrap();
    let (step, _) = try_once(&w, &mut run, &j, o.code.as_ref().unwrap(), Mode::Code, T0);
    assert!(matches!(step, InviterStep::Send(_)));
    let bad = frame::Confirm {
        conf_b: [0; 32],
        record: record(),
    }
    .encode();
    let s = run.on_frame(frame_of(&bad), T0, &mut TestRng::new("i"), Some(w.ctx()));
    assert_eq!(abort_reason(&s), Some(AbortReason::ConfirmFailed));
    drop(run);
    assert_eq!(w.host.status(T0).failed_code_tries, 1);
}

/// Builds a joiner run that has passed conf_a, plus the honest PairState frame.
fn joiner_at_state(w: &mut World, j: &DeviceKey) -> (JoinerRun, Vec<u8>) {
    let o = w.open(T0);
    let (ti, tj) = w.transport_for(j, "192.168.1.20");
    let mut run = w.accept(&ti, T0).unwrap();
    let (mut jr, hello) = JoinerRun::start(
        o.code.as_ref().unwrap(),
        Mode::Code,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("j"),
    )
    .unwrap();
    let InviterStep::Send(reply) = run.on_frame(frame_of(&hello), T0, &mut TestRng::new("i"), None)
    else {
        panic!()
    };
    let JoinerStep::Send(confirm) = jr.on_frame(frame_of(&reply)).unwrap() else {
        panic!()
    };
    let InviterStep::SendState { frame, .. } = run.on_frame(
        frame_of(&confirm),
        T0,
        &mut TestRng::new("i"),
        Some(w.ctx()),
    ) else {
        panic!()
    };
    (jr, frame)
}

#[test]
fn joiner_refuses_bad_state() {
    let j = DeviceKey::from_seed(&arr32("joiner"));
    // Secret does not match.
    let mut w = World::new("st1");
    let (mut jr, f) = joiner_at_state(&mut w, &j);
    let mut st = State::parse(&frame_of(&f).payload).unwrap();
    st.group_secret[0] ^= 1;
    let r = jr.on_frame(frame_of(&st.encode().unwrap()));
    assert_eq!(r.err(), Some(JoinError::BadState(StateError::Secret)));

    // The joiner is not a member (the add is left out).
    let mut w = World::new("st2");
    let (mut jr, f) = joiner_at_state(&mut w, &j);
    let mut st = State::parse(&frame_of(&f).payload).unwrap();
    st.changes.pop();
    let r = jr.on_frame(frame_of(&st.encode().unwrap()));
    assert_eq!(
        r.err(),
        Some(JoinError::BadState(StateError::SelfNotMember))
    );

    // A change is broken.
    let mut w = World::new("st3");
    let (mut jr, f) = joiner_at_state(&mut w, &j);
    let mut st = State::parse(&frame_of(&f).payload).unwrap();
    let last = st.changes.last_mut().unwrap();
    let n = last.len();
    last[n - 1] ^= 1;
    let r = jr.on_frame(frame_of(&st.encode().unwrap()));
    assert_eq!(r.err(), Some(JoinError::BadState(StateError::Change)));

    // A second genesis.
    let mut w = World::new("st4");
    let (mut jr, f) = joiner_at_state(&mut w, &j);
    let mut st = State::parse(&frame_of(&f).payload).unwrap();
    let other = World::new("st4-other");
    st.changes
        .push(other.graph.canonical().next().unwrap().to_bytes());
    let r = jr.on_frame(frame_of(&st.encode().unwrap()));
    assert_eq!(r.err(), Some(JoinError::BadState(StateError::Change)));

    // The inviter's TLS key is not a member: a state from another group.
    let mut w = World::new("st5");
    let (mut jr, _) = joiner_at_state(&mut w, &j);
    let mut other = World::new("st5-other");
    let okey = DeviceKey::from_seed(&other.inv.signing_key().to_bytes());
    other
        .graph
        .sign_add(&okey, j.public(), 2, "MacBook", 1)
        .unwrap();
    let st = State {
        epoch: 0,
        group_secret: other.secret.secret,
        changes: other.graph.canonical().map(Change::to_bytes).collect(),
        addr_book: vec![],
    };
    let r = jr.on_frame(frame_of(&st.encode().unwrap()));
    assert_eq!(
        r.err(),
        Some(JoinError::BadState(StateError::InviterNotMember))
    );

    // Not a state at all.
    let mut w = World::new("st6");
    let (mut jr, _) = joiner_at_state(&mut w, &j);
    let r = jr.on_frame(Frame {
        ty: T_STATE,
        payload: vec![1, 2, 3],
    });
    assert_eq!(r.err(), Some(JoinError::BadState(StateError::Parse)));
}

#[test]
fn joiner_reports_abort() {
    let w = World::new("ab");
    let j = DeviceKey::from_seed(&arr32("joiner"));
    let (_, tj) = w.transport_for(&j, "192.168.1.20");
    let (mut jr, _) = JoinerRun::start(
        &PairSecret::code("123456").unwrap(),
        Mode::Code,
        j.public(),
        &tj,
        None,
        record(),
        TestRng::new("j"),
    )
    .unwrap();
    let r = jr.on_frame(frame_of(&frame::abort(AbortReason::NotOpen)));
    assert_eq!(r.err(), Some(JoinError::Aborted(AbortReason::NotOpen)));
}

#[test]
fn decoder_limits() {
    let mut d = Decoder::new();
    d.push(&[0x09, 0, 0, 0, 0]);
    assert!(d.next_frame().is_err(), "unknown type");
    let mut d = Decoder::new();
    d.push(&[T_HELLO, 0, 0, 1, 1]);
    assert!(d.next_frame().is_err(), "257 bytes is too long");
    let mut d = Decoder::new();
    d.push(&[T_STATE, 0, 0x10, 0, 1]);
    assert!(d.next_frame().is_err(), "state over 1 MiB");
    let mut d = Decoder::new();
    d.push(&[T_STATE, 0, 0x10, 0, 0]);
    assert!(
        d.next_frame().unwrap().is_none(),
        "1 MiB is allowed, waits for bytes"
    );
    // Split delivery.
    let f = frame::abort(AbortReason::Other);
    let mut d = Decoder::new();
    for b in &f {
        assert!(d.next_frame().unwrap().is_none());
        d.push(&[*b]);
    }
    assert_eq!(d.next_frame().unwrap().unwrap().payload, vec![7]);
}

#[test]
fn device_record_rules() {
    assert!(DeviceRecord::new(7, "Phone").is_some());
    assert!(DeviceRecord::new(8, "Phone").is_none());
    assert!(DeviceRecord::new(1, "").is_none());
    assert!(DeviceRecord::new(1, &"x".repeat(65)).is_none());
    assert!(DeviceRecord::new(1, &"x".repeat(64)).is_some());
    assert!(DeviceRecord::new(1, "a\nb").is_none());
    assert!(DeviceRecord::new(1, "Pål's PC").is_some());
}

#[test]
fn secrets_not_in_debug() {
    let s = PairSecret::code("418207").unwrap();
    assert!(!format!("{s:?}").contains("418207"));
    let g = GroupSecret {
        epoch: 3,
        secret: [0xab; 32],
    };
    assert!(!format!("{g:?}").contains("abab"));
    let _ = IpAddr::from([0, 0, 0, 0]);
}
