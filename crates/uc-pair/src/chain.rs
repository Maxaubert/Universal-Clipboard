//! The membership chain (protocol section 5).
//!
//! A group is a set of signed changes that name their parents by ID. [`Graph`] stores the
//! well-formed changes of one group and evaluates the members with the two-layer validity rule
//! of section 5.4. The evaluation is a pure function of the set of changes.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use ed25519_dalek::{Signature, Signer, VerifyingKey};
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};
use uc_keys::DeviceKey;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// An Ed25519 public key: a device ID.
pub type Key = [u8; 32];
/// A change ID (protocol 5.2).
pub type ChangeId = [u8; 32];

/// Highest number of parents a change may name.
pub const MAX_PARENTS: usize = 8;
/// Largest number of changes that wait for missing parents.
pub const POOL_LIMIT: usize = 256;
/// From this many changes on, no add or rename is signed or accepted.
pub const SOFT_LIMIT: usize = 1024;
/// From this many changes on, no change at all is signed or accepted.
pub const HARD_LIMIT: usize = 2048;
/// Highest number of members in a group.
pub const MAX_MEMBERS: usize = 8;

const ID_LABEL: &[u8] = b"uclip-change-id/1";
const SIG_LABEL: &[u8] = b"uclip-change-sig/1";
const COMMIT_LABEL: &[u8] = b"uclip-secret-commit/1";

/// Device types (protocol 4.7).
pub const DEVICE_TYPE_MAX: u8 = 7;

/// The kind byte of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Genesis = 0,
    Add = 1,
    Remove = 2,
    Rename = 3,
    Rotate = 4,
}

/// The payload of a change, by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    Genesis {
        group_nonce: [u8; 16],
        device_type: u8,
        name: String,
        secret_commit: [u8; 32],
    },
    Add {
        device: Key,
        device_type: u8,
        name: String,
    },
    Remove {
        device: Key,
    },
    Rename {
        device: Key,
        name: String,
    },
    Rotate {
        epoch: u32,
        secret_commit: [u8; 32],
    },
}

impl Payload {
    pub fn kind(&self) -> Kind {
        match self {
            Payload::Genesis { .. } => Kind::Genesis,
            Payload::Add { .. } => Kind::Add,
            Payload::Remove { .. } => Kind::Remove,
            Payload::Rename { .. } => Kind::Rename,
            Payload::Rotate { .. } => Kind::Rotate,
        }
    }
}

/// The signed part of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub signer: Key,
    /// Unix seconds, for display only.
    pub time: u64,
    /// Sorted ascending, no doubles.
    pub parents: Vec<ChangeId>,
    pub payload: Payload,
}

/// Why a change was refused (not well formed, or over a limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeError {
    /// Bad version, kind, size, name, device type, parent order or count, or bytes left over.
    Parse,
    /// The signature does not pass `verify_strict`.
    Signature,
    /// A genesis that is not the group's genesis.
    SecondGenesis,
    /// The signer is not named by a genesis or an add in the change's past.
    UnknownSigner,
    /// A rotate whose epoch is not 1 plus the highest epoch in its past.
    Epoch,
    /// The graph is too big for this change (5.7).
    Limit,
    /// The pool of changes that wait for parents is full.
    PoolFull,
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ChangeError {}

/// Checks a device name: 1 to 64 bytes of UTF-8, no control characters.
pub fn check_name(b: &[u8]) -> Option<&str> {
    if b.is_empty() || b.len() > 64 {
        return None;
    }
    let s = std::str::from_utf8(b).ok()?;
    if s.chars().any(char::is_control) {
        return None;
    }
    Some(s)
}

/// `SHA-256("uclip-secret-commit/1" || u32 epoch || group_secret)`.
pub fn secret_commit(epoch: u32, secret: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(COMMIT_LABEL);
    h.update(epoch.to_be_bytes());
    h.update(secret);
    h.finalize().into()
}

