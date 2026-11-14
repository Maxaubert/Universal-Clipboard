# Universal Clipboard: pairing and membership protocol, version 1

Status: **signed off** in milestone M0c on 2026-11-14 by the planning model (Fable), after a paper
review of plan sections 3.6 and 3.7. Milestone M3 implements this page word for word. If M3 finds
that a rule here cannot be built or is wrong, M3 stops and changes this page in its own PR first.
The code never drifts from the page silently.

This page replaces the protocol detail in sections 3.6 and 3.7 of
`docs/plan/2026-11-13-spec-and-plan.md`. Where the plan and this page differ, this page wins.

Words: "MUST" is a rule that a test checks. The **inviter** is the device that shows the code.
The **joiner** is the new device that types the code or scans the QR code.

Contents: 1 review result, 2 decisions, 3 building blocks, 4 pairing, 5 membership,
6 test vectors, 7 what M3 must test, 8 sources.

---

## 1. Review result: errors found in 3.6 and 3.7, and the fix for each

| # | Where | Error or gap in the plan text | Fix on this page |
|---|---|---|---|
| F1 | 3.6 | No message order was fixed. If the joiner sent its confirmation first, a fake inviter on the LAN could test one guess against the joiner. The joiner "tries each pairing it sees", so 1,000 fake pairings would give 1,000 guesses with no limit. | The inviter confirms first. The joiner sends nothing that depends on the code until it has checked the inviter's confirmation (4.4). A fake inviter then learns nothing at all. |
| F2 | 3.6 | "A try" was not defined exactly. | A try is counted at the moment the inviter sends its reply, because that reply is the only message a guesser can test (4.6). |
| F3 | 3.6 | Two secrets (6 digits and the QR secret) but one protocol: the inviter could not know which password to use. | The first message carries a mode byte. The mode is in the transcript. Each mode has its own limit (4.4, 4.6). |
| F4 | 3.6 | The transcript listed "device keys" and "QUIC endpoint keys" as different things. With iroh they are the same key. The text did not say where a side gets the other side's key. | The connection MUST use the device key as its TLS identity. Each side reads the peer key from the TLS layer, never from a message. A TLS exporter value binds this one connection (4.5). |
| F5 | 3.6 | A "session key" was derived but never used. | Removed. The group state travels on the same QUIC connection, which the transcript binds (4.5). |
| F6 | 3.6 | QR path: no rule for what the joiner checks, and no rule against a pairing link from a web page. | The joiner checks the TLS key against the key in the QR before it sends anything. Payloads are accepted only from the in-app scanner. No URL scheme is registered (4.3). |
| F7 | 3.6 | The `spake2` crate does not reject low-order points, and it is not the RFC 9382 wire format. | Our wrapper rejects low-order points. The page says plainly which SPAKE2 variant this is (2.1, 4.4). |
| F8 | 3.6 | A pairing that breaks after the inviter signed "add" left the joiner half paired. | Pairing a key that is already a member sends the state again and signs nothing new (4.7). |
| F9 | 3.6 | An attacker gets 5 new guesses each time the user makes a new code. | After 15 failed code tries in a row, counted over all pairings, the short code is off for 1 hour. The QR code still works (4.6). |
| F10 | 3.7 | **"Two devices remove each other: the older device wins."** A removed device can always sign a removal with old parents, so it looks like "the same moment". A stolen device that joined early could cancel its own removal for ever. | **Removal always wins.** A removal counts if its signer was a member in the removal's own past. Nothing that happens beside it can cancel it. Two devices that remove each other are both removed (5.4, 5.6). |
| F11 | 3.7 | The validity rule was circular: whether a removal counts depended on other removals that depended on it. Two devices could reach two different, both "correct", results. | A rule in two layers that always ends and has one result (5.4). |
| F12 | 3.7 | Could a removed key be added again? Not said. | No. A removed key is dead in that group for ever. A removed device makes a new key (5.4, 5.7). |
| F13 | 3.7 | After a removal "those members make a new group secret": several members would each make a different secret for the same epoch. | Only the remover makes it. A signed `rotate` change holds a commitment to the secret. A fixed tie-break picks one if two exist (5.8). |
| F14 | 3.7 | A device wiped itself when it got a "removed" message. Anyone could send that. | A device acts only on its own evaluation of signed changes (5.7). |
| F15 | 3.7 | No exact bytes for what is signed, no change ID rule, no group identity. | Byte layouts, signature input, change ID, genesis and group ID are fixed (5.1, 5.2). |
| F16 | 3.7 | Order on a new connection was not fixed, so a clip could be sent before the newest removal was known. | Membership sync runs first on every connection. Clips flow only after both sides evaluated the result (5.9). |

Costs that stay, stated honestly (also in 5.6):

- A removed device that still reaches a member which has not yet seen the removal can make other
  devices drop out of the group (they must pair again). It can never get back in, never add a
  device that the others accept, and never get a clip from a device that knows the removal.
- If device A removes device B at the same time as B adds a new device C, then C is not a member.
  The user pairs C again.

---

## 2. The five decisions

### 2.1 PAKE: SPAKE2 with the RustCrypto `spake2` crate, pinned to `=0.4.0`

Facts, checked on 2026-11-14:

