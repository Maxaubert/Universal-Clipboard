//! Sync scenarios on the fake network: echo layers, caps, clocks, the 1-hour rule, roles.

use uc_clip::{FakeClipboard, Format};
use uc_core::caps;
use uc_core::msg::{self, Message};
use uc_core::sync::{Event, FetchOutcome, SkipReason};
use uc_core::{Clipboard, Role};
use uc_testkit::{FakeNet, Manager, device_id, images};

const HOUR: u64 = 60 * 60 * 1000;

fn has(net: &FakeNet, i: usize, pred: impl Fn(&Event) -> bool) -> bool {
    net.devices[i].events.iter().any(pred)
}

#[test]
fn copy_reaches_every_device() {
    let mut net = FakeNet::full_mesh(3);
    net.copy_text(0, "hello");
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("hello"));
    assert_eq!(net.text(2).as_deref(), Some("hello"));
    assert!(net.converged(), "{}", net.describe());
    assert_eq!(net.sends(), 1);
}

#[test]
fn layer1_marker_is_never_sent() {
    let mut net = FakeNet::full_mesh(2);
    // Some other process puts a clip that carries our marker, with a new change counter.
    net.devices[0]
        .clip
        .set(vec![(Format::TextPlain, b"x".to_vec())], true, false, false);
    net.local_change(0);
    assert!(has(&net, 0, |e| *e == Event::Skipped(SkipReason::Marker)));
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn layer2_own_write_is_skipped() {
    let mut net = FakeNet::full_mesh(2);
    net.copy_text(0, "a");
    net.run();
    // The OS reports our own write as a change.
    net.local_change(1);
    assert!(has(&net, 1, |e| *e == Event::Skipped(SkipReason::OwnWrite)));
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn layer3_received_wire_id_is_never_sent_back() {
    let mut net = FakeNet::full_mesh(2);
    net.copy_text(0, "same bytes");
    net.run();
    // Another app copies the identical bytes without our marker: same wire ID.
    net.copy_text(1, "same bytes");
    assert!(has(&net, 1, |e| *e == Event::Skipped(SkipReason::SeenId)));
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn layer4_rewritten_content_is_never_sent_back() {
    let mut net = FakeNet::full_mesh(2);
    net.copy_text(0, "line one\nline two");
    net.run();
    // A clipboard manager rewrites it: CRLF, NUL, extra HTML, no marker. New wire ID.
    let parts = vec![
        (Format::TextPlain, b"line one\r\nline two\0".to_vec()),
        (Format::TextHtml, b"<p>line one</p>".to_vec()),
    ];
    net.copy(1, parts);
    assert!(has(&net, 1, |e| *e == Event::Skipped(SkipReason::SeenContent)));
    assert_eq!(net.in_flight(), 0);
}

/// Plan 6.1 and the M1 acceptance: two devices with clipboard managers that strip the marker
/// and re-encode every clip must not ping-pong.
#[test]
fn clipboard_manager_ping_pong() {
    let mut net = FakeNet::new();
    let a = net.add(Role::Full, FakeClipboard::windows());
    let b = net.add(Role::Full, FakeClipboard::new());
    net.devices[a].manager = Some(Manager::TextRewriter);
    net.devices[b].manager = Some(Manager::TextRewriter);
    net.connect_all();
    net.run();

    for round in 0..10 {
        let (from, to) = if round % 2 == 0 { (a, b) } else { (b, a) };
        let text = format!("clip {round}\nsecond line");
        net.copy_text(from, &text);
        // One push. Without layer 4 this would never go quiet.
        net.run_bounded(4);
        assert_eq!(net.text(to).as_deref(), Some(text.as_str()));
        assert!(net.converged(), "{}", net.describe());
    }
    assert_eq!(net.sends(), 10, "exactly one send per user copy");
}

#[test]
fn clipboard_manager_ping_pong_with_images() {
    let mut net = FakeNet::full_mesh(3);
    for i in 0..3 {
        net.devices[i].manager = Some(Manager::PngReencoder);
    }
    for round in 0..6u8 {
        let img = images::image(16, 9, round);
        net.copy(usize::from(round % 3), vec![(Format::ImagePng, img)]);
        net.run_bounded(6);
        assert!(net.converged(), "{}", net.describe());
    }
    assert_eq!(net.sends(), 6);
}

#[test]
fn private_and_file_copies_are_skipped() {
    let mut net = FakeNet::full_mesh(2);
    net.devices[0]
        .clip
        .copy_private(vec![(Format::TextPlain, b"hunter2".to_vec())]);
    net.local_change(0);
    net.devices[0].clip.copy_files("report.pdf");
    net.local_change(0);
    assert!(has(&net, 0, |e| *e == Event::Skipped(SkipReason::Private)));
    assert!(has(&net, 0, |e| *e == Event::Skipped(SkipReason::Files)));
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn read_failure_and_bad_text_are_skipped() {
    let mut net = FakeNet::full_mesh(2);
    net.devices[0].clip.fail_reads(1);
    net.copy_text(0, "x");
    assert!(has(&net, 0, |e| *e == Event::Skipped(SkipReason::ReadFailed)));
    net.copy(0, vec![(Format::TextPlain, vec![0xff, 0xfe])]);
    assert!(has(&net, 0, |e| matches!(
        e,
        Event::Skipped(SkipReason::Invalid(_))
    )));
    net.copy(0, vec![]);
    assert!(has(&net, 0, |e| *e == Event::Skipped(SkipReason::Empty)));
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn pause_stops_sending_and_applying() {
    let mut net = FakeNet::full_mesh(2);
    net.devices[1].engine.set_paused(true);
    net.copy_text(1, "not sent");
    assert!(has(&net, 1, |e| *e == Event::Skipped(SkipReason::Paused)));
    net.copy_text(0, "held only");
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("not sent"));
    assert!(has(&net, 1, |e| matches!(e, Event::HeldOnly { .. })));
    // An explicit fetch while paused still applies (it is the user's own action).
    net.devices[1].engine.set_paused(false);
    assert!(matches!(net.fetch(1), FetchOutcome::Applied(_)));
    assert_eq!(net.text(1).as_deref(), Some("held only"));
}

#[test]
fn sync_images_off_drops_the_image_part() {
    let mut net = FakeNet::full_mesh(2);
    net.devices[0].engine.set_sync_images(false);
    net.copy(
        0,
        vec![
            (Format::TextPlain, b"caption".to_vec()),
            (Format::ImagePng, images::image(4, 4, 1)),
        ],
    );
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("caption"));
    assert!(net.devices[1].clip.part(Format::ImagePng).is_none());
}

#[test]
fn size_caps() {
    let mut net = FakeNet::full_mesh(2);
    // Text over 1 MiB: not synced.
    net.copy_text(0, &"a".repeat(caps::TEXT_MAX + 1));
    assert!(has(&net, 0, |e| *e == Event::Skipped(SkipReason::TooLarge)));
    assert_eq!(net.in_flight(), 0);
    // HTML over 2 MiB: dropped, text sent.
    net.copy(
        0,
        vec![
            (Format::TextPlain, b"short".to_vec()),
            (Format::TextHtml, vec![b'h'; caps::HTML_MAX + 1]),
        ],
    );
    assert!(has(&net, 0, |e| matches!(
        e,
        Event::PartDropped(d) if d.format == Format::TextHtml
    )));
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("short"));
    assert!(net.devices[1].clip.part(Format::TextHtml).is_none());
}

#[test]
fn oversized_or_hostile_image_is_refused() {
    let mut net = FakeNet::full_mesh(2);
    // A PNG header claiming 20000 x 20000 pixels.
    let mut png = images::image(2, 2, 0);
    png[16..20].copy_from_slice(&20_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&20_000u32.to_be_bytes());
    net.copy(0, vec![(Format::ImagePng, png)]);
    assert!(has(&net, 0, |e| matches!(
        e,
        Event::Skipped(SkipReason::Invalid(_))
    )));
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn line_endings_between_windows_and_mac() {
    let mut net = FakeNet::new();
    let win = net.add(Role::Full, FakeClipboard::windows());
    let mac = net.add(Role::Full, FakeClipboard::new());
    net.connect_all();
    net.run();

    net.copy_text(win, "a\r\nb\r\n");
    net.run();
    assert_eq!(
        net.devices[mac].clip.part(Format::TextPlain),
        Some(&b"a\nb\n"[..])
    );

    net.advance(10);
    net.copy_text(mac, "x\ny");
    net.run();
    assert_eq!(
        net.devices[win].clip.part(Format::TextPlain),
        Some(&b"x\r\ny"[..])
    );
}

#[test]
fn big_clip_is_announced_then_wanted() {
    let mut net = FakeNet::full_mesh(3);
    let img = images::big_image(300, 300, 7);
    assert!(img.len() > caps::PUSH_MAX);
    net.copy(1, vec![(Format::ImagePng, img.clone())]);
    // The first frames are Latest, not Clip.
    net.step(0);
    net.run();
    assert!(net.converged(), "{}", net.describe());
    assert_eq!(net.devices[0].clip.part(Format::ImagePng), Some(&img[..]));
}

#[test]
fn concurrent_copies_in_the_same_millisecond_converge() {
    let mut net = FakeNet::full_mesh(3);
    net.copy_text(0, "from zero");
    net.copy_text(2, "from two");
    net.copy_text(1, "from one");
    net.run();
    assert!(net.converged(), "{}", net.describe());
    // Same wall time, counter 0 each: the highest device ID wins.
    assert_eq!(net.text(0).as_deref(), Some("from two"));
}

#[test]
fn clock_far_ahead_is_refused_and_reported() {
    let mut net = FakeNet::new();
    let a = net.add(Role::Full, FakeClipboard::new());
    let b = net.add(Role::Full, FakeClipboard::new());
    net.skew_ms[b] = 5 * 60 * 1000; // 5 minutes ahead
    net.connect_all();
    net.run();
    assert!(has(&net, b, |e| *e
        == Event::OwnClockAhead { peer: device_id(a) }));
    assert!(!has(&net, a, |e| matches!(e, Event::OwnClockAhead { .. })));

    net.copy_text(b, "from the future");
    net.run();
    assert!(has(&net, a, |e| *e
        == Event::RefusedFutureStamp { peer: device_id(b) }));
    assert_eq!(net.text(a), None);

    // After B's clock is fixed and time passes, A's next copy wins again everywhere.
    net.skew_ms[b] = 0;
    net.advance(6 * 60 * 1000);
    net.copy_text(a, "after the fix");
    net.run();
    assert_eq!(net.text(b).as_deref(), Some("after the fix"));
}

#[test]
fn clock_behind_still_wins_with_a_later_copy() {
    let mut net = FakeNet::new();
    let a = net.add(Role::Full, FakeClipboard::new());
    let b = net.add(Role::Full, FakeClipboard::new());
    net.skew_ms[b] = -90 * 1000; // 90 s behind
    net.connect_all();
    net.run();
    net.copy_text(a, "first");
    net.run();
    net.advance(5);
    net.copy_text(b, "second, from the slow clock");
    net.run();
    assert!(net.converged(), "{}", net.describe());
    assert_eq!(net.text(a).as_deref(), Some("second, from the slow clock"));
}

#[test]
fn one_hour_rule_on_reconnect() {
    let mut net = FakeNet::full_mesh(2);
    net.disconnect(0, 1);
    net.copy_text(0, "fresh");
    net.advance(59 * 60 * 1000);
    net.connect(0, 1);
    net.run();
    assert_eq!(
        net.text(1).as_deref(),
        Some("fresh"),
        "under 1 hour: applied"
    );

    net.disconnect(0, 1);
    net.copy_text(1, "mine, keep it");
    net.advance(1);
    net.copy_text(0, "old news");
    net.advance(HOUR + 1);
    net.connect(0, 1);
    net.run();
    assert_eq!(
        net.text(1).as_deref(),
        Some("mine, keep it"),
        "over 1 hour: not applied automatically"
    );

    // Explicit fetch (uclip fetch): no age limit.
    assert!(matches!(net.fetch(1), FetchOutcome::Requested { .. }));
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("old news"));
    assert_eq!(net.fetch(1), FetchOutcome::AlreadyCurrent);
}

#[test]
fn pull_only_client_gets_the_newest_clip_with_no_age_limit() {
    let mut net = FakeNet::new();
    let a = net.add(Role::Full, FakeClipboard::new());
    let b = net.add(Role::Full, FakeClipboard::new());
    let phone = net.add(Role::PullOnly, FakeClipboard::new());
    net.connect(a, b);
    net.run();
    net.copy_text(a, "from yesterday");
    net.run();
    net.advance(20 * HOUR);

    // The app opens: connect, take the newest clip.
    net.connect(phone, b);
    net.run();
    assert_eq!(net.text(phone).as_deref(), Some("from yesterday"));
    assert_eq!(net.fetch(phone), FetchOutcome::AlreadyCurrent);
}

#[test]
fn pull_only_client_shares_its_copy_with_the_whole_group() {
    let mut net = FakeNet::new();
    let a = net.add(Role::Full, FakeClipboard::new());
    let b = net.add(Role::Full, FakeClipboard::new());
    let phone = net.add(Role::PullOnly, FakeClipboard::new());
    net.connect(a, b);
    net.connect(phone, a);
    net.run();
    // The app reads the clipboard once when it opens and shares it if new.
    net.copy_text(phone, "from the phone");
    net.run();
    assert_eq!(net.text(a).as_deref(), Some("from the phone"));
    assert_eq!(
        net.text(b).as_deref(),
        Some("from the phone"),
        "A passes it on"
    );
    assert!(net.converged(), "{}", net.describe());
}

#[test]
fn members_announce_to_pull_only_clients_instead_of_pushing() {
    let mut net = FakeNet::new();
    let a = net.add(Role::Full, FakeClipboard::new());
    let phone = net.add(Role::PullOnly, FakeClipboard::new());
    net.connect(a, phone);
    net.run();
    net.copy_text(a, "small");
    // The frame on the wire to the phone is a Latest.
    let pending = net.pending_links();
    assert_eq!(pending, vec![(a, phone)]);
    net.run();
    assert_eq!(net.text(phone).as_deref(), Some("small"));
}

#[test]
fn pull_only_client_with_no_member_online() {
    let mut net = FakeNet::new();
    let phone = net.add(Role::PullOnly, FakeClipboard::new());
    assert_eq!(net.fetch(phone), FetchOutcome::NoMemberOnline);
    net.copy_text(phone, "nobody hears this");
    assert!(has(&net, phone, |e| *e == Event::NoMemberOnline));

    // Only another pull-only client is around: still no member.
    let other = net.add(Role::PullOnly, FakeClipboard::new());
    net.connect(phone, other);
    net.run();
    assert_eq!(net.fetch(phone), FetchOutcome::NoMemberOnline);
}

#[test]
fn want_for_an_old_clip_gets_the_current_latest() {
    let mut net = FakeNet::full_mesh(2);
    net.copy_text(0, "one");
    net.run();
    let old = net.devices[0].engine.held().unwrap().id;
    net.advance(5);
    net.copy_text(0, "two");
    net.run();
    // Device 1 asks for the old one by hand.
    let frame = msg::encode(&Message::Want { id: old });
    let now = net.now(0);
    let d = &mut net.devices[0];
    d.engine.on_frame(device_id(1), &frame, &mut d.clip, now);
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("two"));
}

#[test]
fn bad_frames_and_tampered_clips_are_reported_not_applied() {
    let mut net = FakeNet::full_mesh(2);
    let now = net.now(1);
    let d = &mut net.devices[1];
    let ev = d
        .engine
        .on_frame(device_id(0), &[1, 4, 0xff], &mut d.clip, now);
    assert!(matches!(ev[..], [Event::BadFrame { .. }]));

    // A real clip with one byte changed.
    let mut src = uc_core::Engine::new(uc_core::Config::new(
        uc_core::DeviceRecord {
            id: device_id(0),
            name: "x".into(),
            kind: uc_core::DeviceKind::Mac,
        },
        Role::Full,
    ));
    let mut cb = FakeClipboard::new();
    cb.copy_text("genuine");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);
    impl uc_core::Peer for Sink {
        fn id(&self) -> uc_core::DeviceId {
            device_id(1)
        }
        fn send(&mut self, f: Vec<u8>) -> Result<(), uc_core::PeerGone> {
            self.0.lock().unwrap().push(f);
            Ok(())
        }
    }
    src.connect(Box::new(Sink(sink.clone())), now);
    src.on_local_change(&mut cb, now);
    let frames = sink.lock().unwrap().clone();
    let Some(Message::Clip { header, mut data }) = msg::decode(frames.last().unwrap()).unwrap()
    else {
        panic!("expected a clip frame");
    };
    data[0][0] ^= 1;
    let bad = msg::encode(&Message::Clip { header, data });
    let d = &mut net.devices[1];
    let ev = d.engine.on_frame(device_id(0), &bad, &mut d.clip, now);
    assert!(matches!(ev[..], [Event::BadClip { .. }]), "{ev:?}");
    assert_eq!(net.devices[1].clip.write_count(), 0);
}