/// A group secret with its epoch (5.8). Wiped on drop.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct GroupSecret {
    pub epoch: u32,
    pub secret: [u8; 32],
}

impl GroupSecret {
    pub fn random<R: RngCore + CryptoRng>(epoch: u32, rng: &mut R) -> Self {
        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);
        Self { epoch, secret }
    }

    pub fn commit(&self) -> [u8; 32] {
        secret_commit(self.epoch, &self.secret)
    }
}

impl fmt::Debug for GroupSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GroupSecret(epoch {})", self.epoch)
    }
}

struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ChangeError> {
        if self.b.len() < n {
            return Err(ChangeError::Parse);
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, ChangeError> {
        Ok(self.take(1)?[0])
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], ChangeError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    fn name(&mut self) -> Result<String, ChangeError> {
        let n = self.u8()? as usize;
        let b = self.take(n)?;
        check_name(b).map(str::to_string).ok_or(ChangeError::Parse)
    }
    fn device_type(&mut self) -> Result<u8, ChangeError> {
        let t = self.u8()?;
        if t > DEVICE_TYPE_MAX {
            return Err(ChangeError::Parse);
        }
        Ok(t)
    }
}

impl Body {
    /// Encodes the body bytes (protocol 5.2). Panics if a name is not valid; build bodies only
    /// with checked names.
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(128);
        v.push(1);
        v.push(self.payload.kind() as u8);
        v.extend_from_slice(&self.signer);
        v.extend_from_slice(&self.time.to_be_bytes());
        v.push(self.parents.len() as u8);
        for p in &self.parents {
            v.extend_from_slice(p);
        }
        fn name(v: &mut Vec<u8>, n: &str) {
            assert!(check_name(n.as_bytes()).is_some(), "bad name");
            v.push(n.len() as u8);
            v.extend_from_slice(n.as_bytes());
        }
        match &self.payload {
            Payload::Genesis {
                group_nonce,
                device_type,
                name: n,
                secret_commit,
            } => {
                v.extend_from_slice(group_nonce);
                v.push(*device_type);
                name(&mut v, n);
                v.extend_from_slice(secret_commit);
            }
            Payload::Add {
                device,
                device_type,
                name: n,
            } => {
                v.extend_from_slice(device);
                v.push(*device_type);
                name(&mut v, n);
            }
            Payload::Remove { device } => v.extend_from_slice(device),
            Payload::Rename { device, name: n } => {
                v.extend_from_slice(device);
                name(&mut v, n);
            }
            Payload::Rotate {
                epoch,
                secret_commit,
            } => {
                v.extend_from_slice(&epoch.to_be_bytes());
                v.extend_from_slice(secret_commit);
            }
        }
        v
    }

    /// Strict parser: known version and kind, exact sizes, sorted parents, nothing left over.
    pub fn parse(b: &[u8]) -> Result<Body, ChangeError> {
        let mut r = Reader { b };
        if r.u8()? != 1 {
            return Err(ChangeError::Parse);
        }
        let kind = r.u8()?;
        let signer = r.arr::<32>()?;
        let time = u64::from_be_bytes(r.arr::<8>()?);
        let n = r.u8()? as usize;
        if kind == Kind::Genesis as u8 {
            if n != 0 {
                return Err(ChangeError::Parse);
            }
        } else if n == 0 || n > MAX_PARENTS {
            return Err(ChangeError::Parse);
        }
        let mut parents: Vec<ChangeId> = Vec::with_capacity(n);
        for _ in 0..n {
            let p = r.arr::<32>()?;
            if parents.last().is_some_and(|l| *l >= p) {
                return Err(ChangeError::Parse);
            }
            parents.push(p);
        }
        let payload = match kind {
            0 => Payload::Genesis {
                group_nonce: r.arr()?,
                device_type: r.device_type()?,
                name: r.name()?,
                secret_commit: r.arr()?,
            },
            1 => Payload::Add {
                device: r.arr()?,
                device_type: r.device_type()?,
                name: r.name()?,
            },
            2 => Payload::Remove { device: r.arr()? },
            3 => Payload::Rename {
                device: r.arr()?,
                name: r.name()?,
            },
            4 => Payload::Rotate {
                epoch: u32::from_be_bytes(r.arr()?),
                secret_commit: r.arr()?,
            },
            _ => return Err(ChangeError::Parse),
        };
        if !r.b.is_empty() {
            return Err(ChangeError::Parse);
        }
        Ok(Body {
            signer,
            time,
            parents,
            payload,
        })
    }
}