| | SPAKE2 | CPace |
|---|---|---|
| Standard | **RFC 9382**, September 2023, Informational, a product of the IRTF Crypto Forum (CFRG). The RFC says SPAKE2 was not the pick of the CFRG PAKE contest, and that it fits apps that cannot hash onto a curve at run time. | **draft-irtf-cfrg-cpace-21** (2026-04-23). CFRG picked CPace as its balanced PAKE in 2020. The draft is "Sent to the RFC Editor". It is **not an RFC yet**. |
| Rust crate | `spake2` 0.4.0 (2023-07-28), RustCrypto organisation. About 1.9 million downloads, about 710,000 recent. `0.5.0-pre.0` came out on 2026-01-25, so it is maintained. **Never audited.** Has `start_a_with_rng` and `start_b_with_rng`, so fixed test vectors are possible. | `pake-cpace` 0.1.7 (2023-12-15), one maintainer, about 55,000 recent downloads. It uses its own labels (`CPaceRistretto255-1`) and its own key hash, so by reading the source it does not follow the current draft or its test vectors. It has no way to inject the random values, so fixed test vectors are not possible. `cpace` 0.1.0 is from 2020 and dead. |
| Setup | Needs two fixed points M and N with unknown relation. The crate uses the python-spake2 constants. | Needs no fixed points. |

Decision: **SPAKE2**. Reason: the plan's rule was "CPace only if a maintained crate exists". None
does. Writing CPace ourselves would mean our own PAKE core on a draft. The `spake2` crate is the
most used Rust PAKE and runs in magic-wormhole.

Honest limits of this choice:

- The crate implements the **python-spake2 variant** on the Ed25519 group. It is **not** the
  RFC 9382 wire format: other M and N constants, another key hash. So the RFC 9382 test vectors do
  not apply. Our vectors (section 6) are our own.
- The crate has no audit. Our code around it (transcript, confirmation, limits) gets the Opus
  security review in M3.
- Version `0.4.0` pulls older majors of `curve25519-dalek` (4), `sha2` (0.10) and `hkdf` (0.12).
  `deny.toml` only warns on that. Do not move to `0.5.0-pre.0`; it is a pre-release.

Review again when CPace is an RFC **and** a maintained crate passes the RFC vectors. A swap changes
only step 4.4. It needs a new ALPN (`uclip-pair/2`), because the two are not compatible.

### 2.2 Both device keys and the connection are bound into the transcript, three times

1. The pairing connection uses each device's Ed25519 device key as its TLS identity.
2. Both device keys are the SPAKE2 identities.
3. Both device keys and a TLS exporter value are in the confirmation transcript (4.5).

A man in the middle must show its own key to each side. Then the identities, the exporter and the
transcript all differ, and the confirmation fails.

### 2.3 Limits

| Limit | Value |
|---|---|
| Lifetime of one open pairing (code and QR) | 10 minutes |
| Tries for the 6-digit code | 5, counted when the inviter sends its reply |
| Tries for the QR secret | 20 per source address (only to bound work; guessing 128 bits is not possible) |
| Open pairings per device | 1 |
| Pairing connections handled at the same time | 4 |
| Time allowed for each protocol step | 10 seconds |
| Failed code tries in a row, over all pairings, before the short code is turned off | 15, then off for 1 hour |

### 2.4 QR payload

The QR code carries the inviter's device key, a 128-bit secret, the inviter's port and up to 4 LAN
addresses (4.3). The joiner needs no discovery and no typing. It checks the inviter's key before
it sends anything, so a fake inviter gets nothing.

### 2.5 Membership: removal always wins

See F10 to F13 above and section 5.

---

## 3. Building blocks

| Purpose | Choice |
|---|---|
| PAKE | `spake2` `=0.4.0`, `Spake2::<Ed25519Group>`, asymmetric mode (`start_a`, `start_b`) |
| Hash | SHA-256 (`sha2`) |
| KDF | HKDF-SHA256 (`hkdf`), RFC 5869 |
| MAC | HMAC-SHA256 (`hmac`), compared in constant time (`subtle` or the `hmac` verify call) |
| Signature | Ed25519 (`ed25519-dalek`), checked with `verify_strict` |
| Random | the OS random generator, through `rand_core::OsRng` or `getrandom` |

Notation:

- `||` joins byte strings.
- `u8`, `u16`, `u32`, `u64` are unsigned integers, **big-endian**.
- `LP(x)` is `u32(length of x) || x`.
- Text in quotes is ASCII bytes with no end byte.
- A key is always the 32 raw bytes of an Ed25519 public key.

Secrets (code, QR secret, SPAKE2 state, confirmation keys, group secret) are wiped from memory
after use (`zeroize`). They are never written to logs.

---

## 4. Pairing

### 4.1 What the user's 6 digits can and cannot do

A 6-digit code has only 1,000,000 values. It is safe **only** because of three things together:

1. **The PAKE.** Someone who records a whole pairing gets nothing that lets them test codes later.
   Without a PAKE (for example the code used as a MAC key), one recorded pairing would let them
   test all 1,000,000 codes on their own computer in under a second.
2. **One guess per run, and only against the inviter.** An attacker who takes part in a run can
   test exactly one code, and only when it plays the joiner. An attacker who plays the inviter
   learns nothing (F1).
