//! Socket binding behind a trait, so iOS and Android can plug in their own (plan 3.1).
//!
//! The endpoint asks the binder for one UDP socket per address of each physical interface.
//! [`OsBinder`] binds to the address and, where the OS allows, also ties the socket to the
//! interface so the route table cannot send its packets into a VPN tunnel:
//!
//! | OS | Tie |
//! |---|---|
//! | Linux, Android | `SO_BINDTODEVICE` (no root needed on kernel 5.7 and newer) |
//! | macOS, iOS | `IP_BOUND_IF` / `IPV6_BOUND_IF` |
//! | Windows | `IP_UNICAST_IF` / `IPV6_UNICAST_IF` |
//!
//! If the tie fails, the socket stays bound to the address only and [`Tie::AddressOnly`] says
//! so (shown in diagnostics). Android replaces this with a binder that hands the socket's file
//! descriptor to `Network.bindSocket()` in Kotlin.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use socket2::{Domain, Protocol, Socket, Type};

use crate::ifaces::Interface;

/// How a socket is tied to its interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tie {
    /// Bound to the address and tied to the interface.
    Device,
    /// Bound to the address only.
    AddressOnly,
}

/// Makes the UDP sockets for the endpoint.
pub trait SocketBinder: Send + Sync {
    /// Make a UDP socket bound to `local` (an address of `iface`) and, if possible, tied to
    /// `iface`. The socket must be non-blocking-ready (the endpoint sets non-blocking itself).
    fn bind(&self, iface: &Interface, local: SocketAddr) -> io::Result<(UdpSocket, Tie)>;
}

fn base_socket(local: SocketAddr) -> io::Result<Socket> {
    let s = Socket::new(Domain::for_address(local), Type::DGRAM, Some(Protocol::UDP))?;
    if local.is_ipv6() {
        s.set_only_v6(true)?;
    }
    Ok(s)
}

/// Binds to the address only. Used by the loopback tests and as the fallback.
#[derive(Clone, Copy, Debug, Default)]
pub struct AddressBinder;

impl SocketBinder for AddressBinder {
    fn bind(&self, _iface: &Interface, local: SocketAddr) -> io::Result<(UdpSocket, Tie)> {
        let s = base_socket(local)?;
        s.bind(&local.into())?;
        Ok((s.into(), Tie::AddressOnly))
    }
}

/// The desktop binder: address bind plus the OS interface tie.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsBinder;

impl SocketBinder for OsBinder {
    fn bind(&self, iface: &Interface, local: SocketAddr) -> io::Result<(UdpSocket, Tie)> {
        let s = base_socket(local)?;
        let tie = match tie(&s, iface, local) {
            Ok(()) => Tie::Device,
            Err(e) => {
                tracing::debug!(iface = %iface.name, error = %e, "interface tie failed, address bind only");
                Tie::AddressOnly
            }
        };
        s.bind(&local.into())?;
        Ok((s.into(), tie))
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn tie(s: &Socket, iface: &Interface, _local: SocketAddr) -> io::Result<()> {
    s.bind_device(Some(iface.name.as_bytes()))
}

#[cfg(target_vendor = "apple")]
fn tie(s: &Socket, iface: &Interface, local: SocketAddr) -> io::Result<()> {
    let idx = std::num::NonZeroU32::new(iface.index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "interface index 0"))?;
    if local.is_ipv6() {
        s.bind_device_by_index_v6(Some(idx))
    } else {
        s.bind_device_by_index_v4(Some(idx))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn tie(s: &Socket, iface: &Interface, local: SocketAddr) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, SOCKET, setsockopt,
    };
    // IPv4 wants the index in network byte order, IPv6 in host order (Microsoft docs,
    // "IPPROTO_IP socket options" and "IPPROTO_IPV6 socket options").
    let (level, name, value) = if local.is_ipv6() {
        (IPPROTO_IPV6, IPV6_UNICAST_IF, iface.index)
    } else {
        (IPPROTO_IP, IP_UNICAST_IF, iface.index.to_be())
    };
    let bytes = value.to_ne_bytes();
    // SAFETY: the socket handle is valid for the lifetime of `s`, and the option value is a
    // 4-byte buffer that lives across the call.
    let rc = unsafe {
        setsockopt(
            s.as_raw_socket() as SOCKET,
            level,
            name,
            bytes.as_ptr(),
            bytes.len() as i32,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
fn tie(_s: &Socket, _iface: &Interface, _local: SocketAddr) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no interface tie on this OS",
    ))
}