/// `SHA-256("uclip-change-id/1" || body)`.
pub fn change_id(body: &[u8]) -> ChangeId {
    let mut h = Sha256::new();
    h.update(ID_LABEL);
    h.update(body);
    h.finalize().into()
}

/// A parsed change whose signature has been checked.
#[derive(Clone, PartialEq, Eq)]
pub struct Change {
    body: Body,
    body_bytes: Vec<u8>,
    sig: [u8; 64],
    id: ChangeId,
}

impl fmt::Debug for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Change({:?} ", self.body.payload.kind())?;
        for b in &self.id[..4] {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl Change {
    /// Signs a body. Parents are sorted and doubles removed first.
    pub fn sign(mut body: Body, key: &DeviceKey) -> Change {
        body.parents.sort();
        body.parents.dedup();
        assert_eq!(body.signer, key.public(), "signer must be the signing key");
        let body_bytes = body.encode();
        let sig = key.signing_key().sign(&sig_input(&body_bytes)).to_bytes();
        let id = change_id(&body_bytes);
        Change {
            body,
            body_bytes,
            sig,
            id,
        }
    }

    /// Builds a change from raw parts without any check. For test vectors of refused changes.
    pub fn from_raw_unchecked(body_bytes: Vec<u8>, sig: [u8; 64], body: Body) -> Change {
        let id = change_id(&body_bytes);
        Change {
            body,
            body_bytes,
            sig,
            id,
        }
    }

    /// Parses `body || signature` and checks the signature with `verify_strict`.
    pub fn parse(stored: &[u8]) -> Result<Change, ChangeError> {
        if stored.len() < 64 {
            return Err(ChangeError::Parse);
        }
        let (bb, sb) = stored.split_at(stored.len() - 64);
        let body = Body::parse(bb)?;
        let sig: [u8; 64] = sb.try_into().map_err(|_| ChangeError::Parse)?;
        let vk = VerifyingKey::from_bytes(&body.signer).map_err(|_| ChangeError::Signature)?;
        vk.verify_strict(&sig_input(bb), &Signature::from_bytes(&sig))
            .map_err(|_| ChangeError::Signature)?;
        Ok(Change {
            body,
            body_bytes: bb.to_vec(),
            sig,
            id: change_id(bb),
        })
    }

    pub fn id(&self) -> &ChangeId {
        &self.id
    }
    pub fn body(&self) -> &Body {
        &self.body
    }
    pub fn body_bytes(&self) -> &[u8] {
        &self.body_bytes
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.sig
    }
    pub fn kind(&self) -> Kind {
        self.body.payload.kind()
    }
    pub fn signer(&self) -> &Key {
        &self.body.signer
    }
    /// `body || signature`, as stored and sent.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = self.body_bytes.clone();
        v.extend_from_slice(&self.sig);
        v
    }

    /// The device this change names as a member: the genesis signer or the added device.
    fn names_member(&self) -> Option<&Key> {
        match &self.body.payload {
            Payload::Genesis { .. } => Some(&self.body.signer),
            Payload::Add { device, .. } => Some(device),
            _ => None,
        }
    }

    fn removed_device(&self) -> Option<&Key> {
        match &self.body.payload {
            Payload::Remove { device } => Some(device),
            _ => None,
        }
    }

    fn epoch(&self) -> Option<u32> {
        match &self.body.payload {
            Payload::Genesis { .. } => Some(0),
            Payload::Rotate { epoch, .. } => Some(*epoch),
            _ => None,
        }
    }

    fn commit(&self) -> Option<&[u8; 32]> {
        match &self.body.payload {
            Payload::Genesis { secret_commit, .. } | Payload::Rotate { secret_commit, .. } => {
                Some(secret_commit)
            }
            _ => None,
        }
    }
}

fn sig_input(body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(SIG_LABEL.len() + body.len());
    v.extend_from_slice(SIG_LABEL);
    v.extend_from_slice(body);
    v
}

/// A set of change indexes.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Bits(Vec<u64>);

impl Bits {
    fn with_len(n: usize) -> Self {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: usize) {
        if self.0.len() <= i / 64 {
            self.0.resize(i / 64 + 1, 0);
        }
        self.0[i / 64] |= 1 << (i % 64);
    }
    fn has(&self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
    fn union(&mut self, o: &Bits) {
        if self.0.len() < o.0.len() {
            self.0.resize(o.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(&o.0) {
            *a |= b;
        }
    }
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(w, &bits)| {
            (0..64)
                .filter(move |b| bits & (1 << b) != 0)
                .map(move |b| w * 64 + b)
        })
    }
}

struct Node {
    change: Change,
    depth: u32,
    /// Strict ancestors (the change's past, without the change).
    past: Bits,
    /// For a remove: whether it is effective (layer 1). Never changes.
    effective: bool,
    has_child: bool,
}

/// The result of evaluating a view (5.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Evaluation {
    /// Members of the view.
    pub members: BTreeSet<Key>,
    /// Keys named by an effective remove.
    pub removed: BTreeSet<Key>,
    /// IDs of the genesis and the add, rename and rotate changes that are valid.
    pub valid: BTreeSet<ChangeId>,
    /// IDs of the effective removes.
    pub effective_removes: BTreeSet<ChangeId>,
    /// The current secret change (5.8).
    pub current_secret_change: Option<ChangeId>,
}

/// What happened when a change was offered to the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserted {
    /// Stored (and maybe some pooled changes after it).
    Stored,
    /// Already stored or already waiting.
    Duplicate,
    /// Parents are missing; it waits in the pool.
    Pooled,
}