3. **The try limit.** The inviter allows 5 tries per code. So the chance that a guesser gets in is
   at most 5 in 1,000,000, which is 1 in 200,000, per code. Without the limit, 1,000,000 runs
   would take only a few hours.

Take any one of the three away and the code is not safe. In addition the code lives 10 minutes,
works once, and can be tried only from the same local network.

### 4.2 Opening a pairing (inviter)

1. The inviter has a group. If it has none, it creates one now (genesis change, 5.2).
2. If the group has 8 members, the inviter refuses and the app says so.
3. The inviter makes:
   - `code`: 6 decimal digits, `000000` to `999999`, uniform. Draw a random `u32`; if it is
     4,294,000,000 or more, draw again; else take it modulo 1,000,000.
   - `qr_secret`: 16 random bytes.
   - `pairing_tag`: 8 random bytes. It is not derived from the code or the secret.
4. It accepts ALPN `uclip-pair/1` and advertises the mDNS service `_uclip-pair._udp.local` on each
   physical interface. The instance name is `pairing_tag` as 16 lowercase hex characters. The only
   TXT entry is `v=1`. The port is the agent's UDP port.
5. The pairing closes after 10 minutes, on success, or when the user cancels. When it closes, the
   ALPN is no longer accepted, the mDNS service is removed, and both secrets are wiped.
6. If the short code is turned off (4.6), the inviter makes no `code` and shows only the QR code.

When no pairing is open, the device does not accept `uclip-pair/1` at all.

### 4.3 QR payload

Bytes:

| Field | Size | Content |
|---|---|---|
| `version` | 1 | `0x01` |
| `inviter_key` | 32 | the inviter's device key |
| `qr_secret` | 16 | |
| `port` | 2 | `u16`, the inviter's UDP port |
| `n` | 1 | number of addresses, 1 to 4 |
| `n` times `addr` | 5 or 17 each | `u8` family (`0x04` or `0x06`), then 4 or 16 address bytes |

Addresses: only addresses of physical interfaces that pass the LAN rule (4.8). IPv4 first.

The QR text is `UCLIP1:` followed by the bytes in base32 (RFC 4648 alphabet, upper case, no
padding). This fits the QR alphanumeric mode. Error correction level M.

Joiner rules:

- The parser is strict: exact lengths, known version, known families, no bytes left over,
  1 to 4 addresses. Anything else is refused.
- The payload is accepted **only from the in-app scanner on the "add this device" screen.** The
  app registers no URL scheme and takes no pairing payload from a link, a file, the clipboard or
  another app. Reason: a web page must not be able to make a device join a stranger's group.
- The joiner dials each address that passes the LAN rule. An IPv6 link-local address is tried on
  each physical interface. If no address answers, the joiner browses `_uclip-pair._udp.local` and
  dials each instance.
- After the QUIC handshake the joiner compares the TLS peer key with `inviter_key`. If they
  differ, it closes the connection and **sends nothing**.

### 4.4 Messages

The joiner is the QUIC client. It opens one two-way stream. Every message is a frame:
`u8 type || u32 length || payload`. The length limit is 256 bytes, except `PairState` (1 MiB).
A frame that is too long, an unknown type, a wrong payload size or a message out of order ends the
pairing with `PairAbort`.

| Type | Name | From | Payload |
|---|---|---|---|
| `0x01` | `PairHello` | joiner | `u8 version = 1`, `u8 mode` (`0x01` code, `0x02` QR), `msg_b` (33 bytes) |
| `0x02` | `PairReply` | inviter | `msg_a` (33 bytes), `conf_a` (32 bytes) |
| `0x03` | `PairConfirm` | joiner | `conf_b` (32 bytes), `u8 device_type`, `u8 name_len`, `name` |
| `0x04` | `PairState` | inviter | see 4.7 |
| `0x05` | `PairDone` | joiner | empty |
| `0x7F` | `PairAbort` | either | `u8 reason`: 1 version, 2 no open pairing or code dead, 3 busy, 4 bad message, 5 confirmation failed, 6 group full, 7 other |

SPAKE2 inputs (the same on both sides):

- Password, code mode: `"uclip-pair/1 code:" || the 6 ASCII digits`.
- Password, QR mode: `"uclip-pair/1 qr:" || qr_secret`.
- `idA` = the inviter's device key. `idB` = the joiner's device key.
- The inviter is side A (`start_a`). The joiner is side B (`start_b`).
- Each side reads the **other** side's key from the TLS layer of this connection
  (`Connection::remote_id()` in iroh, the peer certificate key in plain `quinn`).
- Every connection gets a fresh SPAKE2 state. A state is never used twice.

`msg_a` and `msg_b` are the 33-byte messages of the crate (1 side byte, 32 point bytes). Before a
side gives the peer's message to `finish`, it MUST check: length 33, the side byte is the peer's
(`0x41` "A" from the inviter, `0x42` "B" from the joiner), the 32 bytes decompress to a point
(`CompressedEdwardsY::decompress`), and the point is not of low order (`is_small_order()` is
false). If a check fails, the run fails (for the inviter this is a counted try).

`K` is the 32-byte output of `finish`.

Flow:

