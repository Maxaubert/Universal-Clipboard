//! File sync on the fake network (file sync spec 7, core list): offers, fetches, the serve
//! queue, cancel and retry rules, echo layers for files and hostile input.

use uc_clip::FakeClipboard;
use uc_core::files::{self, CODE_ABORTED};
use uc_core::msg::{self, FilesOffer, Message};
use uc_core::sync::{CancelReason, Event, FETCH_RETRIES, SERVE_MAX, SkipReason, TransferFailure};
use uc_core::{FilesId, Role, Stamp};
use uc_testkit::{FakeNet, device_id};

const MB: u64 = 1 << 20;
const HOUR: u64 = 60 * 60 * 1000;

fn has(net: &FakeNet, i: usize, pred: impl Fn(&Event) -> bool) -> bool {
    net.devices[i].events.iter().any(pred)
}

fn count(net: &FakeNet, i: usize, pred: impl Fn(&Event) -> bool) -> usize {
    net.devices[i].events.iter().filter(|e| pred(e)).count()
}

fn skipped(net: &FakeNet, i: usize, r: SkipReason) -> bool {
    has(net, i, |e| *e == Event::Skipped(r.clone()))
}

fn types(frames: &[Vec<u8>]) -> Vec<u8> {
    frames.iter().map(|f| f[1]).collect()
}

const T_FILES_OFFER: u8 = 9;

/// A small folder on device 0, copied in the file manager, reaches every device.
#[test]
fn small_copy_syncs_to_every_device() {
    let mut net = FakeNet::full_mesh(3);
    let dir = net.add_folder(
        0,
        "photos",
        &[("a.jpg", 3 * MB), ("trip/b.jpg", 4 * MB)],
        &["empty"],
    );
    let file = net.add_file(0, "notes.txt", 10);
    net.copy_paths(0, &[dir, file]);
    net.run();

    let id = net.current_files(0).expect("device 0 holds the copy");
    assert!(has(&net, 0, |e| matches!(e,
        Event::FilesSent { total, count: 3, peers: 2, .. } if *total == 7 * MB + 10)));
    for i in [1, 2] {
        assert_eq!(net.current_files(i), Some(id), "device {i}");
        assert!(has(&net, i, |e| matches!(e,
            Event::FilesApplied { count: 3, origin, .. } if *origin == device_id(0))));
        // The top-level items are on the clipboard, inside the copy's own folder.
        let on_clip = net.clipboard_files(i);
        assert_eq!(on_clip.len(), 2);
        assert!(on_clip[0].ends_with(format!("received/{}/notes.txt", id.short())));
        assert!(on_clip[1].ends_with(format!("received/{}/photos", id.short())));
        // Our own write is never sent back (layer 2 first, the marker as well).
        assert!(skipped(&net, i, SkipReason::OwnWrite));
        assert!(net.devices[i].engine.running_sends().is_empty());
    }
    // The text and image clip state is untouched: files never appear in Latest or Clip.
    assert!(net.devices[1].engine.held().is_none());
    assert!(net.streams.is_empty());
    assert_eq!(net.in_flight(), 0);
}

#[test]
fn over_the_limit_is_not_sent_automatically() {
    let mut net = FakeNet::full_mesh(2);
    let big = net.add_file(0, "movie.mkv", 100 * MB + 1);
    net.copy_paths(0, &[big]);
    net.run();
    assert!(skipped(&net, 0, SkipReason::FilesOverLimit));
    assert_eq!(net.current_files(0), None);
    assert_eq!(net.devices[1].engine.pending_files(), None);

    // Exactly at the limit is sent.
    let at = net.add_file(0, "at.bin", 100 * MB);
    net.copy_paths(0, &[at]);
    net.run();
    assert!(net.current_files(1).is_some());

    // The limit is a setting on the copying device.
    net.devices[0].engine.set_file_max(MB);
    let small = net.add_file(0, "two.bin", 2 * MB);
    net.copy_paths(0, &[small]);
    net.run();
    assert_eq!(
        count(&net, 0, |e| *e
            == Event::Skipped(SkipReason::FilesOverLimit)),
        2
    );
}

