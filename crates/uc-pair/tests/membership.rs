//! Membership tests: property tests for lines 1 to 4 of protocol 5.5, the sync gate (5.9) and the
//! self-wipe rule (5.7).

mod common;

use std::collections::BTreeSet;

use common::*;
use proptest::prelude::*;
use uc_keys::{DeviceKey, MemoryKeyStore};
use uc_pair::chain::*;
use uc_pair::gate::{Close, Gate};

fn dev(i: usize) -> DeviceKey {
    DeviceKey::from_seed(&arr32(&format!("pt-device-{i}")))
}

#[derive(Debug, Clone)]
enum Op {
    Add { signer: usize, device: usize },
    Remove { signer: usize, device: usize },
    Rename { signer: usize, device: usize },
    Rotate { signer: usize },
}

/// One step: an op, and a choice of parents among the existing changes (by position, mod len).
fn op_strategy(devices: usize) -> impl Strategy<Value = (Op, Vec<usize>)> {
    let d = 0..devices;
    let op = prop_oneof![
        3 => (d.clone(), d.clone()).prop_map(|(signer, device)| Op::Add { signer, device }),
        2 => (d.clone(), d.clone()).prop_map(|(signer, device)| Op::Remove { signer, device }),
        1 => (d.clone(), d.clone()).prop_map(|(signer, device)| Op::Rename { signer, device }),
        1 => d.prop_map(|signer| Op::Rotate { signer }),
    ];
    (op, proptest::collection::vec(any::<usize>(), 1..4))
}

/// Builds a random graph. Changes that are not well formed (signer never added, wrong epoch) are
/// skipped. Returns the changes in insertion order.
fn build(devices: usize, steps: &[(Op, Vec<usize>)]) -> (Graph, Vec<Change>) {
    let keys: Vec<DeviceKey> = (0..devices).map(dev).collect();
    let (mut g, _) = Graph::create(&keys[0], 1, "D0", 0, &mut TestRng::new("pt")).expect("genesis");
    let mut all: Vec<Change> = g.canonical().cloned().collect();
    for (n, (op, ps)) in steps.iter().enumerate() {
        let mut parents: Vec<ChangeId> = ps.iter().map(|p| *all[p % all.len()].id()).collect();
        parents.sort();
        parents.dedup();
        let (signer, payload) = match op {
            Op::Add { signer, device } => (
                *signer,
                Payload::Add {
                    device: keys[*device].public(),
                    device_type: 1,
                    name: format!("D{device}"),
                },
            ),
            Op::Remove { signer, device } => (
                *signer,
                Payload::Remove {
                    device: keys[*device].public(),
                },
            ),
            Op::Rename { signer, device } => (
                *signer,
                Payload::Rename {
                    device: keys[*device].public(),
                    name: format!("N{n}"),
                },
            ),
            Op::Rotate { signer } => {
                // Epoch: 1 plus the highest epoch in the past (computed by trying 1..=8).
                let mut stored = None;
                for epoch in 1..=8u32 {
                    let c = Change::sign(
                        Body {
                            signer: keys[*signer].public(),
                            time: n as u64,
                            parents: parents.clone(),
                            payload: Payload::Rotate {
                                epoch,
                                secret_commit: [epoch as u8; 32],
                            },
                        },
                        &keys[*signer],
                    );
                    if g.insert(c.clone()) == Ok(Inserted::Stored) {
                        stored = Some(c);
                        break;
                    }
                }
                if let Some(c) = stored {
                    all.push(c);
                }
                continue;
            }
        };
        let c = Change::sign(
            Body {
                signer: keys[signer].public(),
                time: n as u64,
                parents,
                payload,
            },
            &keys[signer],
        );
        if g.insert(c.clone()) == Ok(Inserted::Stored) {
            all.push(c);
        }
    }
    (g, all)
}

