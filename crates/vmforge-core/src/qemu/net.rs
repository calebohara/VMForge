//! Pure network-fragment construction for the QEMU command line.
//!
//! No process spawning, no I/O — this builds the `-netdev`/`-device` argv
//! fragments and validates the user-supplied port-forward and MAC inputs, so
//! the whole surface is unit-testable without QEMU installed.
//!
//! Mode policy (Phase 4):
//! - **User** (NAT) — fully supported. Port forwards bind loopback by default;
//!   `expose_lan` opts a forward into all-interfaces binding (decision A1/A9).
//! - **Bridged / Host-only** — abstraction + capability + UX only; privileged
//!   bring-up is deferred (decision A2/A3). [`network_args`] REJECTS them with a
//!   typed [`NetworkBuildError::RequiresElevatedPermissions`] (the engine maps it
//!   to `Error::Config`); there is NEVER a silent NAT fallback.
//!
//! Owned by network-engineer.

use crate::host::Accelerator;
use crate::model::{NetworkConfig, NetworkMode, PortForward};
use std::collections::HashSet;
use std::fmt;

/// A typed failure building the network fragments. Lives entirely inside the
/// core; the engine flattens it to `Error::Config(String)` at the boundary
/// (decision A4 — no new error variant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkBuildError {
    /// The requested mode needs elevated permissions VMForge cannot grant yet.
    /// `reason` is the per-OS, user-facing explanation (needs-permission /
    /// not-implemented, NEVER "unsupported").
    RequiresElevatedPermissions { mode: NetworkMode, reason: String },
    /// A port-forward entry is invalid (bad port, duplicate, …).
    InvalidPortForward(String),
    /// A MAC address is malformed or disallowed (multicast).
    InvalidMac(String),
}

impl fmt::Display for NetworkBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Reason is already a complete, user-facing sentence — emit verbatim.
            NetworkBuildError::RequiresElevatedPermissions { reason, .. } => {
                write!(f, "{reason}")
            }
            NetworkBuildError::InvalidPortForward(msg) => {
                write!(f, "invalid port forward: {msg}")
            }
            NetworkBuildError::InvalidMac(msg) => write!(f, "invalid MAC address: {msg}"),
        }
    }
}

impl std::error::Error for NetworkBuildError {}

/// Host bind address for a port forward: loopback unless the forward opts into
/// LAN exposure, in which case the empty host-addr form (`0.0.0.0`) is used.
fn host_bind_addr(expose_lan: bool) -> &'static str {
    if expose_lan {
        ""
    } else {
        "127.0.0.1"
    }
}

/// Build the `-netdev` + `-device` argv fragments for a VM's network config.
///
/// Rejects bridged/host-only with a typed error (the engine maps it to
/// `Error::Config`); there is NEVER a silent NAT fallback (decision A3).
/// `_accel` is reserved for future per-accelerator netdev tuning.
pub fn network_args(
    net: &NetworkConfig,
    _accel: Accelerator,
) -> Result<Vec<String>, NetworkBuildError> {
    match net.mode {
        NetworkMode::User => user_mode_args(net),
        NetworkMode::Bridged | NetworkMode::HostOnly => {
            Err(NetworkBuildError::RequiresElevatedPermissions {
                mode: net.mode,
                reason: elevated_reason(net.mode),
            })
        }
    }
}

/// Build user-mode (NAT) fragments. Validates port forwards + MAC first.
fn user_mode_args(net: &NetworkConfig) -> Result<Vec<String>, NetworkBuildError> {
    validate_port_forwards(&net.port_forwards).map_err(NetworkBuildError::InvalidPortForward)?;
    if let Some(mac) = &net.mac {
        validate_mac(mac).map_err(NetworkBuildError::InvalidMac)?;
    }

    let mut netdev = String::from("user,id=net0");
    for pf in &net.port_forwards {
        let proto = if pf.udp { "udp" } else { "tcp" };
        let bind = host_bind_addr(pf.expose_lan);
        netdev.push_str(&format!(
            ",hostfwd={proto}:{bind}:{}-:{}",
            pf.host, pf.guest
        ));
    }

    let device = match &net.mac {
        Some(mac) => format!("virtio-net-pci,netdev=net0,mac={mac}"),
        None => "virtio-net-pci,netdev=net0".to_string(),
    };

    Ok(vec!["-netdev".into(), netdev, "-device".into(), device])
}