/// "Copy files..." has no byte limit, and its write-back to the local clipboard is our own:
/// no over-limit note, and it is not sent twice.
#[test]
fn picked_copy_over_the_limit_syncs_without_a_note() {
    let mut net = FakeNet::full_mesh(2);
    let big = net.add_file(0, "backup.zip", 500 * MB);
    net.pick(0, std::slice::from_ref(&big));
    net.run();
    let id = net.current_files(0).expect("picked copy is current");
    assert_eq!(net.current_files(1), Some(id));
    assert!(net.devices[0].engine.current_files().unwrap().picked);
    assert_eq!(net.clipboard_files(0), &[big]);
    assert!(net.devices[0].clip.peek().has_marker);
    assert!(skipped(&net, 0, SkipReason::OwnWrite));
    assert!(!skipped(&net, 0, SkipReason::FilesOverLimit));
    assert_eq!(count(&net, 0, |e| matches!(e, Event::FilesSent { .. })), 1);
}

#[test]
fn text_copied_during_a_walk_wins() {
    let mut net = FakeNet::full_mesh(2);
    let dir = net.add_folder(0, "src", &[("main.rs", 100)], &[]);
    net.hold_walks = true;
    net.copy_paths(0, std::slice::from_ref(&dir));
    assert!(net.devices[0].engine.walk_gen().is_some());
    net.copy_text(0, "newer text");
    assert_eq!(net.devices[0].engine.walk_gen(), None);
    net.hold_walks = false;
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("newer text"));
    assert!(!has(&net, 0, |e| matches!(e, Event::FilesSent { .. })));
    assert_eq!(net.current_files(0), None);

    // A remote copy applied during the walk ends it too.
    net.clear_events();
    net.hold_walks = true;
    net.copy_paths(0, &[dir]);
    net.copy_text(1, "from the other side");
    net.run();
    assert_eq!(net.text(0).as_deref(), Some("from the other side"));
    net.hold_walks = false;
    net.run();
    assert!(!has(&net, 0, |e| matches!(e, Event::FilesSent { .. })));
    assert!(net.converged(), "{}", net.describe());
}

#[test]
fn offers_go_out_only_after_the_peers_engine_hello() {
    let mut net = FakeNet::new();
    net.add(Role::Full, FakeClipboard::new());
    net.add(Role::Full, FakeClipboard::new());
    let f = net.add_file(0, "a.txt", 5);
    net.copy_paths(0, &[f]);
    net.run();
    assert!(has(&net, 0, |e| matches!(
        e,
        Event::FilesSent { peers: 0, .. }
    )));

    net.connect(0, 1);
    // On connect only Hello and Latest go out; Latest carries no header for a file copy.
    assert_eq!(types(&net.frames(0, 1)), [0, 3]);
    assert_eq!(
        msg::decode(&net.frames(0, 1)[1]),
        Ok(Some(Message::Latest { header: None }))
    );
    // Device 0 reads device 1's Hello: now the offer goes out.
    let k = net
        .pending_links()
        .iter()
        .position(|l| *l == (1, 0))
        .unwrap();
    net.step(k);
    assert_eq!(types(&net.frames(0, 1)), [0, 3, T_FILES_OFFER]);
    net.run();
    assert_eq!(net.current_files(1), net.current_files(0));
}

#[test]
fn old_peer_gets_nothing_and_stays_connected() {
    let mut net = FakeNet::new();
    net.add(Role::Full, FakeClipboard::new());
    net.add_with(Role::Full, FakeClipboard::new(), |c| c.features = 0);
    net.connect_all();
    net.run();
    let f = net.add_file(0, "a.txt", 5);
    net.copy_paths(0, &[f]);
    let walked_to = net.delivered;
    net.run();
    assert!(skipped(&net, 0, SkipReason::PeerNoFiles(device_id(1))));
    assert!(has(&net, 0, |e| matches!(
        e,
        Event::FilesSent { peers: 0, .. }
    )));
    assert_eq!(net.delivered, walked_to, "nothing was sent to the old peer");
    // Text still syncs both ways.
    net.copy_text(1, "still here");
    net.run();
    assert_eq!(net.text(0).as_deref(), Some("still here"));
    net.copy_text(0, "and back");
    net.run();
    assert_eq!(net.text(1).as_deref(), Some("and back"));
}

