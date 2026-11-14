//! Test vectors (protocol section 6).
//!
//! `cargo test -p uc-pair -- --ignored gen_vectors` writes the files in `tests/vectors/`. The
//! generator checks every hand-written expectation against the code before it writes. The other
//! tests only read the files.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;

use common::*;
use data_encoding::{BASE32, BASE32_NOPAD};
use serde_json::{Value, json};
use uc_keys::DeviceKey;
use uc_pair::chain::{Body, Change, Graph, Payload, secret_commit};
use uc_pair::pake::{Mode, PairSecret, Pake, TranscriptInput, VERSION, confirmation, verify_mac};
use uc_pair::qr::QrPayload;

// ---------------------------------------------------------------- pairing-v1.json

struct PairCase {
    name: &'static str,
    note: &'static str,
    mode: u8,
    code: Option<&'static str>,
    qr_secret: Option<[u8; 16]>,
    inviter_code: Option<&'static str>,
    joiner_code: Option<&'static str>,
    mitm_keys: bool,
    mitm_exporter: bool,
    tamper: Option<Vec<u8>>,
    result: &'static str,
}

fn pair_cases() -> Vec<PairCase> {
    let qr: [u8; 16] = derive("qr-secret", 16).try_into().unwrap();
    let base = |name, note, result| PairCase {
        name,
        note,
        mode: 1,
        code: Some("418207"),
        qr_secret: None,
        inviter_code: Some("418207"),
        joiner_code: Some("418207"),
        mitm_keys: false,
        mitm_exporter: false,
        tamper: None,
        result,
    };
    let mut low = vec![0x42, 0x01];
    low.extend_from_slice(&[0u8; 31]);
    vec![
        base("code-ok", "6-digit code, both sides honest", "ok"),
        PairCase {
            mode: 2,
            code: None,
            qr_secret: Some(qr),
            inviter_code: None,
            joiner_code: None,
            ..base("qr-ok", "QR secret, both sides honest", "ok")
        },
        PairCase {
            joiner_code: Some("418208"),
            ..base(
                "code-wrong",
                "the joiner types another code: the joiner refuses conf_a and sends nothing more",
                "joiner-rejects-conf-a",
            )
        },
        PairCase {
            mitm_keys: true,
            ..base(
                "mitm-keys",
                "a man in the middle: each side sees a third key in TLS",
                "joiner-rejects-conf-a",
            )
        },
        PairCase {
            mitm_exporter: true,
            ..base(
                "mitm-exporter",
                "keys are equal but the TLS exporters differ (two TLS sessions)",
                "joiner-rejects-conf-a",
            )
        },
        PairCase {
            mode: 2,
            qr_secret: Some(qr),
            inviter_code: None,
            ..base(
                "mode-mismatch",
                "the joiner sends mode 2 and uses the 6 code digits as the QR secret; the inviter uses its QR secret",
                "joiner-rejects-conf-a",
            )
        },
        PairCase {
            code: Some("000042"),
            inviter_code: Some("000042"),
            joiner_code: Some("000042"),
            ..base(
                "code-leading-zeros",
                "a code with leading zeros is 6 ASCII digits",
                "ok",
            )
        },
        PairCase {
            tamper: Some(low),
            ..base(
                "low-order-msg-b",
                "msg_b is replaced by the identity point (low order): the inviter refuses it, a counted try",
                "bad-message",
            )
        },
        PairCase {
            tamper: Some(vec![]), // filled in below: msg_b with side byte A
            ..base(
                "wrong-side-byte",
                "msg_b carries side byte 0x41 (A): the inviter refuses it",
                "bad-message",
            )
        },
    ]
}

