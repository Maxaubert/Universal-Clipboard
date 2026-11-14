//! Path state per peer and the VPN-blocks-LAN rule (plan 3.5, 2.7).
//!
//! The rule: if no direct connection works for [`BLOCK_DELAY`], the peer was seen on this LAN
//! before, and a VPN interface is up on this device (or was up on the peer at its last
//! announcement), the device row shows "Blocked by a VPN or firewall" and the Fix button opens
//! the VPN help ([`NoPath::VpnBlocksLan`], a "Needs attention" reason). Without a VPN the row
//! shows the peer as unreachable (asleep, off, a firewall or a guest network:
//! [`NoPath::Unreachable`]). A peer that was never seen on this LAN is [`NoPath::NotFound`].
//!
//! "Seen on this LAN before" means: a connection to it worked once, and one of its remembered
//! addresses is on-link for one of our physical interfaces now.

use std::time::Duration;

/// How long without a connection before a reason is shown (plan 3.5: 20 seconds).
pub const BLOCK_DELAY: Duration = Duration::from_secs(20);

/// Why there is no path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoPath {
    /// A VPN is up here or on the peer, and the peer was on this LAN before. Needs attention:
    /// "Blocked by a VPN or firewall", help: "Turn on Local network sharing in your VPN."
    VpnBlocksLan,
    /// The peer was on this LAN before, no VPN is involved. It is asleep or off, or a firewall
    /// or the network (guest Wi-Fi, client isolation) blocks it.
    Unreachable,
    /// The peer was never seen on this LAN (no remembered address in our subnets, no mDNS).
    NotFound,
}

/// The path to one peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathState {
    /// Connected directly over the LAN.
    Lan,
    /// Not connected yet; still inside [`BLOCK_DELAY`].
    Connecting,
    /// No path, with the reason.
    NoPath(NoPath),
}

/// The "Needs attention" reasons this layer can raise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attention {
    /// Desktop screen L12: "Can't reach your other device? Turn on Local network sharing in
    /// your VPN."
    VpnBlocksLan,
}

impl PathState {
    /// The "Needs attention" reason, if this state raises one.
    pub fn attention(&self) -> Option<Attention> {
        match self {
            PathState::NoPath(NoPath::VpnBlocksLan) => Some(Attention::VpnBlocksLan),
            _ => None,
        }
    }
}

/// What the rule looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    /// A link to the peer is up.
    pub connected: bool,
    /// The peer was seen on this LAN before (see the module text).
    pub seen_on_lan: bool,
    /// A VPN interface is up here.
    pub local_vpn: bool,
    /// The peer said a VPN was up there.
    pub peer_vpn: bool,
}

/// The rule, as a pure function of the inputs and the time without a connection.
pub fn evaluate(i: Inputs, down_for: Duration, delay: Duration) -> PathState {
    if i.connected {
        PathState::Lan
    } else if down_for < delay {
        PathState::Connecting
    } else if i.seen_on_lan && (i.local_vpn || i.peer_vpn) {
        PathState::NoPath(NoPath::VpnBlocksLan)
    } else if i.seen_on_lan {
        PathState::NoPath(NoPath::Unreachable)
    } else {
        PathState::NoPath(NoPath::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Duration = BLOCK_DELAY;

    fn i(connected: bool, seen: bool, lv: bool, pv: bool) -> Inputs {
        Inputs {
            connected,
            seen_on_lan: seen,
            local_vpn: lv,
            peer_vpn: pv,
        }
    }

    #[test]
    fn table() {
        let long = D + Duration::from_secs(1);
        let short = D - Duration::from_secs(1);
        // Connected wins over everything.
        assert_eq!(evaluate(i(true, true, true, true), long, D), PathState::Lan);
        // Inside the delay: no reason yet (no false alarm during a normal reconnect).
        assert_eq!(
            evaluate(i(false, true, true, false), short, D),
            PathState::Connecting
        );
        // The VPN rule, VPN here or on the peer.
        assert_eq!(
            evaluate(i(false, true, true, false), long, D),
            PathState::NoPath(NoPath::VpnBlocksLan)
        );
        assert_eq!(
            evaluate(i(false, true, false, true), long, D),
            PathState::NoPath(NoPath::VpnBlocksLan)
        );
        // No VPN: unreachable (asleep, firewall, guest network). Not a VPN false alarm.
        assert_eq!(
            evaluate(i(false, true, false, false), long, D),
            PathState::NoPath(NoPath::Unreachable)
        );
        // Never seen here: not the VPN's fault as far as we can tell (another network).
        assert_eq!(
            evaluate(i(false, false, true, false), long, D),
            PathState::NoPath(NoPath::NotFound)
        );
        assert_eq!(
            evaluate(i(false, true, true, false), long, D).attention(),
            Some(Attention::VpnBlocksLan)
        );
        assert_eq!(
            evaluate(i(false, true, false, false), long, D).attention(),
            None
        );
    }
}