/// The user-facing reason a privileged mode is unavailable. Shared by the
/// launch-reject path AND the capability probe so the two never drift
/// (decision A2 — say needs-permission / not-implemented, never "unsupported").
pub(crate) fn elevated_reason(mode: NetworkMode) -> String {
    let label = match mode {
        NetworkMode::Bridged => "Bridged networking",
        NetworkMode::HostOnly => "Host-only networking",
        // User mode is never elevated; keep the match total defensively.
        NetworkMode::User => "User-mode networking",
    };
    format!(
        "{label} requires a configured bridged network adapter and Administrator \
         privileges; VMForge cannot configure it in this build yet. This mode is \
         not available in this build yet."
    )
}

/// Validate a set of port forwards. Each port must be in `1..=65535` (reject 0);
/// no two forwards may share the same `(protocol, host)` pair, but TCP and UDP
/// may share a host port. Returns a human-readable message on the first error.
pub fn validate_port_forwards(pfs: &[PortForward]) -> Result<(), String> {
    let mut seen: HashSet<(bool, u16)> = HashSet::new();
    for pf in pfs {
        if pf.host == 0 {
            return Err(format!(
                "host port must be 1-65535 (got 0, guest {})",
                pf.guest
            ));
        }
        if pf.guest == 0 {
            return Err(format!(
                "guest port must be 1-65535 (got 0, host {})",
                pf.host
            ));
        }
        // Duplicate detection keys on (udp, host): tcp+udp may share a host
        // port, but two same-protocol forwards on one host port collide.
        if !seen.insert((pf.udp, pf.host)) {
            let proto = if pf.udp { "udp" } else { "tcp" };
            return Err(format!("duplicate {proto} host port {}", pf.host));
        }
    }
    Ok(())
}