/// The well-formed changes of one group, plus a pool of changes that wait for parents.
pub struct Graph {
    group_id: ChangeId,
    nodes: Vec<Node>,
    index: HashMap<ChangeId, usize>,
    /// Indexes in canonical order (5.3).
    order: Vec<usize>,
    pool: Vec<Change>,
}

impl fmt::Debug for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Graph({} changes, {} pooled)",
            self.nodes.len(),
            self.pool.len()
        )
    }
}

impl Graph {
    /// An empty graph that knows its group ID. Only the genesis with this ID is accepted.
    pub fn with_group_id(group_id: ChangeId) -> Graph {
        Graph {
            group_id,
            nodes: Vec::new(),
            index: HashMap::new(),
            order: Vec::new(),
            pool: Vec::new(),
        }
    }

    /// A graph that starts from a genesis change.
    pub fn from_genesis(genesis: Change) -> Result<Graph, ChangeError> {
        if genesis.kind() != Kind::Genesis {
            return Err(ChangeError::Parse);
        }
        let mut g = Graph::with_group_id(genesis.id);
        g.insert(genesis)?;
        Ok(g)
    }

    /// Creates a new group (4.2 step 1): signs a genesis and makes the epoch 0 secret.
    pub fn create<R: RngCore + CryptoRng>(
        key: &DeviceKey,
        device_type: u8,
        name: &str,
        time: u64,
        rng: &mut R,
    ) -> Result<(Graph, GroupSecret), ChangeError> {
        if check_name(name.as_bytes()).is_none() || device_type > DEVICE_TYPE_MAX {
            return Err(ChangeError::Parse);
        }
        let secret = GroupSecret::random(0, rng);
        let mut group_nonce = [0u8; 16];
        rng.fill_bytes(&mut group_nonce);
        let g = Change::sign(
            Body {
                signer: key.public(),
                time,
                parents: vec![],
                payload: Payload::Genesis {
                    group_nonce,
                    device_type,
                    name: name.to_string(),
                    secret_commit: secret.commit(),
                },
            },
            key,
        );
        Ok((Graph::from_genesis(g)?, secret))
    }

