//! The order of a sync connection (protocol 5.9).
//!
//! On every `uclip/1` connection: `Hello`, group ID check, removed-key check, membership sync,
//! then data only if the peer is a member. Until the gate opens, a device sends and reads no
//! clip, no secret and no address. M2 drives this gate from the real connection.

use crate::chain::{ChangeId, Graph, Key};

/// Limits for a peer whose key is not known yet (5.9).
pub const UNKNOWN_PEER_MAX_CHANGES: usize = 256;
pub const UNKNOWN_PEER_MAX_BYTES: usize = 64 * 1024;
pub const UNKNOWN_PEER_TIMEOUT_MS: u64 = 10_000;

/// Why the connection must close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    /// The peer's group ID is not ours.
    OtherGroup,
    /// The peer's TLS key is removed in our group.
    Removed,
    /// After membership sync the peer is still not a member.
    NotMember,
    /// A step came out of order.
    Order,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitHello,
    Membership,
    Open,
    Closed,
}

/// The gate of one sync connection.
#[derive(Debug)]
pub struct Gate {
    peer: Key,
    phase: Phase,
    /// The peer was not a member when the connection started: steps 1 to 4 only, with limits.
    pub unknown_peer: bool,
}

impl Gate {
    /// `peer` is the key from the TLS layer.
    pub fn new(peer: Key) -> Gate {
        Gate {
            peer,
            phase: Phase::WaitHello,
            unknown_peer: false,
        }
    }

    /// Steps 1 to 3: the peer's `Hello` arrived with its group ID.
    pub fn on_hello(&mut self, graph: &Graph, peer_group: &ChangeId) -> Result<(), Close> {
        if self.phase != Phase::WaitHello {
            return self.close(Close::Order);
        }
        if peer_group != graph.group_id() {
            return self.close(Close::OtherGroup);
        }
        if graph.is_removed(&self.peer) {
            return self.close(Close::Removed);
        }
        self.unknown_peer = !graph.is_member(&self.peer);
        self.phase = Phase::Membership;
        Ok(())
    }

    /// Step 5: membership sync is done and the graph was evaluated again.
    pub fn after_membership_sync(&mut self, graph: &Graph) -> Result<(), Close> {
        if self.phase != Phase::Membership {
            return self.close(Close::Order);
        }
        if graph.is_member(&self.peer) {
            self.phase = Phase::Open;
            Ok(())
        } else {
            self.close(Close::NotMember)
        }
    }

    /// Call after every later change of the graph (5.7): closes the gate if the peer is no
    /// longer a member.
    pub fn recheck(&mut self, graph: &Graph) -> Result<(), Close> {
        if self.phase == Phase::Open && !graph.is_member(&self.peer) {
            return self.close(Close::NotMember);
        }
        Ok(())
    }

    /// True once step 5 passed: clips, secrets and addresses may flow.
    pub fn data_allowed(&self) -> bool {
        self.phase == Phase::Open
    }

    /// True while membership changes may be exchanged.
    pub fn membership_allowed(&self) -> bool {
        matches!(self.phase, Phase::Membership | Phase::Open)
    }

    fn close(&mut self, c: Close) -> Result<(), Close> {
        self.phase = Phase::Closed;
        Err(c)
    }
}