/// Computes the `expect` object of a pairing vector from its inputs.
fn run_pair(v: &Value) -> Value {
    let b32 = |k: &str| -> [u8; 32] { unhex(v[k].as_str().unwrap()).try_into().unwrap() };
    let mode = v["mode"].as_u64().unwrap() as u8;
    let m = Mode::from_byte(mode).unwrap();
    let qr: Option<[u8; 16]> = v["qr_secret"]
        .as_str()
        .map(|s| unhex(s).try_into().unwrap());
    let code_of = |k: &str| v[k].as_str().map(|s| PairSecret::code(s).unwrap());
    let inv_secret = match m {
        Mode::Code => code_of("inviter_code").unwrap(),
        Mode::Qr => PairSecret::Qr(qr.unwrap()),
    };
    let join_secret = match (m, code_of("joiner_code")) {
        (Mode::Code, c) => c.unwrap(),
        (Mode::Qr, Some(c)) => c,
        (Mode::Qr, None) => PairSecret::Qr(qr.unwrap()),
    };
    let ik = b32("inviter_key");
    let jk = b32("joiner_key");
    let i_sees = b32("inviter_sees_joiner_key");
    let j_sees = b32("joiner_sees_inviter_key");
    let i_exp = b32("inviter_exporter");
    let j_exp = b32("joiner_exporter");
    let mut irng = FixedRng::new(&unhex(v["inviter_rng"].as_str().unwrap()));
    let mut jrng = FixedRng::new(&unhex(v["joiner_rng"].as_str().unwrap()));

    let (jp, msg_b) = Pake::start(false, &join_secret.password(m), &j_sees, &jk, &mut jrng);
    assert!(jrng.used_all());
    let delivered = match v["tamper_msg_b"].as_str() {
        Some(t) => unhex(t),
        None => msg_b.to_vec(),
    };
    let (ip, msg_a) = Pake::start(true, &inv_secret.password(m), &ik, &i_sees, &mut irng);
    assert!(irng.used_all());
    let Some(ki) = ip.finish(&delivered) else {
        return json!({ "msg_b": hx(&msg_b), "msg_a": null, "inviter": null, "joiner": null,
                       "result": "bad-message" });
    };
    let ci = confirmation(
        &ki,
        &TranscriptInput {
            version: VERSION,
            mode,
            inviter_key: &ik,
            joiner_key: &i_sees,
            exporter: &i_exp,
            msg_b: &delivered,
            msg_a: &msg_a,
        },
    );
    let kj = jp.finish(&msg_a).expect("honest msg_a");
    let cj = confirmation(
        &kj,
        &TranscriptInput {
            version: VERSION,
            mode,
            inviter_key: &j_sees,
            joiner_key: &jk,
            exporter: &j_exp,
            msg_b: &msg_b,
            msg_a: &msg_a,
        },
    );
    let side = |k: &[u8; 32], c: &uc_pair::pake::Confirmation| {
        json!({ "k": hx(k), "tt": hx(&c.tt), "t": hx(&c.t), "kc_a": hx(&c.kc_a), "kc_b": hx(&c.kc_b),
                "conf_a": hx(&c.conf_a), "conf_b": hx(&c.conf_b) })
    };
    let result = if !verify_mac(&cj.kc_a, &cj.t, &ci.conf_a) {
        "joiner-rejects-conf-a"
    } else if !verify_mac(&ci.kc_b, &ci.t, &cj.conf_b) {
        "inviter-rejects-conf-b"
    } else {
        "ok"
    };
    json!({ "msg_b": hx(&msg_b), "msg_a": hx(&msg_a), "inviter": side(&ki, &ci),
            "joiner": side(&kj, &cj), "result": result })
}

fn gen_pairing() -> Value {
    let ik = DeviceKey::from_seed(&arr32("inviter-seed")).public();
    let jk = DeviceKey::from_seed(&arr32("joiner-seed")).public();
    let mk = DeviceKey::from_seed(&arr32("mitm-seed")).public();
    let exp = arr32("exporter");
    let exp2 = arr32("exporter-other");
    let mut out = Vec::new();
    for c in pair_cases() {
        let irng = derive(&format!("{}-inviter-rng", c.name), 64);
        let jrng = derive(&format!("{}-joiner-rng", c.name), 64);
        let mut tamper = c.tamper.clone();
        if c.name == "wrong-side-byte" {
            let mut r = FixedRng::new(&jrng);
            let pw = PairSecret::code(c.joiner_code.unwrap())
                .unwrap()
                .password(Mode::Code);
            let (_, mut mb) = Pake::start(false, &pw, &ik, &jk, &mut r);
            mb[0] = 0x41;
            tamper = Some(mb.to_vec());
        }
        let mut v = json!({
            "name": c.name, "note": c.note, "mode": c.mode,
            "code": c.code, "qr_secret": c.qr_secret.map(|q| hx(&q)),
            "inviter_key": hx(&ik), "joiner_key": hx(&jk),
            "inviter_sees_joiner_key": hx(if c.mitm_keys { &mk } else { &jk }),
            "joiner_sees_inviter_key": hx(if c.mitm_keys { &mk } else { &ik }),
            "inviter_exporter": hx(&exp),
            "joiner_exporter": hx(if c.mitm_exporter { &exp2 } else { &exp }),
            "inviter_code": c.inviter_code, "joiner_code": c.joiner_code,
            "inviter_rng": hx(&irng), "joiner_rng": hx(&jrng),
            "tamper_msg_b": tamper.map(|t| hx(&t)),
        });
        let e = run_pair(&v);
        assert_eq!(e["result"], c.result, "{}", c.name);
        v["expect"] = e;
        out.push(v);
    }
    json!({ "format": 1, "vectors": out })
}

// ---------------------------------------------------------------- qr-v1.json