#[test]
fn frames_from_unknown_peers_are_ignored() {
    let mut net = FakeNet::full_mesh(2);
    let frame = msg::encode(&Message::Removed);
    let now = net.now(0);
    let d = &mut net.devices[0];
    assert!(
        d.engine
            .on_frame(device_id(9), &frame, &mut d.clip, now)
            .is_empty()
    );
    let ev = d.engine.on_frame(device_id(1), &frame, &mut d.clip, now);
    assert_eq!(ev, vec![Event::Removed { by: device_id(1) }]);
}

#[test]
fn lost_peer_is_dropped_on_send() {
    let mut net = FakeNet::full_mesh(3);
    // The wire between 0 and 2 dies, and the engines are not told.
    net.cut(0, 2);
    net.copy_text(0, "still works");
    assert!(has(&net, 0, |e| *e == Event::PeerLost { peer: device_id(2) }));
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("still works"));
    assert_eq!(net.devices[0].engine.peers().count(), 1);
}

#[test]
fn write_failure_is_reported() {
    struct Broken;
    impl Clipboard for Broken {
        fn read(&mut self) -> Result<uc_clip::Snapshot, uc_clip::ClipboardError> {
            Ok(uc_clip::Snapshot::default())
        }
        fn write(&mut self, _: &[uc_clip::RawPart]) -> Result<u64, uc_clip::ClipboardError> {
            Err(uc_clip::ClipboardError::Denied)
        }
    }
    let mut net = FakeNet::full_mesh(2);
    net.devices[0].engine.set_paused(true);
    net.copy_text(1, "x");
    net.run();
    net.devices[0].engine.set_paused(false);
    let now = net.now(0);
    let (_, ev) = net.devices[0].engine.fetch(&mut Broken, now);
    assert_eq!(ev, vec![Event::WriteFailed]);
}