    /// The group ID: the change ID of the genesis.
    pub fn group_id(&self) -> &ChangeId {
        &self.group_id
    }

    /// Number of stored changes (not counting the pool).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn pool_len(&self) -> usize {
        self.pool.len()
    }

    pub fn contains(&self, id: &ChangeId) -> bool {
        self.index.contains_key(id)
    }

    pub fn get(&self, id: &ChangeId) -> Option<&Change> {
        self.index.get(id).map(|&i| &self.nodes[i].change)
    }

    /// All stored changes in canonical order (5.3).
    pub fn canonical(&self) -> impl Iterator<Item = &Change> {
        self.order.iter().map(|&i| &self.nodes[i].change)
    }

    /// The current heads: changes that no other change names as a parent. At most 8, the lowest
    /// IDs, sorted.
    pub fn heads(&self) -> Vec<ChangeId> {
        let mut h: Vec<ChangeId> = self
            .nodes
            .iter()
            .filter(|n| !n.has_child)
            .map(|n| n.change.id)
            .collect();
        h.sort();
        h.truncate(MAX_PARENTS);
        h
    }

    /// Changes of this graph that `known` does not hold, in canonical order (5.9 step 4).
    pub fn missing_from(&self, known: &dyn Fn(&ChangeId) -> bool) -> Vec<&Change> {
        self.canonical().filter(|c| !known(&c.id)).collect()
    }

    /// Offers a change. Well-formed changes are stored; a change with missing parents waits in
    /// the pool; anything else is refused and dropped.
    pub fn insert(&mut self, c: Change) -> Result<Inserted, ChangeError> {
        if self.index.contains_key(&c.id) || self.pool.iter().any(|p| p.id == c.id) {
            return Ok(Inserted::Duplicate);
        }
        let r = self.insert_one(c)?;
        if r == Inserted::Stored {
            self.drain_pool();
        }
        Ok(r)
    }

    /// Parses `body || signature` and offers it.
    pub fn insert_bytes(&mut self, stored: &[u8]) -> Result<Inserted, ChangeError> {
        self.insert(Change::parse(stored)?)
    }

    fn drain_pool(&mut self) {
        loop {
            let ready = self
                .pool
                .iter()
                .position(|c| c.body.parents.iter().all(|p| self.index.contains_key(p)));
            let Some(i) = ready else { break };
            let c = self.pool.swap_remove(i);
            // A pooled change that turns out not to be well formed is dropped.
            let _ = self.insert_one(c);
        }
    }

    fn insert_one(&mut self, c: Change) -> Result<Inserted, ChangeError> {
        let kind = c.kind();
        if kind == Kind::Genesis {
            if c.id != self.group_id || !self.nodes.is_empty() {
                return Err(ChangeError::SecondGenesis);
            }
            self.push(c, 0, Bits::default(), false);
            return Ok(Inserted::Stored);
        }
        let n = self.nodes.len();
        if n >= HARD_LIMIT || (matches!(kind, Kind::Add | Kind::Rename) && n >= SOFT_LIMIT) {
            return Err(ChangeError::Limit);
        }
        if self.nodes.is_empty() || !c.body.parents.iter().all(|p| self.index.contains_key(p)) {
            if self.pool.len() >= POOL_LIMIT {
                return Err(ChangeError::PoolFull);
            }
            self.pool.push(c);
            return Ok(Inserted::Pooled);
        }
        let mut past = Bits::with_len(n);
        let mut depth = 0;
        for p in &c.body.parents {
            let i = self.index[p];
            past.set(i);
            past.union(&self.nodes[i].past);
            depth = depth.max(self.nodes[i].depth + 1);
        }
        // The signer must be named by a genesis or an add in the past.
        let signer = c.body.signer;
        if !past
            .iter()
            .any(|i| self.nodes[i].change.names_member() == Some(&signer))
        {
            return Err(ChangeError::UnknownSigner);
        }
        if let Payload::Rotate { epoch, .. } = c.body.payload {
            let max = past
                .iter()
                .filter_map(|i| self.nodes[i].change.epoch())
                .max()
                .unwrap_or(0);
            if Some(epoch) != max.checked_add(1) {
                return Err(ChangeError::Epoch);
            }
        }
        let effective = if kind == Kind::Remove {
            // Layer 1: the signer is a member of the view "the past of r".
            self.evaluate_view(&past).members.contains(&signer)
        } else {
            false
        };
        self.push(c, depth, past, effective);
        Ok(Inserted::Stored)
    }

    fn push(&mut self, c: Change, depth: u32, past: Bits, effective: bool) {
        let i = self.nodes.len();
        for p in &c.body.parents {
            let pi = self.index[p];
            self.nodes[pi].has_child = true;
        }
        self.index.insert(c.id, i);
        self.nodes.push(Node {
            change: c,
            depth,
            past,
            effective,
            has_child: false,
        });
        let nodes = &self.nodes;
        let pos = self.order.partition_point(|&j| {
            (nodes[j].depth, nodes[j].change.id) < (depth, nodes[i].change.id)
        });
        self.order.insert(pos, i);
    }

    /// Evaluates the whole graph (5.4).
    pub fn evaluate(&self) -> Evaluation {
        let mut all = Bits::with_len(self.nodes.len());
        for i in 0..self.nodes.len() {
            all.set(i);
        }
        self.evaluate_view(&all)
    }

    /// Evaluates the view made of the given changes and all of their past.
    pub fn evaluate_ids(&self, ids: &[ChangeId]) -> Option<Evaluation> {
        let mut v = Bits::with_len(self.nodes.len());
        for id in ids {
            let i = *self.index.get(id)?;
            v.set(i);
            v.union(&self.nodes[i].past);
        }
        Some(self.evaluate_view(&v))
    }

    /// The two-layer rule on a view (a set closed under "past").
    fn evaluate_view(&self, view: &Bits) -> Evaluation {
        let in_view: Vec<usize> = self
            .order
            .iter()
            .copied()
            .filter(|&i| view.has(i))
            .collect();
        let mut effective: Vec<usize> = Vec::new();
        let mut removed = BTreeSet::new();
        for &i in &in_view {
            let n = &self.nodes[i];
            if n.effective
                && let Some(d) = n.change.removed_device()
            {
                effective.push(i);
                removed.insert(*d);
            }
        }
        let mut valid = Bits::with_len(self.nodes.len());
        let mut members = BTreeSet::new();
        let mut current: Option<(u32, ChangeId)> = None;
        for &i in &in_view {
            let n = &self.nodes[i];
            let c = &n.change;
            let ok = match c.kind() {
                Kind::Remove => continue,
                Kind::Genesis => true,
                _ => {
                    let x = &c.body.signer;
                    // (a) a valid change in the past names X.
                    let a = n
                        .past
                        .iter()
                        .any(|d| valid.has(d) && self.nodes[d].change.names_member() == Some(x));
                    // (b) every effective remove of X has c in its past.
                    let b = effective.iter().all(|&r| {
                        self.nodes[r].change.removed_device() != Some(x)
                            || self.nodes[r].past.has(i)
                    });
                    a && b
                }
            };
            if !ok {
                continue;
            }
            valid.set(i);
            if let Some(m) = c.names_member() {
                members.insert(*m);
            }
            if let Some(e) = c.epoch() {
                let cand = (e, c.id);
                if current.is_none_or(|cur| cand > cur) {
                    current = Some(cand);
                }
            }
        }
        for r in &removed {
            members.remove(r);
        }
        Evaluation {
            members,
            removed,
            valid: valid.iter().map(|i| self.nodes[i].change.id).collect(),
            effective_removes: effective.iter().map(|&i| self.nodes[i].change.id).collect(),
            current_secret_change: current.map(|(_, id)| id),
        }
    }

    /// Members of the whole graph.
    pub fn members(&self) -> BTreeSet<Key> {
        self.evaluate().members
    }

    pub fn is_member(&self, key: &Key) -> bool {
        self.members().contains(key)
    }

    /// True if an effective remove names this key. Such a key is dead in this group for ever.
    pub fn is_removed(&self, key: &Key) -> bool {
        self.nodes
            .iter()
            .any(|n| n.effective && n.change.removed_device() == Some(key))
    }

    /// The current secret change: its epoch and commitment.
    pub fn current_secret(&self) -> (u32, [u8; 32]) {
        let ev = self.evaluate();
        let id = ev
            .current_secret_change
            .expect("the genesis is always valid");
        let c = self.get(&id).expect("stored");
        (
            c.epoch().unwrap_or(0),
            *c.commit().expect("genesis or rotate"),
        )
    }

    /// Checks a received secret against the current secret change (5.8).
    pub fn secret_matches(&self, s: &GroupSecret) -> bool {
        let (epoch, commit) = self.current_secret();
        use subtle::ConstantTimeEq;
        s.epoch == epoch && bool::from(s.commit().ct_eq(&commit))
    }

    /// True if an effective remove exists that no valid rotate has in its past (5.8). The agent
    /// then waits 30 to 60 seconds and calls [`Graph::sign_rotate`] if it is still so.
    pub fn needs_rotate(&self) -> bool {
        let ev = self.evaluate();
        let rotates: Vec<usize> = ev
            .valid
            .iter()
            .map(|id| self.index[id])
            .filter(|&i| self.nodes[i].change.kind() == Kind::Rotate)
            .collect();
        ev.effective_removes.iter().any(|rid| {
            let r = self.index[rid];
            !rotates.iter().any(|&t| self.nodes[t].past.has(r))
        })
    }

    /// The name and device type of a member, after renames (the newest valid rename wins in
    /// canonical order).
    pub fn device_name(&self, key: &Key) -> Option<(u8, String)> {
        let ev = self.evaluate();
        let mut out = None;
        for c in self.canonical() {
            if !ev.valid.contains(&c.id) {
                continue;
            }
            match &c.body.payload {
                Payload::Genesis {
                    device_type, name, ..
                } if &c.body.signer == key => out = Some((*device_type, name.clone())),
                Payload::Add {
                    device,
                    device_type,
                    name,
                } if device == key => out = Some((*device_type, name.clone())),
                Payload::Rename { device, name } if device == key => {
                    if let Some((t, _)) = out {
                        out = Some((t, name.clone()));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn sign_and_insert(
        &mut self,
        key: &DeviceKey,
        time: u64,
        payload: Payload,
    ) -> Result<Change, ChangeError> {
        let c = Change::sign(
            Body {
                signer: key.public(),
                time,
                parents: self.heads(),
                payload,
            },
            key,
        );
        match self.insert(c.clone())? {
            Inserted::Stored => Ok(c),
            _ => Err(ChangeError::Parse),
        }
    }

    fn require_member(&self, key: &DeviceKey) -> Result<(), ChangeError> {
        if self.is_member(&key.public()) {
            Ok(())
        } else {
            Err(ChangeError::UnknownSigner)
        }
    }

    /// Signs an `add`. Returns `Ok(None)` if the key is already a member (nothing is signed),
    /// and an error if the key is removed, the group is full, or a limit is hit.
    pub fn sign_add(
        &mut self,
        key: &DeviceKey,
        device: Key,
        device_type: u8,
        name: &str,
        time: u64,
    ) -> Result<Option<Change>, AddError> {
        if self.is_removed(&device) {
            return Err(AddError::Removed);
        }
        let members = self.members();
        if members.contains(&device) {
            return Ok(None);
        }
        if members.len() >= MAX_MEMBERS {
            return Err(AddError::Full);
        }
        if check_name(name.as_bytes()).is_none() || device_type > DEVICE_TYPE_MAX {
            return Err(AddError::Change(ChangeError::Parse));
        }
        self.require_member(key).map_err(AddError::Change)?;
        self.sign_and_insert(
            key,
            time,
            Payload::Add {
                device,
                device_type,
                name: name.to_string(),
            },
        )
        .map(Some)
        .map_err(AddError::Change)
    }

    /// Signs a `rename` of a member.
    pub fn sign_rename(
        &mut self,
        key: &DeviceKey,
        device: Key,
        name: &str,
        time: u64,
    ) -> Result<Change, ChangeError> {
        if check_name(name.as_bytes()).is_none() || !self.is_member(&device) {
            return Err(ChangeError::Parse);
        }
        self.require_member(key)?;
        self.sign_and_insert(
            key,
            time,
            Payload::Rename {
                device,
                name: name.to_string(),
            },
        )
    }

    /// Signs a `remove`. Unless the device removes itself, it also signs a `rotate` whose parent
    /// is the remove and returns the new secret (5.8).
    pub fn sign_remove<R: RngCore + CryptoRng>(
        &mut self,
        key: &DeviceKey,
        device: Key,
        time: u64,
        rng: &mut R,
    ) -> Result<(Change, Option<(Change, GroupSecret)>), ChangeError> {
        self.require_member(key)?;
        let rm = self.sign_and_insert(key, time, Payload::Remove { device })?;
        if device == key.public() {
            return Ok((rm, None));
        }
        let (c, s) = self.sign_rotate(key, time, rng)?;
        Ok((rm, Some((c, s))))
    }

    /// Signs a `rotate` with a new random secret (epoch is 1 plus the highest epoch in the past).
    pub fn sign_rotate<R: RngCore + CryptoRng>(
        &mut self,
        key: &DeviceKey,
        time: u64,
        rng: &mut R,
    ) -> Result<(Change, GroupSecret), ChangeError> {
        self.require_member(key)?;
        let heads = self.heads();
        let mut past = Bits::with_len(self.nodes.len());
        for h in &heads {
            let i = self.index[h];
            past.set(i);
            past.union(&self.nodes[i].past);
        }
        let max = past
            .iter()
            .filter_map(|i| self.nodes[i].change.epoch())
            .max()
            .unwrap_or(0);
        let secret = GroupSecret::random(max + 1, rng);
        let c = self.sign_and_insert(
            key,
            time,
            Payload::Rotate {
                epoch: secret.epoch,
                secret_commit: secret.commit(),
            },
        )?;
        Ok((c, secret))
    }
}

/// Why an `add` was not signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddError {
    /// The key is removed in this group (4.7 step 1).
    Removed,
    /// The group has 8 members (4.7 step 3).
    Full,
    Change(ChangeError),
}

/// What a device must do after its graph changed (5.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfAction {
    /// Its own key is a member: go on.
    Stay,
    /// Its own key is no longer a member: stop sync, wipe the group state and the device key,
    /// make a new key, show "Another device removed this one".
    WipeAndRekey,
}

/// Decides 5.7 from the device's own evaluation. This is the only input that can make a device
/// wipe itself; a bare "removed" hint from a peer has no effect.
pub fn self_action(graph: &Graph, me: &Key) -> SelfAction {
    if graph.is_member(me) {
        SelfAction::Stay
    } else {
        SelfAction::WipeAndRekey
    }
}

/// Carries out [`SelfAction::WipeAndRekey`] on the key store: deletes the device key and makes a
/// new one. The caller wipes the group state.
pub fn rekey(store: &dyn uc_keys::KeyStore) -> Result<DeviceKey, uc_keys::KeyError> {
    DeviceKey::replace(store)
}