```
joiner (B)                                             inviter (A)
QUIC connect, ALPN uclip-pair/1  --------------------> LAN rule (4.8), else close
[QR mode: check TLS key == inviter_key, else close]
start_b -> msg_b
PairHello(version, mode, msg_b)  --------------------> check version, mode, pairing open,
                                                       tries left for this mode (4.6)
                                                       start_a -> msg_a ; K = finish(msg_b)
                                                       compute T, kc_a, kc_b, conf_a (4.5)
                                                       count one try (4.6)
                                 <-------------------- PairReply(msg_a, conf_a)
K = finish(msg_a)
compute T, kc_a, kc_b
check conf_a, else close and send NOTHING more
PairConfirm(conf_b, device record) ------------------> check conf_b, else PairAbort(5)
                                                       sign "add B" if needed (4.7)
                                 <-------------------- PairState
check PairState (4.7), store
PairDone  -------------------------------------------> store, close pairing, spread the change
```

The joiner MUST NOT send any byte that depends on `K` before it has checked `conf_a`. If `conf_a`
is wrong, the joiner closes the connection without `PairAbort` and without `PairConfirm`.

Code mode, finding the inviter: the joiner browses `_uclip-pair._udp.local` on each physical
interface and runs the flow with each instance it sees: at most 8 at the same time and at most 32
in total, for at most 20 seconds. The first run with a correct `conf_a` wins; the joiner closes the
others. This is safe because a run with a wrong or fake inviter tells that inviter nothing. Honest
cost: if two people on one network pair at the same moment, each joiner uses one try of the other
inviter.

### 4.5 Transcript and confirmation

```
exporter = TLS exporter of this connection, label "EXPORTER-uclip-pair-1", empty context, 32 bytes
           (export_keying_material in iroh and quinn)

TT = LP("uclip-pair/1") || LP(u8 version) || LP(u8 mode)
  || LP(inviter_key) || LP(joiner_key) || LP(exporter)
  || LP(msg_b) || LP(msg_a)

T    = SHA-256(TT)
PRK  = HKDF-Extract(salt = T, input key = K)
kc_a = HKDF-Expand(PRK, info = "uclip-pair/1 confirm inviter", 32 bytes)
kc_b = HKDF-Expand(PRK, info = "uclip-pair/1 confirm joiner", 32 bytes)
conf_a = HMAC-SHA256(kc_a, T)
conf_b = HMAC-SHA256(kc_b, T)
```

- `inviter_key` in `TT` is: on the inviter its own key, on the joiner the TLS peer key.
  `joiner_key` the other way round. The roles are bound by the position in `TT`, by the SPAKE2
  side, and by the two different confirmation keys.
- The password is not in `TT`. It is in `K` (the crate hashes the password, both identities, both
  messages and the shared point into `K`).
- No session key is derived. After both confirmations, `PairState` goes over the same QUIC
  connection. That connection is the one bound by `exporter` and by both keys.

### 4.6 Limits and counting (inviter)

- `tries_code` and `tries_qr` start at 0 for each open pairing.
- When a `PairHello` arrives: if the mode is code and (`tries_code` is 5 or the short code is off),
  or the mode is QR and `tries_qr` for the remote address of that connection is 20, the inviter
  answers `PairAbort(2)`. The QR count is kept per remote IP address, so a device on the LAN
  cannot use up the QR tries of the real joiner. At most 1,024 addresses are tracked per pairing;
  a QR hello from a new address after that gets `PairAbort(2)`.
- **The inviter adds 1 to the counter of that mode just before it sends `PairReply`.** The check
  and the add are one atomic step, so 4 connections at once cannot pass the limit.
- A run that does not end with a correct `conf_b` within 10 seconds stays counted. That covers a
  wrong MAC, a dropped connection, a timeout and a bad message.
- When `tries_code` reaches 5 and that run fails, the code is dead. The QR code stays valid until
  the 10 minutes end. The inviter shows the notice "Someone else tried this code". The inviter
  cannot know who made a failed try, so the notice appears after every failed try.
- A correct `conf_b` ends the pairing for both modes. All other pairing connections are closed.
- More than 4 pairing connections at once: the extra ones get `PairAbort(3)`. This is not a try.
- A connection that sends no `PairHello` within 10 seconds is closed. This is not a try.
- **Lockout.** The device counts failed code tries over all pairings (not dead codes: an attacker
  who stops at 4 tries per code must still be counted). A failed try counts even when its pairing
  was cancelled, expired or replaced by a new one before the try ended. A correct `conf_b` resets
  the count. At 15 the short code is off on this device for 1 hour: no further code try is
  answered, and a new pairing shows only the QR code and the text "Someone on this network keeps
  guessing the code. Use the QR code, or try again in an hour." The count and the time are kept in
  memory only; an agent restart clears them. When the lockout starts, the count goes back to 0.
- **Denial of service.** A device on the same LAN can always stop a pairing: it can use up the 5
  code tries, start the lockout, or hold the 4 pairing connections. This is accepted. It reveals
  nothing, the user sees the notice, and the per-address QR count means the QR code cannot be
  killed with a few junk hellos.

Result: per open pairing a guesser has at most 5 in 1,000,000. Before each 1-hour lockout a
guesser has at most 15 tries, so over time at most about 15 in 1,000,000 per hour, and only while
the user keeps pressing "Add a device".

