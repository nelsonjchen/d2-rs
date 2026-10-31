// SPDX-License-Identifier: MPL-2.0
// Exact pre-L23 address predicate (HEAD); baseline evidence, not a runtime path.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
fn v4_in(address: Ipv4Addr, prefix: [u8; 4], bits: u32) -> bool {
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    u32::from(address) & mask == u32::from(Ipv4Addr::from(prefix)) & mask
}

fn v6_in(address: Ipv6Addr, prefix: &str, bits: u32) -> bool {
    let prefix: Ipv6Addr = prefix.parse().expect("netpolicy prefix");
    let mask = if bits == 0 {
        0
    } else {
        u128::MAX << (128 - bits)
    };
    u128::from(address) & mask == u128::from(prefix) & mask
}

/// netpolicy.isPublicAddress over an Unmap()ped netip.Addr: Go's
/// IsGlobalUnicast/IsPrivate/IsLinkLocal*/IsMulticast predicates followed by
/// the nonPublicPrefixes table.
pub fn is_public_address(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    };
    match address {
        IpAddr::V4(v4) => {
            let non_public: &[([u8; 4], u32)] = &[
                // IsGlobalUnicast/IsUnspecified/IsLoopback/IsPrivate/
                // IsLinkLocal*/IsMulticast and the broadcast address.
                ([0, 0, 0, 0], 32),
                ([255, 255, 255, 255], 32),
                ([127, 0, 0, 0], 8),
                ([10, 0, 0, 0], 8),
                ([172, 16, 0, 0], 12),
                ([192, 168, 0, 0], 16),
                ([169, 254, 0, 0], 16),
                ([224, 0, 0, 0], 4),
                // nonPublicPrefixes
                ([0, 0, 0, 0], 8),
                ([100, 64, 0, 0], 10),
                ([168, 63, 129, 16], 32),
                ([192, 0, 0, 0], 24),
                ([192, 0, 2, 0], 24),
                ([192, 88, 99, 0], 24),
                ([198, 18, 0, 0], 15),
                ([198, 51, 100, 0], 24),
                ([203, 0, 113, 0], 24),
                ([240, 0, 0, 0], 4),
            ];
            !non_public
                .iter()
                .any(|(prefix, bits)| v4_in(v4, *prefix, *bits))
        }
        IpAddr::V6(v6) => {
            let non_public: &[(&str, u32)] = &[
                // IsGlobalUnicast/IsUnspecified/IsLoopback/IsPrivate/
                // IsLinkLocal*/IsMulticast.
                ("::", 128),
                ("::1", 128),
                ("fc00::", 7),
                ("fe80::", 10),
                ("ff00::", 8),
                // nonPublicPrefixes
                ("::", 96),
                ("64:ff9b::", 96),
                ("64:ff9b:1::", 48),
                ("100::", 64),
                ("2001::", 23),
                ("2001:db8::", 32),
                ("2002::", 16),
                ("3fff::", 20),
                ("5f00::", 16),
                ("fec0::", 10),
            ];
            !non_public
                .iter()
                .any(|(prefix, bits)| v6_in(v6, prefix, *bits))
        }
    }
}