fn shuffle<T>(v: &mut [T], seed: u64) {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for i in (1..v.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        v.swap(i, (s % (i as u64 + 1)) as usize);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// 5.5 lines 1 to 4 on random graphs of up to 8 devices and 60 changes.
    #[test]
    fn membership_properties(
        devices in 2usize..=8,
        steps in proptest::collection::vec(op_strategy(8), 1..60),
        seed in any::<u64>(),
    ) {
        let steps: Vec<_> = steps
            .into_iter()
            .map(|(op, ps)| {
                let m = |x: usize| x % devices;
                let op = match op {
                    Op::Add { signer, device } => Op::Add { signer: m(signer), device: m(device) },
                    Op::Remove { signer, device } => Op::Remove { signer: m(signer), device: m(device) },
                    Op::Rename { signer, device } => Op::Rename { signer: m(signer), device: m(device) },
                    Op::Rotate { signer } => Op::Rotate { signer: m(signer) },
                };
                (op, ps)
            })
            .collect();
        let (g, all) = build(devices, &steps);
        let full = g.evaluate();

        // Line 1: same set, same result, in any arrival order.
        let mut order = all.clone();
        shuffle(&mut order, seed);
        let mut g2 = Graph::with_group_id(*g.group_id());
        let mut prev_effective: BTreeSet<ChangeId> = BTreeSet::new();
        let mut invalid_seen: BTreeSet<ChangeId> = BTreeSet::new();
        let mut ever_removed: BTreeSet<Key> = BTreeSet::new();
        for c in &order {
            g2.insert(c.clone()).unwrap();
            // The stored set is always a view; check lines 2 and 3 between growing views.
            let ev = g2.evaluate();
            // Line 3: invalid stays invalid.
            prop_assert!(invalid_seen.is_disjoint(&ev.valid), "invalid change became valid");
            for c2 in g2.canonical() {
                if c2.kind() != Kind::Remove && !ev.valid.contains(c2.id()) {
                    invalid_seen.insert(*c2.id());
                }
            }
            // Effective removes never change once known.
            prop_assert!(prev_effective.is_subset(&ev.effective_removes));
            prev_effective = ev.effective_removes.clone();
            // Line 2: removed stays removed, and is never a member.
            ever_removed.extend(ev.removed.iter().copied());
            prop_assert!(ever_removed.is_subset(&ev.removed));
            prop_assert!(ev.members.is_disjoint(&ever_removed));
        }
        prop_assert_eq!(g2.pool_len(), 0);
        prop_assert_eq!(&g2.evaluate(), &full);

        // Line 4: no back-dating. Every add, rename, rotate signed by a removed X that is not in
        // the past of every effective remove of X is invalid.
        for x in &full.removed {
            let removes: Vec<&ChangeId> = full
                .effective_removes
                .iter()
                .filter(|id| matches!(g.get(id).unwrap().body().payload, Payload::Remove { device } if device == *x))
                .collect();
            for c in g.canonical() {
                if c.signer() != x || matches!(c.kind(), Kind::Remove | Kind::Genesis) {
                    continue;
                }
                let in_all_pasts = removes.iter().all(|r| in_past(&g, c.id(), r));
                if !in_all_pasts {
                    prop_assert!(!full.valid.contains(c.id()), "back-dated change is valid");
                }
            }
        }
    }
}

/// True if `c` is in the strict past of `r`.
fn in_past(g: &Graph, c: &ChangeId, r: &ChangeId) -> bool {
    let mut stack = vec![*r];
    let mut seen = BTreeSet::new();
    while let Some(x) = stack.pop() {
        for p in &g.get(&x).unwrap().body().parents {
            if p == c {
                return true;
            }
            if seen.insert(*p) {
                stack.push(*p);
            }
        }
    }
    false
}

#[test]
fn invalid_stays_invalid_exhaustive_small() {
    // Every change seen invalid in a view is invalid in every larger view: check along all
    // prefixes of the canonical order of a few fixed random graphs.
    for seed in 0..30u64 {
        let mut rng = TestRng::from_u64(seed);
        let steps: Vec<(Op, Vec<usize>)> = (0..40)
            .map(|_| {
                let r =
                    |rng: &mut TestRng, n: u32| (rand_core::RngCore::next_u32(rng) % n) as usize;
                let op = match r(&mut rng, 7) {
                    0..=2 => Op::Add {
                        signer: r(&mut rng, 5),
                        device: r(&mut rng, 5),
                    },
                    3 | 4 => Op::Remove {
                        signer: r(&mut rng, 5),
                        device: r(&mut rng, 5),
                    },
                    5 => Op::Rename {
                        signer: r(&mut rng, 5),
                        device: r(&mut rng, 5),
                    },
                    _ => Op::Rotate {
                        signer: r(&mut rng, 5),
                    },
                };
                (op, vec![r(&mut rng, 1000), r(&mut rng, 1000)])
            })
            .collect();
        let (g, _) = build(5, &steps);
        assert!(g.len() > 1);
        let canon: Vec<Change> = g.canonical().cloned().collect();
        let mut invalid_seen: BTreeSet<ChangeId> = BTreeSet::new();
        let mut g2 = Graph::with_group_id(*g.group_id());
        for c in &canon {
            g2.insert(c.clone()).unwrap();
            let ev = g2.evaluate();
            for id in &invalid_seen {
                assert!(!ev.valid.contains(id), "seed {seed}: invalid became valid");
            }
            for c2 in g2.canonical() {
                if c2.kind() != Kind::Remove && !ev.valid.contains(c2.id()) {
                    invalid_seen.insert(*c2.id());
                }
            }
        }
    }
}

#[test]
fn signing_helpers() {
    let a = dev(0);
    let b = dev(1);
    let c = dev(2);
    let mut rng = TestRng::new("helpers");
    let (mut g, s0) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    assert!(g.secret_matches(&s0));
    assert!(g.sign_add(&a, b.public(), 2, "B", 2).unwrap().is_some());
    assert!(
        g.sign_add(&a, b.public(), 2, "B", 2).unwrap().is_none(),
        "already a member"
    );
    assert!(g.sign_add(&b, c.public(), 3, "C", 3).unwrap().is_some());
    g.sign_rename(&c, c.public(), "C2", 4).unwrap();
    assert_eq!(g.device_name(&c.public()).unwrap(), (3, "C2".to_string()));
    // Remove signs a rotate too.
    let (_, rot) = g.sign_remove(&a, c.public(), 5, &mut rng).unwrap();
    let (_, s1) = rot.unwrap();
    assert_eq!(s1.epoch, 1);
    assert!(g.secret_matches(&s1));
    assert!(!g.secret_matches(&s0));
    assert!(!g.needs_rotate());
    assert_eq!(
        g.sign_add(&a, c.public(), 3, "C", 6).unwrap_err(),
        AddError::Removed
    );
    // A removed device cannot sign.
    assert!(g.sign_rename(&c, c.public(), "X", 7).is_err());
    // Leaving: no rotate by the leaver, but the others see a rotate is needed.
    let (_, rot) = g.sign_remove(&b, b.public(), 8, &mut rng).unwrap();
    assert!(rot.is_none());
    assert!(g.needs_rotate());
    let (_, s2) = g.sign_rotate(&a, 9, &mut rng).unwrap();
    assert_eq!(s2.epoch, 2);
    assert!(!g.needs_rotate());
    assert_eq!(g.members(), [a.public()].into_iter().collect());
    // Heads are sorted and at most 8.
    let h = g.heads();
    assert!(h.windows(2).all(|w| w[0] < w[1]) && h.len() <= MAX_PARENTS);
}

#[test]
fn group_full_and_limits() {
    let a = dev(0);
    let mut rng = TestRng::new("full");
    let (mut g, _) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    for i in 1..8 {
        g.sign_add(&a, dev(i).public(), 1, "D", 1).unwrap();
    }
    assert_eq!(g.members().len(), MAX_MEMBERS);
    assert_eq!(
        g.sign_add(&a, dev(9).public(), 1, "D", 1).unwrap_err(),
        AddError::Full
    );
}

#[test]
fn soft_and_hard_limits() {
    let a = dev(0);
    let b = dev(1);
    let mut rng = TestRng::new("limits");
    let (mut g, _) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    g.sign_add(&a, b.public(), 1, "B", 1).unwrap();
    let mut n = 0;
    while g.len() < SOFT_LIMIT {
        g.sign_rename(&a, b.public(), if n % 2 == 0 { "B1" } else { "B2" }, 1)
            .unwrap();
        n += 1;
    }
    assert_eq!(
        g.sign_rename(&a, b.public(), "B3", 1).unwrap_err(),
        ChangeError::Limit
    );
    assert!(matches!(
        g.sign_add(&a, dev(5).public(), 1, "E", 1).unwrap_err(),
        AddError::Change(ChangeError::Limit)
    ));
    while g.len() < HARD_LIMIT {
        g.sign_rotate(&a, 1, &mut rng).unwrap();
    }
    assert_eq!(
        g.sign_rotate(&a, 1, &mut rng).unwrap_err(),
        ChangeError::Limit
    );
}

#[test]
fn pool_waits_for_parents_and_is_bounded() {
    let a = dev(0);
    let b = dev(1);
    let mut rng = TestRng::new("pool");
    let (mut g, _) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    let add = g.sign_add(&a, b.public(), 1, "B", 1).unwrap().unwrap();
    let ren = g.sign_rename(&b, b.public(), "B2", 2).unwrap();
    let mut g2 = Graph::with_group_id(*g.group_id());
    assert_eq!(g2.insert(ren.clone()).unwrap(), Inserted::Pooled);
    assert_eq!(g2.insert(ren.clone()).unwrap(), Inserted::Duplicate);
    assert_eq!(
        g2.insert(g.canonical().next().unwrap().clone()).unwrap(),
        Inserted::Stored
    );
    assert_eq!(g2.pool_len(), 1);
    assert_eq!(g2.insert(add).unwrap(), Inserted::Stored);
    assert_eq!(g2.pool_len(), 0);
    assert_eq!(g2.len(), 3);
    // Bounded: 256 orphans, the 257th is refused.
    let mut g3 = Graph::with_group_id(*g.group_id());
    for i in 0..POOL_LIMIT {
        let c = Change::sign(
            Body {
                signer: a.public(),
                time: i as u64,
                parents: vec![arr32(&format!("missing-{i}"))],
                payload: Payload::Remove { device: b.public() },
            },
            &a,
        );
        assert_eq!(g3.insert(c).unwrap(), Inserted::Pooled);
    }
    let c = Change::sign(
        Body {
            signer: a.public(),
            time: 999,
            parents: vec![arr32("missing-x")],
            payload: Payload::Remove { device: b.public() },
        },
        &a,
    );
    assert_eq!(g3.insert(c).unwrap_err(), ChangeError::PoolFull);
}

#[test]
fn self_wipe_only_from_own_evaluation() {
    let a = dev(0);
    let b = dev(1);
    let mut rng = TestRng::new("wipe");
    let (mut g, _) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    let add = g.sign_add(&a, b.public(), 1, "B", 1).unwrap().unwrap();
    // B's own graph.
    let mut gb = Graph::with_group_id(*g.group_id());
    gb.insert(g.canonical().next().unwrap().clone()).unwrap();
    gb.insert(add).unwrap();
    let store = MemoryKeyStore::new();
    let kb = DeviceKey::load_or_create(&store).unwrap();
    // A bare "you were removed" hint changes nothing: there is no change to evaluate.
    assert_eq!(self_action(&gb, &b.public()), SelfAction::Stay);
    // A forged remove, signed by a non-member, is refused and changes nothing.
    let mallory = dev(7);
    let forged = Change::sign(
        Body {
            signer: mallory.public(),
            time: 5,
            parents: gb.heads(),
            payload: Payload::Remove { device: b.public() },
        },
        &mallory,
    );
    assert_eq!(gb.insert(forged).unwrap_err(), ChangeError::UnknownSigner);
    assert_eq!(self_action(&gb, &b.public()), SelfAction::Stay);
    // The real removal, signed by A.
    let (rm, rot) = g.sign_remove(&a, b.public(), 6, &mut rng).unwrap();
    gb.insert(rm).unwrap();
    gb.insert(rot.unwrap().0).unwrap();
    assert_eq!(self_action(&gb, &b.public()), SelfAction::WipeAndRekey);
    let nk = rekey(&store).unwrap();
    assert_ne!(nk.public(), kb.public());
    assert_eq!(store.len(), 1);
}

#[test]
fn gate_order() {
    let a = dev(0);
    let b = dev(1);
    let c = dev(2);
    let mut rng = TestRng::new("gate");
    let (mut g, _) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    g.sign_add(&a, b.public(), 1, "B", 1).unwrap();
    let gid = *g.group_id();

    // A member peer: data flows only after membership sync.
    let mut gate = Gate::new(b.public());
    assert!(!gate.data_allowed() && !gate.membership_allowed());
    gate.on_hello(&g, &gid).unwrap();
    assert!(!gate.data_allowed() && gate.membership_allowed());
    gate.after_membership_sync(&g).unwrap();
    assert!(gate.data_allowed());

    // Another group: close.
    let mut gate = Gate::new(b.public());
    assert_eq!(gate.on_hello(&g, &[0; 32]), Err(Close::OtherGroup));
    assert!(!gate.data_allowed());

    // An unknown peer gets membership sync only; still not a member afterwards: close.
    let mut gate = Gate::new(c.public());
    gate.on_hello(&g, &gid).unwrap();
    assert!(gate.unknown_peer);
    assert_eq!(gate.after_membership_sync(&g), Err(Close::NotMember));
    assert!(!gate.data_allowed());

    // Order matters.
    let mut gate = Gate::new(b.public());
    assert_eq!(gate.after_membership_sync(&g), Err(Close::Order));

    // An open gate closes after the peer is removed; a removed key is refused at hello.
    let mut gate = Gate::new(b.public());
    gate.on_hello(&g, &gid).unwrap();
    gate.after_membership_sync(&g).unwrap();
    g.sign_remove(&a, b.public(), 2, &mut rng).unwrap();
    assert_eq!(gate.recheck(&g), Err(Close::NotMember));
    assert!(!gate.data_allowed());
    let mut gate = Gate::new(b.public());
    assert_eq!(gate.on_hello(&g, &gid), Err(Close::Removed));
}

#[test]
fn missing_from_gives_parents_first() {
    let a = dev(0);
    let b = dev(1);
    let mut rng = TestRng::new("missing");
    let (mut g, _) = Graph::create(&a, 1, "A", 1, &mut rng).unwrap();
    g.sign_add(&a, b.public(), 1, "B", 1).unwrap();
    g.sign_rename(&b, b.public(), "B2", 2).unwrap();
    let genesis = *g.canonical().next().unwrap().id();
    let missing = g.missing_from(&|id| *id == genesis);
    assert_eq!(missing.len(), 2);
    let mut g2 = Graph::with_group_id(genesis);
    g2.insert(g.get(&genesis).unwrap().clone()).unwrap();
    for c in missing {
        assert_eq!(g2.insert(c.clone()).unwrap(), Inserted::Stored);
    }
}