### 4.7 After confirmation

`PairConfirm` carries the joiner's device record: `device_type` (0 unknown, 1 Windows, 2 macOS,
3 Linux, 4 iPhone, 5 iPad, 6 Android phone, 7 Android tablet) and `name` (1 to 64 bytes of UTF-8,
no control characters). A `device_type` above 7 is refused, here and in changes (5.2): the
parsers are strict.

The inviter, after it checked `conf_b`:

1. If the joiner's key is removed in this group (5.4): `PairAbort(7)`. (A removed device makes a
   new key, so an honest device never gets here.)
2. If the joiner's key is already a member: sign nothing.
3. Else, if the group has 8 members: `PairAbort(6)`.
4. Else sign an `add` change for the joiner's key (5.2) with the current heads as parents.
5. Send `PairState`:

| Field | Size | Content |
|---|---|---|
| `epoch` | 4 | epoch of the current group secret (5.8) |
| `group_secret` | 32 | |
| `n_changes` | 2 | |
| `n_changes` times | `u16 length`, then `body || signature` | the whole graph, in canonical order (5.3) |
| `addr_len` | 2 | at most 4,096 |
| `addr_book` | `addr_len` | remembered addresses of the members, in the format that `uc-net` defines (`docs/net.md`: the members' signed address announcements) |

6. Wait up to 10 seconds for `PairDone`. Then, with or without it, keep the change and spread it
   to the connected members. (If the joiner did not store the state, the user pairs again and
   rule 2 applies.)

The joiner, when it gets `PairState`, MUST check all of this before it stores anything:

- Every change is well formed (5.2) and the first one is the only genesis.
- In the evaluated graph (5.4) the inviter's TLS key is a member and the joiner's own key is a
  member.
- `SHA-256` commitment of `group_secret` and `epoch` equals the commitment in the current
  secret change (5.8).

If a check fails, the joiner stores nothing and shows an error. Else it stores the graph, the
secret and the addresses, sends `PairDone`, and starts normal sync.

Before pairing, a joiner that is already in a group with other members asks "Leave your other
devices and join these?". On yes it signs a `remove` of itself in the old group, spreads it if a
member is reachable, and wipes the old group state. A joiner whose group has only itself drops that
group without a question. The joiner keeps its device key in both cases.

### 4.8 The LAN rule

The inviter accepts a pairing connection only if the remote address is:

- IPv4 inside `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` or `169.254.0.0/16`, **and** inside
  the subnet of an address on one of the inviter's physical interfaces; or
- IPv6 link-local (`fe80::/10`) on a physical interface, or unique-local (`fc00::/7`) inside an
  on-link prefix of a physical interface.

Everything else is closed before `PairHello` is read. The joiner applies the same rule to every
address it dials (from mDNS or from the QR code). "Physical interface" is the definition of plan
section 3.5.

Honest limit: during the 10 minutes, any device on the LAN that connects to the pairing ALPN sees
the inviter's device key in the TLS handshake. That key is a stable ID. The sync port shows the
same key to anyone who connects, so pairing adds nothing new.

---

## 5. Membership

### 5.1 Model

- A device has one Ed25519 key pair. The public key is its ID.
- A group is a set of signed **changes**. Each change names its parent changes by ID, so the set
  is a hash-linked graph. "X is in the past of Y" means X can be reached from Y through parents.
- Every device computes the members from the graph with the rule in 5.4. The rule is a pure
  function of the set of changes. Two devices with the same set get the same result, in whatever
  order the changes arrived.

### 5.2 Change format

`body`:

| Field | Size | Content |
|---|---|---|
| `version` | 1 | `0x01` |
| `kind` | 1 | 0 genesis, 1 add, 2 remove, 3 rename, 4 rotate |
| `signer` | 32 | key of the device that signs |
| `time` | 8 | `u64` Unix seconds. For display only. No rule uses it. |
| `n_parents` | 1 | 0 for genesis, else 1 to 8 |
| `parents` | 32 each | change IDs, sorted ascending as bytes, no doubles |
| payload | | by kind, below |

Payload by kind (`name` is `u8 length || UTF-8`, 1 to 64 bytes, no control characters):

| Kind | Payload |
|---|---|
| genesis | `group_nonce` (16 random bytes), `u8 device_type`, `name` (of the signer), `secret_commit` (32) for epoch 0 |
| add | `device` (32), `u8 device_type`, `name` |
| remove | `device` (32) |
| rename | `device` (32), `name` |
| rotate | `u32 epoch`, `secret_commit` (32) |

```
change ID     = SHA-256("uclip-change-id/1" || body)
signature     = Ed25519 signature by signer over ("uclip-change-sig/1" || body)
stored / sent = body || signature (64 bytes)
group ID      = change ID of the genesis
secret_commit = SHA-256("uclip-secret-commit/1" || u32 epoch || group_secret)
```

A change is **well formed** if all of this holds. A change that is not well formed is dropped and
never stored or passed on:

- It parses exactly: known version and kind, sizes right, no bytes left over, parents sorted.
- The signature passes `verify_strict`.
- Genesis: no parents. Every other kind: 1 to 8 parents, and every parent is already in the graph.
  (A change whose parents are missing waits in a pool of at most 256 changes.)
- The device already has a genesis (its group ID). Any other genesis is refused.
- The `signer` is named by a genesis or an `add` in the past of the change. (This is a cheap
  filter against junk. It is not the validity rule.)
- Rotate: `epoch` equals 1 plus the highest epoch of any rotate or genesis in its past (the
  genesis has epoch 0).

When a device signs a change, the parents are all its current heads (changes that no other change
names as a parent). If there are more than 8 heads, it takes the 8 with the lowest IDs.

### 5.3 Canonical order

`depth` of the genesis is 0. `depth` of any other change is 1 plus the highest depth of its
parents. The **canonical order** sorts changes by `depth`, then by change ID as bytes, both
ascending. A parent always comes before its child.

### 5.4 The validity rule

Let a **view** be a set of well-formed changes that contains the genesis and, with each change,
all of its past. The whole graph of a device is a view. The past of one change is a view.

**Layer 1: which removals count.** A `remove` change `r` signed by `Z` is **effective** if and
only if `Z` is a member of the view "the past of `r`" (computed with this same rule, on that
smaller view).

- This looks only at the past of `r`. So it has one fixed answer for ever, on every device, and
  nothing that arrives later can change it.
- It ends, because the past of `r` is always smaller than the view that contains `r`.

**Layer 2: which other changes count, in a view V.** Go through the genesis, add, rename and
rotate changes of V in canonical order. The genesis is valid. Another change `c` signed by `X` is
**valid in V** if and only if both hold:

- (a) the past of `c` holds a change that is valid in V and names `X` as a device: the genesis
  (its signer) or an `add` of `X`; and
- (b) every effective `remove` of `X` in V has `c` in its past.

**Result.**

- **Removed keys of V**: every key that an effective `remove` in V names.
- **Members of V**: every key named by the genesis or by an `add` that is valid in V, minus the
  removed keys.

A device's members are the members of its whole graph. Implementation note: compute "effective"
once per `remove`, in canonical order, and keep the answer (it never changes).

### 5.5 What follows from the rule (each line is a property test)

1. **Same set, same result.** The result does not depend on arrival order.
2. **Removed stays removed.** Once a device holds an effective `remove` of key X, X is never a
   member on that device again, whatever arrives later. A removed key cannot be added again.
3. **Invalid stays invalid.** A change that is not valid in a view is not valid in any larger view.
4. **No back-dating.** If X is removed, every add, rename and rotate signed by X that is not in the
   past of **every** effective `remove` of X is invalid. That covers changes X signs after the
   removal and changes X signs with old parents. A device that only such a change added is not a
   member. Adds, renames and rotates signed by such a device are invalid too (rule a).
5. **Removal wins.** A `remove` is effective if its signer was a member in the removal's own past.
   A removal of the signer that sits beside it (not in its past) does not cancel it.
6. **Two devices remove each other at the same time: both are removed.**
7. **A device removes all others:** all the others are removed. If one of them removed that device
   in the same moment, that device is removed too.
8. A device may remove itself (leaving a group).
9. A second `add` of a key that is already a member changes nothing.

### 5.6 Cases, with the result

| Case | Result |
|---|---|
| X is removed. Later X signs "add Y" and gives it to a member S that was offline. | S accepts Y only until S gets the removal. Then Y is not a member on any device. |
| X is removed. X signs "add Y" with parents from before the removal. | Same: invalid on every device that holds the removal. |
| X is removed. X signs "remove W" with parents from before its removal and gets it to a member that has not seen the removal. | **W is removed too.** This is the price of "removal wins". X gains no access. W shows "Another device removed this one" and is paired again. |
| A stolen old device removes the device that removed it. | Both are removed. The other members stay. (With the plan's old rule the stolen device would have stayed.) |
| A stolen device removes all the others before anyone removes it. | As in the plan: it gains nothing, the others stop sending to it, each shows "Pair my devices again". |
| A removes B while B, at the same time, adds C. | C is not a member (rule b). The user pairs C again. |
| A and B both remove X at the same time. | X is removed. Both removals are effective. |
| A member was offline during a removal. | It still accepts the removed device until it next talks to any member that knows. 5.9 makes that the first thing on every connection. |

### 5.7 What a device does with the result

After every change to its graph a device evaluates again and then:

- closes every sync connection to a key that is not a member;
- refuses new sync connections from removed keys before any data is read;
- if **its own key** is no longer a member (removed, or its `add` became invalid): it stops all
  sync, wipes the group state **and its device key**, makes a new device key, and shows "Another
  device removed this one" with the button "Pair my devices again".

A device does this only from its own evaluation of well-formed changes. A bare "you were removed"
message without the changes has no effect. The `Removed` message of plan section 3.8 is only a
hint that is sent after the changes.

Before a device signs a `remove`, it syncs membership with every member it can reach (at most
3 seconds), so that the removal's past is as new as possible. That keeps the "A removes B while B
adds C" case rare.

Limits: a device opens no pairing when the group has 8 members. A device signs no add or rename
when its graph holds 1,024 changes (the app then says "Set up your devices again"). It accepts and
signs `remove` and `rotate` up to 2,048 changes and refuses every change after that. "Signs no add or
rename" also means "accepts no add or rename": from 1,024 changes on, an incoming add or rename is
refused too.

### 5.8 Group secret

The group secret is 32 random bytes with an epoch number. It is used only for the discovery tags
(plan section 3.4). It protects no clip.

- The genesis commits to the secret of epoch 0.
- **The device that signs a `remove` also makes the new secret.** In the same step it signs a
  `rotate` change (epoch + 1, the commitment of the new secret) whose parent is the `remove`.
- The **current secret change** is, among the genesis and all valid `rotate` changes, the one with
  the highest epoch; if several have that epoch, the one with the highest change ID.
- The secret itself is never in a change. A member gets it from another member over the sync
  connection, after membership sync (5.9), with two new messages on `uclip/1`:
  `WantSecret { epoch }` and `Secret { epoch, group_secret }`. The receiver checks the secret
  against the commitment of the current secret change and drops it if it does not match. A member
  sends the secret only to a key that is a member in its own evaluation.
- If a member sees an effective `remove` that no valid `rotate` follows, it waits a random time of
  30 to 60 seconds. If that is still so, it signs the `rotate` itself. Two members may do this at
  once; the tie-break above picks one.
- Discovery: a device advertises with the newest secret it holds. It recognises tags of the
  current epoch and of the epoch before, so a member that missed a rotation is still found and
  gets the new secret.

### 5.9 Sync connections: order and admission

On every `uclip/1` connection, in this order:

1. Both sides send `Hello` (plan 3.8). It carries the group ID and the chain heads.
2. If the group ID is not the receiver's group ID: close.
3. If the peer's TLS key is a removed key: close.
4. **Membership sync.** Each side sends the changes the other lacks, parents first (canonical
   order). Both evaluate again (5.4, 5.7).
5. If the peer's key is now a member: go on to `WantSecret` and `Secret`, addresses and clips.
   If not: close.

Until step 5 passes, a device sends **no clip, no secret and no address**, and reads none.

A peer whose key the device does not know yet (for example a device that was added while this one
was offline) is allowed steps 1 to 4 only. Limits for such a peer: one at a time, at most 256
changes and 64 KiB, 10 seconds.

---

## 6. Test vectors

Vectors are JSON files in `crates/uc-pair/tests/vectors/`. All byte strings are lowercase hex with
no separators. Each file is `{ "format": 1, "vectors": [ ... ] }`. Each vector has a `name` and a
`note` in plain English. M3 writes the first values with a generator test
(`cargo test -p uc-pair -- --ignored gen_vectors`), commits them, and from then on the tests only
read them. A vector file changes only together with this page.

### 6.1 `pairing-v1.json`

```json
{
  "name": "code-ok",
  "note": "6-digit code, both sides honest",
  "mode": 1,
  "code": "418207",
  "qr_secret": null,
  "inviter_key": "<32 bytes>",
  "joiner_key": "<32 bytes>",
  "inviter_sees_joiner_key": "<32 bytes>",
  "joiner_sees_inviter_key": "<32 bytes>",
  "inviter_exporter": "<32 bytes>",
  "joiner_exporter": "<32 bytes>",
  "inviter_code": "418207",
  "joiner_code": "418207",
  "inviter_rng": "<64 bytes>",
  "joiner_rng": "<64 bytes>",
  "tamper_msg_b": null,
  "expect": {
    "msg_b": "<33 bytes>",
    "msg_a": "<33 bytes>",
    "inviter": { "k": "<32>", "tt": "<bytes>", "t": "<32>", "kc_a": "<32>", "kc_b": "<32>", "conf_a": "<32>", "conf_b": "<32>" },
    "joiner":  { "k": "<32>", "tt": "<bytes>", "t": "<32>", "kc_a": "<32>", "kc_b": "<32>", "conf_a": "<32>", "conf_b": "<32>" },
    "result": "ok"
  }
}
```

- `inviter_rng` and `joiner_rng` are the bytes that a test random generator hands to
  `start_a_with_rng` and `start_b_with_rng`, in order. The test generator fails the test if the
  crate asks for more bytes than given.
- `inviter_sees_joiner_key`, `joiner_sees_inviter_key` and the two exporters let a vector model a
  man in the middle: in an honest run they equal the real keys and each other.
- `inviter_code` and `joiner_code` (or `qr_secret` on both sides in mode 2) let a vector model a
  wrong code. In mode 2 a non-null `joiner_code` means the joiner uses those 6 digits as its QR
  secret (password `"uclip-pair/1 qr:" || digits`); this is the `mode-mismatch` vector.
- `tamper_msg_b` (hex or `null`) is the message the inviter receives in place of the joiner's
  `msg_b`. It models a low-order point or a wrong side byte. `expect.msg_b` stays the joiner's own
  message.
- `result` is one of: `ok`, `joiner-rejects-conf-a`, `inviter-rejects-conf-b`, `bad-message`.
  When the result is not `ok`, the fields of a side that never computes them are `null`.

Required vectors: `code-ok`; `qr-ok`; `code-wrong` (joiner types another code:
`joiner-rejects-conf-a`); `mitm-keys` (each side sees a third key: `joiner-rejects-conf-a`);
`mitm-exporter` (keys equal, exporters differ: `joiner-rejects-conf-a`); `mode-mismatch` (joiner
uses the code as the QR secret with mode 2: `joiner-rejects-conf-a`); `code-leading-zeros` (`000042`); `low-order-msg-b` and
`wrong-side-byte` (`bad-message`).

### 6.2 `qr-v1.json`

```json
{ "name": "two-addresses", "note": "...",
  "inviter_key": "<32>", "qr_secret": "<16>", "port": 51820,
  "addrs": ["192.168.1.20", "fe80::1c2d:3eff:fe4f:5a6b"],
  "payload": "<bytes>", "text": "UCLIP1:....", "valid": true }
```

Vectors with `"valid": false` hold only `text` and `note`. Required: one IPv4; IPv4 plus IPv6;
4 addresses; and refused cases: wrong prefix, version 2, 0 addresses, 5 addresses, unknown family,
1 byte too many, 1 byte too few, lower-case base32, padding characters.

### 6.3 `membership-v1.json`

```json
{
  "name": "mutual-remove",
  "note": "A and B remove each other at the same time: both are removed, C stays",
  "devices": { "A": { "seed": "<32>", "key": "<32>" }, "B": { "...": "..." }, "C": { "...": "..." } },
  "changes": [
    { "label": "g",  "body": "<bytes>", "sig": "<64>", "id": "<32>" },
    { "label": "addB", "body": "<bytes>", "sig": "<64>", "id": "<32>" }
  ],
  "views": [
    { "have": ["g", "addB", "addC", "rmB_byA", "rmA_byB"],
      "expect": { "well_formed": true,
                  "effective_removes": ["rmA_byB", "rmB_byA"],
                  "valid": ["g", "addB", "addC"],
                  "members": ["C"], "removed": ["A", "B"],
                  "current_secret_change": "g" } }
  ]
}
```

- `seed` is the 32-byte Ed25519 secret seed of a test device, so the generator can sign again.
- A vector may hold several `views`: the same changes with different `have` sets. That tests a
  device that has not yet seen a removal, and then has.
- The test feeds the changes of `have` in at least 20 random orders and expects the same result.
- A change that must be refused as not well formed has `"well_formed": false` and a `why` in its
  entry in `changes`. In a view, `expect.well_formed` is true only if every change in `have` is
  stored. A view's expected results ignore the refused changes.
- A device knows its group ID before it gets any change, so a test graph is created with the ID
  of the change labelled `g`. Changes whose parents are missing wait in the pool until they come.

Required vectors, one for each row of 5.6 and each line of 5.5, and also: bad signature; parents
not sorted; second genesis; signer never added; rotate with a wrong epoch; two rotates with the
same epoch (tie-break); a removed key that is added again (stays removed); a 3-way removal ring
(A removes B, B removes C, C removes A, all at once: all three are removed); a device added by a
back-dated add that then signs a rename (invalid) and a remove (effective, see 5.6 row 3).

---

## 7. What M3 must test (adds to plan 6.1)

Pairing: every vector of 6.1 and 6.2; the 5-try limit with 4 connections at once (never more than
5 `PairReply` frames per code); a dropped connection after `PairReply` stays counted; a fake
inviter gets no byte after a wrong `conf_a`; the QR joiner sends nothing to a wrong key; QR stays
valid after the code is dead; lockout after 15 failed code tries in a row, also when spread as 4
per code over many pairings and when the pairing was cancelled; QR tries counted per address;
both secrets wiped after success; expiry at 10 minutes; a pairing from a
non-local address is refused; no `uclip-pair/1` when no pairing is open; pairing an existing member
signs nothing; the joiner refuses a `PairState` in which it is not a member or the secret does not
match.

Membership: every vector of 6.3; property tests for lines 1 to 4 of 5.5 with random graphs of up to
8 devices and 60 changes; the device wipes itself only from its own evaluation; no clip is sent
before step 5 of 5.9.

---

## 8. Sources (checked 2026-11-14)

- RFC 9382, "SPAKE2, a Password-Authenticated Key Exchange", September 2023, Informational:
  <https://www.rfc-editor.org/rfc/rfc9382>
- draft-irtf-cfrg-cpace-21, "CPace, a balanced composable PAKE", 2026-04-23, state "Sent to the
  RFC Editor": <https://datatracker.ietf.org/doc/draft-irtf-cfrg-cpace/>
- `spake2` 0.4.0 source (tag `spake2-v0.4.0`) and README: <https://github.com/RustCrypto/PAKEs>.
  Read: `start_a_with_rng`, the key hash `SHA-256(SHA-256(pw) || SHA-256(idA) || SHA-256(idB) || X || Y || K)`,
  the python-spake2 M and N constants, no low-order check.
- `pake-cpace` 0.1.7 source: <https://github.com/jedisct1/rust-cpace>. Read, not run.
- crates.io download numbers and dates for `spake2`, `pake-cpace`, `cpace`.
- iroh 1.4.1 `Connection`: `remote_id` and `export_keying_material` exist (docs.rs).
- RFC 5869 (HKDF), RFC 5705 and RFC 8446 section 7.5 (TLS exporters), RFC 4648 (base32).

Not checked here: spike S1 (iroh or plain QUIC) has no result yet. This page works with both,
because both give the peer key and a TLS exporter.
