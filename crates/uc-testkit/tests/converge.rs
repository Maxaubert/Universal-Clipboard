//! M1 acceptance: three fake devices converge in every random order (plan 6.1).

use proptest::prelude::*;
use uc_clip::{FakeClipboard, Format, RawPart};
use uc_core::Role;
use uc_testkit::{FakeNet, Manager, images};

#[derive(Clone, Debug)]
enum Op {
    /// The user copies on a device: kind 0 = short text, 1 = small image, 2 = big image.
    Copy { dev: usize, kind: u8 },
    /// Deliver the next frame on the n-th busy link.
    Deliver(usize),
    /// Time passes.
    Advance(u64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0usize..3, prop_oneof![6 => Just(0u8), 2 => Just(1u8), 1 => Just(2u8)])
            .prop_map(|(dev, kind)| Op::Copy { dev, kind }),
        6 => any::<usize>().prop_map(Op::Deliver),
        1 => (0u64..3_000).prop_map(Op::Advance),
    ]
}

fn parts(kind: u8, n: usize) -> Vec<RawPart> {
    match kind {
        0 => vec![(Format::TextPlain, format!("clip {n}\nline").into_bytes())],
        1 => vec![
            (Format::TextPlain, format!("caption {n}").into_bytes()),
            (Format::ImagePng, images::image(8, 8, n as u8)),
        ],
        // Over the push limit, so it goes through Latest and Want. Unique by size and seed.
        _ => vec![(
            Format::ImagePng,
            images::big_image(260 + (n % 50) as u32, 260, n as u8),
        )],
    }
}

fn play(ops: &[Op], managers: [Option<Manager>; 3], windows: [bool; 3]) -> FakeNet {
    let mut net = FakeNet::new();
    for w in windows {
        let clip = if w {
            FakeClipboard::windows()
        } else {
            FakeClipboard::new()
        };
        net.add(Role::Full, clip);
    }
    for (i, m) in managers.into_iter().enumerate() {
        net.devices[i].manager = m;
    }
    net.connect_all();
    let mut n = 0;
    for op in ops {
        match op {
            Op::Copy { dev, kind } => {
                n += 1;
                net.copy(*dev, parts(*kind, n));
            }
            Op::Deliver(c) => {
                net.step(*c);
            }
            Op::Advance(ms) => net.advance(*ms),
        }
    }
    // Then the network goes quiet, in any order.
    net.run_bounded(10_000);
    net
}

fn manager() -> impl Strategy<Value = Option<Manager>> {
    prop_oneof![
        2 => Just(None),
        1 => Just(Some(Manager::TextRewriter)),
        1 => Just(Some(Manager::PngReencoder)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(300)
    ))]

    #[test]
    fn three_devices_converge(ops in proptest::collection::vec(op(), 1..60)) {
        let net = play(&ops, [None, None, None], [false, false, false]);
        prop_assert!(net.converged(), "{}", net.describe());
        // The winner is one of the clips that were sent.
        let sent: Vec<_> = net
            .devices
            .iter()
            .flat_map(|d| d.events.iter())
            .filter_map(|e| match e {
                uc_core::Event::Sent { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let held = net.devices[0].engine.held().map(|h| h.id);
        prop_assert_eq!(held.is_some(), !sent.is_empty());
        if let Some(id) = held {
            prop_assert!(sent.contains(&id));
        }
    }

    #[test]
    fn three_devices_converge_with_managers_and_mixed_os(
        ops in proptest::collection::vec(op(), 1..60),
        managers in [manager(), manager(), manager()],
        windows in [any::<bool>(), any::<bool>(), any::<bool>()],
    ) {
        let net = play(&ops, managers, windows);
        prop_assert!(net.converged(), "{}", net.describe());
        // No loop: never more sends than user copies.
        let copies = ops.iter().filter(|o| matches!(o, Op::Copy { .. })).count();
        prop_assert!(net.sends() <= copies, "{} sends for {} copies", net.sends(), copies);
    }
}

/// Every delivery order, exhaustively, for three copies made at the same moment on three
/// devices (6 frames on 6 links: 720 orders).
#[test]
fn every_order_of_three_concurrent_copies() {
    fn orders(prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        let net = replay(prefix);
        let busy = net.pending_links().len();
        if busy == 0 {
            out.push(prefix.clone());
            return;
        }
        for c in 0..busy {
            prefix.push(c);
            orders(prefix, out);
            prefix.pop();
        }
    }
    fn replay(choices: &[usize]) -> FakeNet {
        let mut net = FakeNet::full_mesh(3);
        net.copy_text(0, "zero");
        net.copy_text(1, "one");
        net.copy_text(2, "two");
        for &c in choices {
            net.step(c);
        }
        net
    }
    let mut all = Vec::new();
    orders(&mut Vec::new(), &mut all);
    assert_eq!(all.len(), 720);
    let mut winner = None;
    for order in &all {
        let net = replay(order);
        assert!(net.converged(), "order {order:?}\n{}", net.describe());
        let w = net.text(0);
        if let Some(prev) = &winner {
            assert_eq!(&w, prev, "every order gives the same winner");
        }
        winner = Some(w);
    }
}