#[test]
fn pending_fetch_is_cancelled_by_newer_text() {
    let mut net = FakeNet::full_mesh(2);
    net.hold_streams = true;
    let f = net.add_file(0, "big.iso", 50 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.devices[1].engine.fetching().expect("fetching");
    assert_eq!(net.streams.len(), 1);

    net.copy_text(1, "newer");
    net.run();
    assert!(has(&net, 1, |e| *e
        == Event::CancelTransfer {
            id,
            peer: device_id(0),
            reason: CancelReason::Superseded,
            sending: false
        }));
    assert!(net.streams.is_empty());
    assert!(net.devices[0].engine.running_sends().is_empty());
    assert_eq!(net.text(0).as_deref(), Some("newer"));
    assert_eq!(net.current_files(0), None);
    assert_eq!(net.current_files(1), None);
    assert_eq!(net.devices[1].engine.fetching(), None);
}

#[test]
fn newer_files_cancel_a_running_send_and_fetch() {
    let mut net = FakeNet::full_mesh(3);
    net.hold_streams = true;
    let a = net.add_file(0, "a.bin", 10 * MB);
    net.copy_paths(0, &[a]);
    net.run();
    let old = net.current_files(0).unwrap();
    assert_eq!(net.streams.len(), 2);

    // Device 2 copies other files: device 0's sends and device 1's fetch of the old copy end.
    let b = net.add_file(2, "b.bin", 10 * MB);
    net.copy_paths(2, &[b]);
    net.run();
    let new = net.current_files(2).unwrap();
    assert!(has(&net, 0, |e| matches!(e,
        Event::CancelTransfer { id, sending: true, reason: CancelReason::Superseded, .. } if *id == old)));
    assert!(has(&net, 1, |e| matches!(e,
        Event::CancelTransfer { id, sending: false, .. } if *id == old)));
    assert!(net.streams.iter().all(|s| s.id == new && s.from == 2));
    net.hold_streams = false;
    net.run();
    for i in 0..3 {
        assert_eq!(net.current_files(i), Some(new), "device {i}");
    }
}

#[test]
fn received_files_are_never_offered_again() {
    let mut net = FakeNet::full_mesh(3);
    let f = net.add_file(0, "a.txt", 5);
    net.copy_paths(0, &[f]);
    net.run();
    assert!(net.current_files(2).is_some());
    // Receivers reconnect: only the origin offers, and nobody fetches twice.
    net.disconnect(1, 2);
    net.connect(1, 2);
    net.run();
    for i in [1, 2] {
        assert_eq!(count(&net, i, |e| matches!(e, Event::FetchFiles { .. })), 1);
        assert!(!has(&net, i, |e| matches!(e, Event::FilesSent { .. })));
    }
    // The receiver's clipboard change is our own write: layer 2, and the marker.
    net.devices[1].events.clear();
    net.local_change(1);
    assert!(skipped(&net, 1, SkipReason::OwnWrite));
}

#[test]
fn the_same_files_are_not_sent_twice() {
    let mut net = FakeNet::full_mesh(2);
    let dir = net.add_folder(0, "docs", &[("a.pdf", 10), ("b.pdf", 20)], &[]);
    net.copy_paths(0, std::slice::from_ref(&dir));
    net.run();
    // The user copies the same folder again: layer 4.
    net.copy_paths(0, std::slice::from_ref(&dir));
    net.run();
    assert!(skipped(&net, 0, SkipReason::SeenContent));
    // The receiver copies the received folder in its file manager: layer 4 too.
    let received = net.clipboard_files(1).to_vec();
    net.devices[1].clip.copy_paths(&received);
    net.local_change(1);
    net.run();
    assert!(skipped(&net, 1, SkipReason::SeenContent));
    assert_eq!(count(&net, 1, |e| matches!(e, Event::FilesSent { .. })), 0);

    // After another copy in between, the same folder is a new copy and syncs again.
    net.copy_text(1, "something else");
    net.run();
    net.copy_paths(0, &[dir]);
    net.run();
    assert_eq!(count(&net, 0, |e| matches!(e, Event::FilesSent { .. })), 2);
    assert_eq!(net.current_files(1), net.current_files(0));
}

#[test]
fn sends_run_two_at_once_and_wait_in_order() {
    let mut net = FakeNet::full_mesh(4);
    net.hold_streams = true;
    let f = net.add_file(0, "big.bin", 80 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    let e = &net.devices[0].engine;
    assert_eq!(e.running_sends().len(), SERVE_MAX);
    assert_eq!(e.queued_sends(), vec![(device_id(3), id)]);
    assert_eq!(
        e.running_sends(),
        &[(device_id(1), id), (device_id(2), id)],
        "first come, first served"
    );

    net.deliver_stream(0);
    assert_eq!(net.current_files(1), Some(id));
    let e = &net.devices[0].engine;
    assert_eq!(e.running_sends(), &[(device_id(2), id), (device_id(3), id)]);
    assert!(e.queued_sends().is_empty());
    net.hold_streams = false;
    net.run();
    for i in 1..4 {
        assert_eq!(net.current_files(i), Some(id), "device {i}");
    }
    assert_eq!(count(&net, 0, |e| matches!(e, Event::ServeFiles { .. })), 3);
}

#[test]
fn a_new_want_from_the_same_peer_resets_its_old_stream() {
    let mut net = FakeNet::full_mesh(2);
    net.hold_streams = true;
    let f = net.add_file(0, "a.bin", MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    net.inject(1, 0, &msg::encode(&Message::FilesWant { id }));
    assert!(has(&net, 0, |e| *e
        == Event::CancelTransfer {
            id,
            peer: device_id(1),
            reason: CancelReason::Requested,
            sending: true
        }));
    assert_eq!(net.devices[0].engine.running_sends(), &[(device_id(1), id)]);
    assert_eq!(net.streams.len(), 1);
    assert_eq!(count(&net, 0, |e| matches!(e, Event::ServeFiles { .. })), 2);
}

#[test]
fn a_broken_fetch_is_asked_again_when_the_origin_is_back() {
    let mut net = FakeNet::full_mesh(2);
    net.hold_streams = true;
    let f = net.add_file(0, "a.bin", 20 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();

    // A Wi-Fi blip: the link and its stream die, then come back.
    net.disconnect(0, 1);
    assert_eq!(net.devices[1].engine.fetching(), None);
    assert_eq!(
        net.devices[1].engine.pending_files().map(|o| o.id),
        Some(id)
    );
    net.connect(0, 1);
    net.run();
    assert_eq!(count(&net, 1, |e| matches!(e, Event::FetchFiles { .. })), 2);
    assert_eq!(net.streams.len(), 1);

    // A link replaced mid-transfer counts the same.
    net.connect(0, 1);
    net.run();
    assert_eq!(count(&net, 1, |e| matches!(e, Event::FetchFiles { .. })), 3);

    net.hold_streams = false;
    net.run();
    assert_eq!(net.current_files(1), Some(id));
}

#[test]
fn a_fetch_gives_up_after_three_retries() {
    let mut net = FakeNet::full_mesh(2);
    net.hold_streams = true;
    let f = net.add_file(0, "a.bin", 20 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    for _ in 0..=FETCH_RETRIES {
        net.disconnect(0, 1);
        net.connect(0, 1);
        net.run();
    }
    assert!(has(&net, 1, |e| *e
        == Event::TransferFailed {
            id,
            peer: device_id(0),
            failure: TransferFailure::Retries
        }));
    assert_eq!(
        count(&net, 1, |e| matches!(e, Event::FetchFiles { .. })),
        1 + FETCH_RETRIES as usize
    );
    assert!(net.streams.is_empty());
    assert_eq!(net.current_files(1), None);
}

#[test]
fn a_hostile_manifest_is_refused() {
    let mut net = FakeNet::full_mesh(2);
    net.tamper = true;
    let f = net.add_file(0, "a.txt", 5);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    assert!(has(&net, 1, |e| *e
        == Event::TransferFailed {
            id,
            peer: device_id(0),
            failure: TransferFailure::BadData
        }));
    assert_eq!(net.current_files(1), None);
    assert!(net.clipboard_files(1).is_empty());
    // It is not fetched again when the origin offers it again.
    net.disconnect(0, 1);
    net.connect(0, 1);
    net.run();
    assert_eq!(count(&net, 1, |e| matches!(e, Event::FetchFiles { .. })), 1);
}

#[test]
fn inconsistent_or_forwarded_offers_are_refused() {
    let mut net = FakeNet::full_mesh(3);
    let stamp = Stamp {
        wall_ms: net.now_ms,
        counter: 0,
        device: device_id(0),
    };
    let good = FilesOffer {
        id: files::files_id(&[3; 32], &stamp),
        origin: device_id(0),
        stamp,
        total: 1,
        count: 1,
        manifest_hash: [3; 32],
        picked: false,
    };
    // Wrong ID for the manifest hash.
    let mut bad = good.clone();
    bad.manifest_hash = [4; 32];
    net.inject(0, 1, &msg::encode(&Message::FilesOffer(bad)));
    // A device that is not the origin offers it.
    net.inject(2, 1, &msg::encode(&Message::FilesOffer(good)));
    assert_eq!(count(&net, 1, |e| matches!(e, Event::BadClip { .. })), 2);
    assert_eq!(net.devices[1].engine.pending_files(), None);
}

#[test]
fn unsolicited_streams_are_not_allowed() {
    let mut net = FakeNet::full_mesh(3);
    net.hold_streams = true;
    let f = net.add_file(0, "a.bin", MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    let e = &net.devices[1].engine;
    assert!(e.stream_allowed(device_id(0), id));
    assert!(!e.stream_allowed(device_id(2), id), "wrong peer");
    assert!(
        !e.stream_allowed(device_id(0), FilesId([9; 32])),
        "wrong id"
    );
}

#[test]
fn stop_cancels_on_both_sides_and_is_not_retried() {
    let mut net = FakeNet::full_mesh(2);
    net.hold_streams = true;
    let f = net.add_file(0, "a.bin", 20 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    net.stop(0);
    net.run();
    assert!(has(&net, 0, |e| *e
        == Event::CancelTransfer {
            id,
            peer: device_id(1),
            reason: CancelReason::Stopped,
            sending: true
        }));
    assert!(has(&net, 1, |e| *e
        == Event::CancelTransfer {
            id,
            peer: device_id(0),
            reason: CancelReason::ByPeer(CODE_ABORTED),
            sending: false
        }));
    assert!(net.streams.is_empty());
    // Not offered or fetched again after a reconnect.
    net.disconnect(0, 1);
    net.connect(0, 1);
    net.run();
    assert_eq!(count(&net, 1, |e| matches!(e, Event::FetchFiles { .. })), 1);

    // Stop on the receiving side.
    let g = net.add_file(0, "b.bin", 20 * MB);
    net.copy_paths(0, &[g]);
    net.run();
    let id = net.current_files(0).unwrap();
    net.stop(1);
    net.run();
    assert!(has(&net, 1, |e| matches!(e,
        Event::CancelTransfer { id: x, reason: CancelReason::Stopped, sending: false, .. } if *x == id)));
    assert!(net.streams.is_empty());
    assert!(net.devices[0].engine.running_sends().is_empty());
    assert_eq!(net.current_files(1), None);
}

#[test]
fn pause_and_sync_files_off_cancel_transfers() {
    let mut net = FakeNet::full_mesh(2);
    net.hold_streams = true;
    let f = net.add_file(0, "a.bin", 20 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    net.set_paused(1, true);
    net.run();
    assert!(has(&net, 1, |e| matches!(
        e,
        Event::CancelTransfer {
            reason: CancelReason::Paused,
            ..
        }
    )));
    assert!(net.streams.is_empty());
    assert!(net.devices[0].engine.running_sends().is_empty());

    net.set_paused(1, false);
    let g = net.add_file(0, "b.bin", 20 * MB);
    net.copy_paths(0, &[g]);
    net.run();
    assert_eq!(net.streams.len(), 1);
    net.set_sync_files(0, false);
    net.run();
    assert!(has(&net, 0, |e| matches!(
        e,
        Event::CancelTransfer {
            reason: CancelReason::FilesOff,
            sending: true,
            ..
        }
    )));
    assert!(net.streams.is_empty());

    // With the setting off, a file copy is skipped with its own note.
    let h = net.add_file(0, "c.bin", 1);
    net.copy_paths(0, &[h]);
    assert!(skipped(&net, 0, SkipReason::FilesOff));
}

#[test]
fn not_enough_space_fails_without_asking() {
    let mut net = FakeNet::full_mesh(2);
    net.free_space[1] = 10 * MB;
    let f = net.add_file(0, "a.bin", 10 * MB);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    assert!(has(&net, 1, |e| *e
        == Event::TransferFailed {
            id,
            peer: device_id(0),
            failure: TransferFailure::NoSpace
        }));
    assert!(!has(&net, 0, |e| matches!(e, Event::ServeFiles { .. })));
    assert_eq!(net.current_files(1), None);
}

#[test]
fn an_old_offer_is_only_remembered() {
    let mut net = FakeNet::new();
    net.add(Role::Full, FakeClipboard::new());
    net.add(Role::Full, FakeClipboard::new());
    let f = net.add_file(0, "a.bin", 5);
    net.copy_paths(0, &[f]);
    net.run();
    net.advance(2 * HOUR);
    net.connect_all();
    net.run();
    let id = net.current_files(0).unwrap();
    assert!(has(&net, 1, |e| *e == Event::FilesHeldOnly { id }));
    assert!(!has(&net, 1, |e| matches!(e, Event::FetchFiles { .. })));
    // A later local copy on device 1 is still newer than the remembered offer.
    net.copy_text(1, "later");
    net.run();
    assert_eq!(net.text(0).as_deref(), Some("later"));
    assert_eq!(net.devices[1].engine.pending_files(), None);
}

#[test]
fn walk_failures_and_virtual_files_are_skipped() {
    let mut net = FakeNet::full_mesh(2);
    let f = net.add_file(0, "online-only.docx", 5);
    net.fs_fail.insert(f.clone(), SkipReason::FilesCloudOnly);
    net.copy_paths(0, &[f]);
    net.run();
    assert!(skipped(&net, 0, SkipReason::FilesCloudOnly));
    net.devices[0]
        .clip
        .copy_virtual_files("mail attachment.pdf");
    net.local_change(0);
    assert!(skipped(&net, 0, SkipReason::FilesVirtual));
    // A stale walk result is dropped, and a stale failure too.
    assert_eq!(net.devices[0].engine.on_walked(999, 1), vec![]);
    assert_eq!(
        net.devices[0]
            .engine
            .walk_failed(999, SkipReason::FilesTooMany),
        vec![]
    );
    assert_eq!(net.in_flight(), 0);
    assert_eq!(net.current_files(1), None);
}

#[test]
fn a_want_for_a_copy_we_moved_on_from_gets_a_cancel() {
    let mut net = FakeNet::full_mesh(2);
    let f = net.add_file(0, "a.bin", 5);
    net.copy_paths(0, &[f]);
    net.run();
    let id = net.current_files(0).unwrap();
    net.copy_text(0, "moved on");
    net.run();
    net.inject(1, 0, &msg::encode(&Message::FilesWant { id }));
    let frames = net.frames(0, 1);
    assert_eq!(
        msg::decode(frames.last().unwrap()),
        Ok(Some(Message::FilesCancel {
            id,
            reason: files::CODE_SUPERSEDED
        }))
    );
    assert!(net.devices[0].engine.running_sends().is_empty());
}