fn gen_qr() -> Value {
    let key = DeviceKey::from_seed(&arr32("inviter-seed")).public();
    let secret: [u8; 16] = derive("qr-secret", 16).try_into().unwrap();
    let mk = |addrs: &[&str], port| QrPayload {
        inviter_key: key,
        qr_secret: secret,
        port,
        addrs: addrs.iter().map(|a| a.parse::<IpAddr>().unwrap()).collect(),
    };
    let mut out = Vec::new();
    let mut valid = |name: &str, note: &str, p: QrPayload| {
        let addrs: Vec<String> = p.addrs.iter().map(|a| a.to_string()).collect();
        out.push(
            json!({ "name": name, "note": note, "inviter_key": hx(&p.inviter_key),
            "qr_secret": hx(&p.qr_secret), "port": p.port, "addrs": addrs,
            "payload": hx(&p.to_bytes()), "text": p.to_text(), "valid": true }),
        );
    };
    valid("one-ipv4", "one IPv4 address", mk(&["192.168.1.20"], 51820));
    valid(
        "two-addresses",
        "IPv4 then IPv6 link-local",
        mk(&["192.168.1.20", "fe80::1c2d:3eff:fe4f:5a6b"], 51820),
    );
    valid(
        "four-addresses",
        "the most addresses a payload may hold",
        mk(
            &["10.0.0.5", "172.16.4.9", "fe80::1", "fd12:3456:789a::1"],
            40000,
        ),
    );
    let good = mk(&["192.168.1.20"], 51820).to_bytes();
    let enc = |b: &[u8]| format!("UCLIP1:{}", BASE32_NOPAD.encode(b));
    let mut invalid = |name: &str, note: &str, text: String| {
        out.push(json!({ "name": name, "note": note, "text": text, "valid": false }));
    };
    invalid(
        "wrong-prefix",
        "prefix UCLIP2: is not ours",
        format!("UCLIP2:{}", BASE32_NOPAD.encode(&good)),
    );
    invalid(
        "lower-case-prefix",
        "the prefix is case sensitive",
        format!("uclip1:{}", BASE32_NOPAD.encode(&good)),
    );
    let mut b = good.clone();
    b[0] = 2;
    invalid("version-2", "payload version 2 is unknown", enc(&b));
    let mut b = good[..51].to_vec();
    b.push(0);
    invalid("zero-addresses", "n is 0", enc(&b));
    let mut b = good[..51].to_vec();
    b.push(5);
    for i in 0..5 {
        b.extend_from_slice(&[4, 192, 168, 1, 20 + i]);
    }
    invalid("five-addresses", "n is 5", enc(&b));
    let mut b = good.clone();
    b[52] = 5;
    invalid("unknown-family", "family byte 0x05", enc(&b));
    let mut b = good.clone();
    b.push(0);
    invalid("one-byte-too-many", "a byte left over", enc(&b));
    let b = &good[..good.len() - 1];
    invalid("one-byte-too-few", "the address is cut short", enc(b));
    invalid(
        "lower-case-base32",
        "base32 must be upper case",
        format!("UCLIP1:{}", BASE32_NOPAD.encode(&good).to_lowercase()),
    );
    assert!(BASE32.encode(&good).ends_with('='));
    invalid(
        "padding",
        "padding characters are not allowed",
        format!("UCLIP1:{}", BASE32.encode(&good)),
    );
    invalid("empty", "nothing after the prefix", "UCLIP1:".to_string());
    for v in &out {
        check_qr_vector(v);
    }
    json!({ "format": 1, "vectors": out })
}