/// Validate a MAC address: exactly six colon-separated hex octets, and not a
/// multicast address (low bit of the first octet must be clear). Returns a
/// human-readable message on failure.
pub fn validate_mac(mac: &str) -> Result<(), String> {
    let octets: Vec<&str> = mac.split(':').collect();
    if octets.len() != 6 {
        return Err(format!(
            "expected 6 colon-separated octets (e.g. 52:54:00:12:34:56), got {:?}",
            mac
        ));
    }
    let mut bytes = [0u8; 6];
    for (i, octet) in octets.iter().enumerate() {
        if octet.len() != 2 || !octet.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "octet {} ({:?}) is not two hex digits",
                i + 1,
                octet
            ));
        }
        bytes[i] = u8::from_str_radix(octet, 16).map_err(|e| e.to_string())?;
    }
    // Multicast bit (least-significant bit of the first octet). QEMU/guests
    // expect a unicast MAC for a NIC.
    if bytes[0] & 0x01 != 0 {
        return Err(format!(
            "{mac} is a multicast address (low bit of first octet set); use a unicast MAC"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NetworkConfig;

    fn user_net(port_forwards: Vec<PortForward>, mac: Option<String>) -> NetworkConfig {
        NetworkConfig {
            mode: NetworkMode::User,
            mac,
            port_forwards,
        }
    }

    fn pf(host: u16, guest: u16, udp: bool, expose_lan: bool) -> PortForward {
        PortForward {
            host,
            guest,
            udp,
            expose_lan,
        }
    }

    /// Pull the value following a flag key out of an argv slice.
    fn after<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    // ---- loopback by default ----
    #[test]
    fn hostfwd_binds_loopback_by_default() {
        let net = user_net(vec![pf(2222, 22, false, false)], None);
        let args = network_args(&net, Accelerator::Whpx).unwrap();
        assert_eq!(
            after(&args, "-netdev"),
            Some("user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22")
        );
        assert_eq!(after(&args, "-device"), Some("virtio-net-pci,netdev=net0"));
    }

    // ---- expose_lan → bind-all (empty host addr) ----
    #[test]
    fn expose_lan_binds_all_interfaces() {
        let net = user_net(vec![pf(2222, 22, false, true)], None);
        let args = network_args(&net, Accelerator::Whpx).unwrap();
        assert_eq!(
            after(&args, "-netdev"),
            Some("user,id=net0,hostfwd=tcp::2222-:22")
        );
    }

    // ---- udp protocol + mac on the device ----
    #[test]
    fn udp_forward_and_mac_on_device() {
        let net = user_net(
            vec![pf(5353, 53, true, false)],
            Some("52:54:00:12:34:56".into()),
        );
        let args = network_args(&net, Accelerator::Tcg).unwrap();
        assert_eq!(
            after(&args, "-netdev"),
            Some("user,id=net0,hostfwd=udp:127.0.0.1:5353-:53")
        );
        assert_eq!(
            after(&args, "-device"),
            Some("virtio-net-pci,netdev=net0,mac=52:54:00:12:34:56")
        );
    }

    // ---- zero forwards → bare netdev (regression guard) ----
    #[test]
    fn zero_forwards_emit_bare_netdev() {
        let net = NetworkConfig::default();
        let args = network_args(&net, Accelerator::Whpx).unwrap();
        assert_eq!(after(&args, "-netdev"), Some("user,id=net0"));
        assert_eq!(after(&args, "-device"), Some("virtio-net-pci,netdev=net0"));
    }

    // ---- bridged rejected with a needs-permission reason, no NAT fallback ----
    #[test]
    fn bridged_rejected_with_reason() {
        let net = NetworkConfig {
            mode: NetworkMode::Bridged,
            mac: None,
            port_forwards: vec![],
        };
        match network_args(&net, Accelerator::Whpx) {
            Ok(args) => panic!("bridged must NOT silently fall back to NAT: {args:?}"),
            Err(NetworkBuildError::RequiresElevatedPermissions { mode, reason }) => {
                assert_eq!(mode, NetworkMode::Bridged);
                assert!(!reason.is_empty(), "reason must be non-empty");
                assert!(
                    !reason.to_lowercase().contains("unsupported"),
                    "reason must not say 'unsupported': {reason}"
                );
                // The reason names the Windows requirement (matches host.rs).
                assert!(
                    reason.contains("Administrator"),
                    "reason must mention the Administrator requirement: {reason}"
                );
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn host_only_rejected_no_fallback() {
        let net = NetworkConfig {
            mode: NetworkMode::HostOnly,
            mac: None,
            port_forwards: vec![],
        };
        match network_args(&net, Accelerator::Tcg) {
            Ok(args) => panic!("host-only must NOT fall back to NAT: {args:?}"),
            Err(NetworkBuildError::RequiresElevatedPermissions { mode, .. }) => {
                assert_eq!(mode, NetworkMode::HostOnly);
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    // ---- validator: reject zero port ----
    #[test]
    fn rejects_zero_port() {
        assert!(validate_port_forwards(&[pf(0, 22, false, false)]).is_err());
        assert!(validate_port_forwards(&[pf(2222, 0, false, false)]).is_err());
        // Surfaces through network_args as InvalidPortForward.
        let net = user_net(vec![pf(0, 22, false, false)], None);
        assert!(matches!(
            network_args(&net, Accelerator::Whpx),
            Err(NetworkBuildError::InvalidPortForward(_))
        ));
    }

    // ---- validator: reject duplicate same-proto host port ----
    #[test]
    fn rejects_duplicate_same_proto_host_port() {
        let err = validate_port_forwards(&[pf(2222, 22, false, false), pf(2222, 80, false, false)])
            .unwrap_err();
        assert!(err.contains("2222"), "got {err}");
    }

    // ---- validator: allow tcp + udp sharing one host port ----
    #[test]
    fn allows_tcp_and_udp_same_host_port() {
        assert!(
            validate_port_forwards(&[pf(5000, 22, false, false), pf(5000, 53, true, false)])
                .is_ok()
        );
    }

    // ---- validator: reject bad / multicast MAC ----
    #[test]
    fn rejects_bad_and_multicast_mac() {
        // Too few octets.
        assert!(validate_mac("52:54:00:12:34").is_err());
        // Non-hex.
        assert!(validate_mac("52:54:00:12:34:zz").is_err());
        // Wrong octet width.
        assert!(validate_mac("5:54:00:12:34:56").is_err());
        // Multicast: low bit of first octet set (0x01).
        assert!(validate_mac("01:54:00:12:34:56").is_err());
        assert!(validate_mac("03:54:00:12:34:56").is_err());
        // Unicast is accepted (low bit clear: 0x02).
        assert!(validate_mac("02:54:00:12:34:56").is_ok());
        assert!(validate_mac("52:54:00:12:34:56").is_ok());
    }

    // ---- MAC comma-injection is rejected (would break the option list) ----
    #[test]
    fn rejects_mac_comma_injection() {
        // A comma-bearing MAC must fail validation before it can splice extra
        // QEMU device options into the -device string.
        let injected = "52:54:00:12:34:56,if=foo";
        assert!(validate_mac(injected).is_err());
        let net = user_net(vec![], Some(injected.into()));
        assert!(matches!(
            network_args(&net, Accelerator::Whpx),
            Err(NetworkBuildError::InvalidMac(_))
        ));
    }
}