fn check_qr_vector(v: &Value) {
    let text = v["text"].as_str().unwrap();
    let r = QrPayload::parse(text);
    if v["valid"].as_bool().unwrap() {
        let p = r.unwrap_or_else(|e| panic!("{}: {e:?}", v["name"]));
        assert_eq!(hx(&p.inviter_key), v["inviter_key"].as_str().unwrap());
        assert_eq!(hx(&p.qr_secret), v["qr_secret"].as_str().unwrap());
        assert_eq!(p.port as u64, v["port"].as_u64().unwrap());
        let addrs: Vec<String> = p.addrs.iter().map(|a| a.to_string()).collect();
        let want: Vec<String> = v["addrs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().parse::<IpAddr>().unwrap().to_string())
            .collect();
        assert_eq!(addrs, want);
        assert_eq!(hx(&p.to_bytes()), v["payload"].as_str().unwrap());
        assert_eq!(p.to_text(), text);
    } else {
        assert!(r.is_err(), "{} must be refused", v["name"]);
    }
}

// ---------------------------------------------------------------- membership-v1.json

const NAMES: [&str; 6] = ["A", "B", "C", "W", "X", "Y"];

struct Sc {
    keys: BTreeMap<&'static str, DeviceKey>,
    changes: Vec<(String, Change, Option<&'static str>)>,
    ids: HashMap<String, [u8; 32]>,
    t: u64,
}

impl Sc {
    fn new() -> Sc {
        let keys = NAMES
            .iter()
            .map(|n| (*n, DeviceKey::from_seed(&arr32(&format!("device-{n}")))))
            .collect();
        Sc {
            keys,
            changes: Vec::new(),
            ids: HashMap::new(),
            t: 1_763_000_000,
        }
    }
    fn key(&self, n: &str) -> [u8; 32] {
        self.keys[n].public()
    }
    fn parents(&self, ps: &[&str]) -> Vec<[u8; 32]> {
        let mut v: Vec<_> = ps.iter().map(|p| self.ids[*p]).collect();
        v.sort();
        v
    }
    fn push(&mut self, label: &str, signer: &str, parents: &[&str], payload: Payload) -> Change {
        self.t += 1;
        let body = Body {
            signer: self.key(signer),
            time: self.t,
            parents: self.parents(parents),
            payload,
        };
        let c = Change::sign(body, &self.keys[signer]);
        self.ids.insert(label.to_string(), *c.id());
        self.changes.push((label.to_string(), c.clone(), None));
        c
    }
    fn genesis(&mut self, label: &str, signer: &str) {
        let commit = secret_commit(0, &arr32(&format!("secret-{label}")));
        let p = Payload::Genesis {
            group_nonce: derive("group-nonce", 16).try_into().unwrap(),
            device_type: 1,
            name: format!("Device {signer}"),
            secret_commit: commit,
        };
        self.push(label, signer, &[], p);
    }
    fn add(&mut self, label: &str, signer: &str, dev: &str, ps: &[&str]) {
        let p = Payload::Add {
            device: self.key(dev),
            device_type: 2,
            name: format!("Device {dev}"),
        };
        self.push(label, signer, ps, p);
    }
    fn remove(&mut self, label: &str, signer: &str, dev: &str, ps: &[&str]) {
        let p = Payload::Remove {
            device: self.key(dev),
        };
        self.push(label, signer, ps, p);
    }
    fn rename(&mut self, label: &str, signer: &str, dev: &str, ps: &[&str]) {
        let p = Payload::Rename {
            device: self.key(dev),
            name: format!("Renamed {dev}"),
        };
        self.push(label, signer, ps, p);
    }
    fn rotate(&mut self, label: &str, signer: &str, epoch: u32, ps: &[&str]) {
        let p = Payload::Rotate {
            epoch,
            secret_commit: secret_commit(epoch, &arr32(&format!("secret-{label}"))),
        };
        self.push(label, signer, ps, p);
    }
    /// Marks the last change as not well formed.
    fn refused(&mut self, why: &'static str) {
        self.changes.last_mut().unwrap().2 = Some(why);
    }
    /// Adds a raw change (not well formed by parse or signature).
    fn raw(
        &mut self,
        label: &str,
        body_bytes: Vec<u8>,
        sig: [u8; 64],
        body: Body,
        why: &'static str,
    ) {
        let c = Change::from_raw_unchecked(body_bytes, sig, body);
        self.ids.insert(label.to_string(), *c.id());
        self.changes.push((label.to_string(), c, Some(why)));
    }
}

struct ViewExp {
    have: Vec<&'static str>,
    effective: Vec<&'static str>,
    valid: Vec<&'static str>,
    members: Vec<&'static str>,
    removed: Vec<&'static str>,
    current: &'static str,
}

#[allow(clippy::too_many_arguments)]
fn view(
    have: &[&'static str],
    effective: &[&'static str],
    valid: &[&'static str],
    members: &[&'static str],
    removed: &[&'static str],
    current: &'static str,
) -> ViewExp {
    ViewExp {
        have: have.to_vec(),
        effective: effective.to_vec(),
        valid: valid.to_vec(),
        members: members.to_vec(),
        removed: removed.to_vec(),
        current,
    }
}

fn scenario_json(name: &str, note: &str, sc: &Sc, views: Vec<ViewExp>) -> Value {
    let devices: BTreeMap<&str, Value> = sc
        .keys
        .iter()
        .map(|(n, k)| {
            (
                *n,
                json!({ "seed": hx(&k.signing_key().to_bytes()), "key": hx(&k.public()) }),
            )
        })
        .collect();
    let changes: Vec<Value> = sc
        .changes
        .iter()
        .map(|(l, c, why)| {
            let mut v = json!({ "label": l, "body": hx(c.body_bytes()), "sig": hx(c.signature()),
                                "id": hx(c.id()) });
            if let Some(w) = why {
                v["well_formed"] = json!(false);
                v["why"] = json!(w);
            }
            v
        })
        .collect();
    let views: Vec<Value> = views
        .into_iter()
        .map(|v| {
            let well_formed = v
                .have
                .iter()
                .all(|h| sc.changes.iter().any(|(l, _, w)| l == h && w.is_none()));
            fn sorted(x: &[&'static str]) -> Vec<&'static str> {
                let s: BTreeSet<&'static str> = x.iter().copied().collect();
                s.into_iter().collect()
            }
            json!({ "have": v.have, "expect": {
                "well_formed": well_formed,
                "effective_removes": sorted(&v.effective), "valid": sorted(&v.valid),
                "members": sorted(&v.members), "removed": sorted(&v.removed),
                "current_secret_change": v.current } })
        })
        .collect();
    let v = json!({ "name": name, "note": note, "devices": devices, "changes": changes, "views": views });
    check_membership_vector(&v, 3);
    v
}

fn gen_membership() -> Value {
    let mut out = Vec::new();
    let base = |sc: &mut Sc| {
        sc.genesis("g", "A");
        sc.add("addB", "A", "B", &["g"]);
        sc.add("addC", "A", "C", &["addB"]);
    };

    // 5.5 line 6 and the example of 6.3.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.remove("rmB_byA", "A", "B", &["addC"]);
    sc.remove("rmA_byB", "B", "A", &["addC"]);
    out.push(scenario_json(
        "mutual-remove",
        "A and B remove each other at the same time: both are removed, C stays",
        &sc,
        vec![
            view(
                &["g", "addB", "addC", "rmB_byA", "rmA_byB"],
                &["rmA_byB", "rmB_byA"],
                &["g", "addB", "addC"],
                &["C"],
                &["A", "B"],
                "g",
            ),
            view(
                &["g", "addB", "addC", "rmB_byA"],
                &["rmB_byA"],
                &["g", "addB", "addC"],
                &["A", "C"],
                &["B"],
                "g",
            ),
        ],
    ));

    // 5.6 row 1: removed X signs an add later; offline member S (B) first without the removal.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addX", "A", "X", &["addC"]);
    sc.remove("rmX", "A", "X", &["addX"]);
    sc.add("addY_byX", "X", "Y", &["rmX"]);
    out.push(scenario_json(
        "removed-adds-later",
        "X is removed, then signs add Y on top of the removal: Y is never a member",
        &sc,
        vec![view(
            &["g", "addB", "addC", "addX", "rmX", "addY_byX"],
            &["rmX"],
            &["g", "addB", "addC", "addX"],
            &["A", "B", "C"],
            &["X"],
            "g",
        )],
    ));

    // 5.6 rows 1 and 2, 5.5 line 4: back-dated add, seen before and after the removal.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addX", "A", "X", &["addC"]);
    sc.remove("rmX", "A", "X", &["addX"]);
    sc.add("addY_byX", "X", "Y", &["addX"]);
    out.push(scenario_json(
        "backdated-add",
        "X is removed and signs add Y with parents from before the removal. A member that has not seen the removal accepts Y until it gets the removal",
        &sc,
        vec![
            view(&["g", "addB", "addC", "addX", "addY_byX"], &[],
                 &["g", "addB", "addC", "addX", "addY_byX"], &["A", "B", "C", "X", "Y"], &[], "g"),
            view(&["g", "addB", "addC", "addX", "addY_byX", "rmX"], &["rmX"],
                 &["g", "addB", "addC", "addX"], &["A", "B", "C"], &["X"], "g"),
        ],
    ));

    // 5.6 row 3.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addW", "A", "W", &["addC"]);
    sc.add("addX", "A", "X", &["addW"]);
    sc.remove("rmX", "A", "X", &["addX"]);
    sc.remove("rmW_byX", "X", "W", &["addX"]);
    out.push(scenario_json(
        "backdated-remove",
        "X is removed and signs remove W with parents from before its removal: W is removed too, X gains nothing",
        &sc,
        vec![
            view(&["g", "addB", "addC", "addW", "addX", "rmW_byX"], &["rmW_byX"],
                 &["g", "addB", "addC", "addW", "addX"], &["A", "B", "C", "X"], &["W"], "g"),
            view(&["g", "addB", "addC", "addW", "addX", "rmW_byX", "rmX"], &["rmW_byX", "rmX"],
                 &["g", "addB", "addC", "addW", "addX"], &["A", "B", "C"], &["W", "X"], "g"),
        ],
    ));

    // 5.6 row 4, 5.5 line 5.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addX", "A", "X", &["addC"]);
    sc.remove("rmX_byA", "A", "X", &["addX"]);
    sc.remove("rmA_byX", "X", "A", &["addX"]);
    out.push(scenario_json(
        "stolen-removes-remover",
        "a stolen device X removes the device A that removed it: both are removed, the others stay (removal wins)",
        &sc,
        vec![view(&["g", "addB", "addC", "addX", "rmX_byA", "rmA_byX"], &["rmA_byX", "rmX_byA"],
                  &["g", "addB", "addC", "addX"], &["B", "C"], &["A", "X"], "g")],
    ));

    // 5.6 row 5, 5.5 line 7 (first half).
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addX", "A", "X", &["addC"]);
    sc.remove("rmA_byX", "X", "A", &["addX"]);
    sc.remove("rmB_byX", "X", "B", &["rmA_byX"]);
    sc.remove("rmC_byX", "X", "C", &["rmB_byX"]);
    out.push(scenario_json(
        "remove-all-others",
        "a stolen device removes all others before anyone removes it: all others are removed",
        &sc,
        vec![view(
            &["g", "addB", "addC", "addX", "rmA_byX", "rmB_byX", "rmC_byX"],
            &["rmA_byX", "rmB_byX", "rmC_byX"],
            &["g", "addB", "addC", "addX"],
            &["X"],
            &["A", "B", "C"],
            "g",
        )],
    ));

    // 5.5 line 7 (second half).
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addX", "A", "X", &["addC"]);
    sc.remove("rmA_byX", "X", "A", &["addX"]);
    sc.remove("rmB_byX", "X", "B", &["rmA_byX"]);
    sc.remove("rmC_byX", "X", "C", &["rmB_byX"]);
    sc.remove("rmX_byB", "B", "X", &["addX"]);
    out.push(scenario_json(
        "remove-all-and-removed",
        "X removes all others while B removes X at the same moment: X is removed too",
        &sc,
        vec![view(
            &[
                "g", "addB", "addC", "addX", "rmA_byX", "rmB_byX", "rmC_byX", "rmX_byB",
            ],
            &["rmA_byX", "rmB_byX", "rmC_byX", "rmX_byB"],
            &["g", "addB", "addC", "addX"],
            &[],
            &["A", "B", "C", "X"],
            "g",
        )],
    ));

    // 5.6 row 6.
    let mut sc = Sc::new();
    sc.genesis("g", "A");
    sc.add("addB", "A", "B", &["g"]);
    sc.remove("rmB_byA", "A", "B", &["addB"]);
    sc.add("addC_byB", "B", "C", &["addB"]);
    out.push(scenario_json(
        "remove-while-adding",
        "A removes B while B adds C at the same time: C is not a member",
        &sc,
        vec![view(
            &["g", "addB", "rmB_byA", "addC_byB"],
            &["rmB_byA"],
            &["g", "addB"],
            &["A"],
            &["B"],
            "g",
        )],
    ));

    // 5.6 row 7.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addX", "A", "X", &["addC"]);
    sc.remove("rmX_byA", "A", "X", &["addX"]);
    sc.remove("rmX_byB", "B", "X", &["addX"]);
    out.push(scenario_json(
        "double-remove",
        "A and B both remove X at the same time: X is removed, both removals are effective",
        &sc,
        vec![view(
            &["g", "addB", "addC", "addX", "rmX_byA", "rmX_byB"],
            &["rmX_byA", "rmX_byB"],
            &["g", "addB", "addC", "addX"],
            &["A", "B", "C"],
            &["X"],
            "g",
        )],
    ));

    // 5.5 line 8.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.remove("rmC_byC", "C", "C", &["addC"]);
    out.push(scenario_json(
        "leave",
        "a device removes itself (leaves the group)",
        &sc,
        vec![view(
            &["g", "addB", "addC", "rmC_byC"],
            &["rmC_byC"],
            &["g", "addB", "addC"],
            &["A", "B"],
            &["C"],
            "g",
        )],
    ));

    // 5.5 line 9.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addC_again", "B", "C", &["addC"]);
    out.push(scenario_json(
        "double-add",
        "a second add of a key that is already a member changes nothing",
        &sc,
        vec![view(
            &["g", "addB", "addC", "addC_again"],
            &[],
            &["g", "addB", "addC", "addC_again"],
            &["A", "B", "C"],
            &[],
            "g",
        )],
    ));

    // 5.5 line 2: a removed key that is added again stays removed.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.remove("rmC", "A", "C", &["addC"]);
    sc.add("addC_again", "B", "C", &["rmC"]);
    out.push(scenario_json(
        "re-add-removed",
        "a removed key that is added again stays removed",
        &sc,
        vec![view(
            &["g", "addB", "addC", "rmC", "addC_again"],
            &["rmC"],
            &["g", "addB", "addC", "addC_again"],
            &["A", "B"],
            &["C"],
            "g",
        )],
    ));

    // 3-way ring.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addW", "A", "W", &["addC"]);
    sc.remove("rmB_byA", "A", "B", &["addW"]);
    sc.remove("rmC_byB", "B", "C", &["addW"]);
    sc.remove("rmA_byC", "C", "A", &["addW"]);
    out.push(scenario_json(
        "removal-ring",
        "A removes B, B removes C, C removes A, all at once: all three are removed, W stays",
        &sc,
        vec![view(
            &["g", "addB", "addC", "addW", "rmB_byA", "rmC_byB", "rmA_byC"],
            &["rmA_byC", "rmB_byA", "rmC_byB"],
            &["g", "addB", "addC", "addW"],
            &["W"],
            &["A", "B", "C"],
            "g",
        )],
    ));

    // Back-dated add, then the added device signs a rename (invalid) and a remove (effective).
    let mut sc = Sc::new();
    base(&mut sc);
    sc.add("addW", "A", "W", &["addC"]);
    sc.add("addX", "A", "X", &["addW"]);
    sc.remove("rmX", "A", "X", &["addX"]);
    sc.add("addY_byX", "X", "Y", &["addX"]);
    sc.rename("renY_byY", "Y", "Y", &["addY_byX"]);
    sc.remove("rmW_byY", "Y", "W", &["addY_byX"]);
    out.push(scenario_json(
        "backdated-child",
        "Y was added by a back-dated add of removed X. Y's rename is invalid; Y's remove of W is effective (5.6 row 3)",
        &sc,
        vec![view(&["g", "addB", "addC", "addW", "addX", "rmX", "addY_byX", "renY_byY", "rmW_byY"],
                  &["rmW_byY", "rmX"], &["g", "addB", "addC", "addW", "addX"],
                  &["A", "B", "C"], &["W", "X"], "g")],
    ));

    // Rotates: remover rotates, and the tie-break of two rotates of one epoch.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.remove("rmC", "A", "C", &["addC"]);
    sc.rotate("rotA", "A", 1, &["rmC"]);
    sc.rotate("rotB", "B", 1, &["rmC"]);
    let (ra, rb) = (sc.ids["rotA"], sc.ids["rotB"]);
    let winner = if ra > rb { "rotA" } else { "rotB" };
    out.push(scenario_json(
        "rotate-tie-break",
        "two members rotate to epoch 1 at once: the one with the higher change ID is the current secret change",
        &sc,
        vec![
            view(&["g", "addB", "addC", "rmC"], &["rmC"], &["g", "addB", "addC"], &["A", "B"],
                 &["C"], "g"),
            view(&["g", "addB", "addC", "rmC", "rotA"], &["rmC"], &["g", "addB", "addC", "rotA"],
                 &["A", "B"], &["C"], "rotA"),
            view(&["g", "addB", "addC", "rmC", "rotA", "rotB"], &["rmC"],
                 &["g", "addB", "addC", "rotA", "rotB"], &["A", "B"], &["C"], winner),
        ],
    ));

    // A rotate by a removed device is invalid.
    let mut sc = Sc::new();
    base(&mut sc);
    sc.remove("rmC", "A", "C", &["addC"]);
    sc.rotate("rotC", "C", 1, &["addC"]);
    out.push(scenario_json(
        "rotate-by-removed",
        "a removed device signs a back-dated rotate: it is not valid, the secret stays at the genesis",
        &sc,
        vec![view(&["g", "addB", "addC", "rmC", "rotC"], &["rmC"], &["g", "addB", "addC"],
                  &["A", "B"], &["C"], "g")],
    ));

    // Changes that are not well formed.
    let mut sc = Sc::new();
    base(&mut sc);
    // Bad signature.
    let kb = sc.key("B");
    let c = sc.push("badsig", "A", &["addC"], Payload::Remove { device: kb });
    sc.changes.pop();
    let mut sig = *c.signature();
    sig[5] ^= 1;
    sc.raw(
        "badsig",
        c.body_bytes().to_vec(),
        sig,
        c.body().clone(),
        "bad signature",
    );
    // Parents not sorted.
    let mut ps = sc.parents(&["addB", "addC"]);
    ps.reverse();
    let body = Body {
        signer: sc.key("A"),
        time: 1,
        parents: ps,
        payload: Payload::Remove {
            device: sc.key("B"),
        },
    };
    let bb = body.encode();
    let sig = {
        use ed25519_dalek::Signer;
        let mut m = b"uclip-change-sig/1".to_vec();
        m.extend_from_slice(&bb);
        sc.keys["A"].signing_key().sign(&m).to_bytes()
    };
    sc.raw("unsorted", bb, sig, body, "parents not sorted");
    // Second genesis.
    sc.genesis("g2", "B");
    sc.refused("second genesis");
    // Signer never added.
    sc.add("addY_byW", "W", "Y", &["addC"]);
    sc.refused("signer never added");
    // Rotate with a wrong epoch.
    sc.rotate("rot2", "A", 2, &["addC"]);
    sc.refused("rotate epoch is not 1 plus the highest epoch in its past");
    out.push(scenario_json(
        "not-well-formed",
        "changes that are refused and never stored: bad signature, parents not sorted, second genesis, signer never added, rotate with a wrong epoch",
        &sc,
        vec![view(&["g", "addB", "addC", "badsig", "unsorted", "g2", "addY_byW", "rot2"], &[],
                  &["g", "addB", "addC"], &["A", "B", "C"], &[], "g")],
    ));

    json!({ "format": 1, "vectors": out })
}

/// Shuffles with a simple seeded generator.
fn shuffle<T>(v: &mut [T], seed: u64) {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for i in (1..v.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        v.swap(i, (s % (i as u64 + 1)) as usize);
    }
}

fn check_membership_vector(v: &Value, orders: u64) {
    let name = v["name"].as_str().unwrap();
    let devs: BTreeMap<String, [u8; 32]> = v["devices"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(n, d)| {
            let seed: [u8; 32] = unhex(d["seed"].as_str().unwrap()).try_into().unwrap();
            let key = DeviceKey::from_seed(&seed).public();
            assert_eq!(hx(&key), d["key"].as_str().unwrap());
            (n.clone(), key)
        })
        .collect();
    let dev_label = |k: &[u8; 32]| devs.iter().find(|(_, v)| *v == k).unwrap().0.clone();
    let mut by_label: HashMap<String, (Vec<u8>, bool)> = HashMap::new();
    let mut label_of_id: HashMap<[u8; 32], String> = HashMap::new();
    let mut group_id = None;
    for c in v["changes"].as_array().unwrap() {
        let label = c["label"].as_str().unwrap().to_string();
        let mut stored = unhex(c["body"].as_str().unwrap());
        let id = uc_pair::chain::change_id(&stored);
        assert_eq!(hx(&id), c["id"].as_str().unwrap(), "{name}/{label} id");
        stored.extend_from_slice(&unhex(c["sig"].as_str().unwrap()));
        let wf = c
            .get("well_formed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if label == "g" {
            group_id = Some(id);
        }
        label_of_id.insert(id, label.clone());
        by_label.insert(label, (stored, wf));
    }
    let group_id = group_id.expect("genesis labelled g");
    for (vi, view) in v["views"].as_array().unwrap().iter().enumerate() {
        let have: Vec<String> = view["have"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap().to_string())
            .collect();
        let e = &view["expect"];
        let labels = |k: &str| -> BTreeSet<String> {
            e[k].as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        let mut first = None;
        for o in 0..orders {
            let mut order = have.clone();
            shuffle(&mut order, o * 7919 + vi as u64);
            let mut g = Graph::with_group_id(group_id);
            let mut all_ok = true;
            for l in &order {
                let (bytes, wf) = &by_label[l];
                let r = g.insert_bytes(bytes);
                if r.is_err() {
                    all_ok = false;
                    assert!(!wf, "{name}: {l} must be accepted, got {r:?}");
                }
            }
            for l in &have {
                let (bytes, wf) = &by_label[l];
                let id = uc_pair::chain::change_id(&bytes[..bytes.len() - 64]);
                assert_eq!(g.contains(&id), *wf, "{name}: {l} stored?");
                if !wf {
                    all_ok = false;
                }
            }
            assert_eq!(g.pool_len(), 0, "{name}: pool not empty");
            assert_eq!(
                all_ok,
                e["well_formed"].as_bool().unwrap(),
                "{name}: well_formed"
            );
            let ev = g.evaluate();
            let got = (
                ev.effective_removes
                    .iter()
                    .map(|i| label_of_id[i].clone())
                    .collect::<BTreeSet<_>>(),
                ev.valid
                    .iter()
                    .map(|i| label_of_id[i].clone())
                    .collect::<BTreeSet<_>>(),
                ev.members.iter().map(&dev_label).collect::<BTreeSet<_>>(),
                ev.removed.iter().map(&dev_label).collect::<BTreeSet<_>>(),
                label_of_id[&ev.current_secret_change.unwrap()].clone(),
            );
            assert_eq!(
                got.0,
                labels("effective_removes"),
                "{name} view {vi}: effective"
            );
            assert_eq!(got.1, labels("valid"), "{name} view {vi}: valid");
            assert_eq!(got.2, labels("members"), "{name} view {vi}: members");
            assert_eq!(got.3, labels("removed"), "{name} view {vi}: removed");
            assert_eq!(
                got.4,
                e["current_secret_change"].as_str().unwrap(),
                "{name} view {vi}: current secret"
            );
            match &first {
                None => first = Some(got),
                Some(f) => assert_eq!(f, &got, "{name}: result depends on order"),
            }
        }
    }
}

// ---------------------------------------------------------------- files

fn write(name: &str, v: &Value) {
    let dir = vectors_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    std::fs::write(dir.join(name), s).unwrap();
}

fn read(name: &str) -> Value {
    let s = std::fs::read_to_string(vectors_dir().join(name))
        .unwrap_or_else(|_| panic!("{name} missing: run gen_vectors"));
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["format"], 1);
    v
}

#[test]
#[ignore]
fn gen_vectors() {
    write("pairing-v1.json", &gen_pairing());
    write("qr-v1.json", &gen_qr());
    write("membership-v1.json", &gen_membership());
}

#[test]
fn pairing_vectors() {
    let f = read("pairing-v1.json");
    let vs = f["vectors"].as_array().unwrap();
    let names: BTreeSet<&str> = vs.iter().map(|v| v["name"].as_str().unwrap()).collect();
    for n in [
        "code-ok",
        "qr-ok",
        "code-wrong",
        "mitm-keys",
        "mitm-exporter",
        "mode-mismatch",
        "code-leading-zeros",
        "low-order-msg-b",
        "wrong-side-byte",
    ] {
        assert!(names.contains(n), "missing vector {n}");
    }
    for v in vs {
        assert_eq!(run_pair(v), v["expect"], "{}", v["name"]);
    }
}

#[test]
fn qr_vectors() {
    let f = read("qr-v1.json");
    let vs = f["vectors"].as_array().unwrap();
    assert!(vs.len() >= 12);
    for v in vs {
        check_qr_vector(v);
    }
}

#[test]
fn membership_vectors() {
    let f = read("membership-v1.json");
    for v in f["vectors"].as_array().unwrap() {
        check_membership_vector(v, 20);
    }
}

#[test]
fn vectors_match_generator() {
    // The files only change together with protocol.md. If this fails, the code changed a value.
    assert_eq!(read("pairing-v1.json"), gen_pairing());
    assert_eq!(read("qr-v1.json"), gen_qr());
    assert_eq!(read("membership-v1.json"), gen_membership());
}
