//! Pure-Rust packet parsing and checksum computation.
//!
//! Mirrors the parts of WinDivert's packet helpers that ClumsyMagi needs:
//! locating IPv4/IPv6/TCP/UDP/ICMP headers and the payload, and recomputing
//! checksums after packet modification. All functions operate on raw IP
//! packet bytes (the format WinDivert hands out at the NETWORK layer).

/// A captured packet: raw IP packet bytes plus WinDivert metadata.
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    pub addr: crate::wd::WdAddress,
    /// Set by modules (e.g. lag) for internal bookkeeping.
    pub due_ms: u64,
}

impl Packet {
    pub fn new(data: Vec<u8>, addr: crate::wd::WdAddress) -> Packet {
        Packet { data, addr, due_ms: 0 }
    }
    pub fn outbound(&self) -> bool {
        self.addr.outbound
    }
}

/// Parsed header views of a packet. Offsets are into the packet buffer.
#[derive(Debug, Clone, Copy)]
pub struct PacketView {
    pub ipv6: bool,
    /// Length of the IP header (incl. IPv6 extension headers).
    pub ip_hdr_len: usize,
    /// Offset of the transport header (TCP/UDP/ICMP), if any.
    pub transport: Option<usize>,
    /// Transport header length (TCP data offset / UDP 8 / ICMP 8 / ICMPv6 8).
    pub transport_len: usize,
    pub protocol: u8,
    /// Offset of the payload, `None` when there is none.
    pub payload: Option<usize>,
    pub payload_len: usize,
}

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_ICMPV6: u8 = 58;

/// Parse a raw IP packet. Follows IPv6 extension headers (up to 8) to find the
/// transport header. Returns `None` for anything it cannot interpret.
pub fn parse(data: &[u8]) -> Option<PacketView> {
    if data.len() < 1 {
        return None;
    }
    let version = data[0] >> 4;
    match version {
        4 => parse_ipv4(data),
        6 => parse_ipv6(data),
        _ => None,
    }
}

fn parse_ipv4(data: &[u8]) -> Option<PacketView> {
    if data.len() < 20 {
        return None;
    }
    let ihl = (data[0] & 0x0F) as usize * 4;
    if ihl < 20 || data.len() < ihl {
        return None;
    }
    let protocol = data[9];
    finish(data, ihl, transport_start(data, ihl, protocol), protocol, false)
}

fn parse_ipv6(data: &[u8]) -> Option<PacketView> {
    if data.len() < 40 {
        return None;
    }
    let mut offset = 40usize;
    let mut next = data[6];
    // Walk the extension header chain (each ext header: [next, len*8+8 - 8 bytes...])
    let mut hops = 0;
    while matches!(
        next,
        0 | 43 | 44 | 51 | 50 | 60 | 135 | 139 | 140 // hop-by-hop, routing, frag, ah, dst, mobility, hip, shim6
    ) && hops < 8
    {
        if next == 44 {
            // fragment header is fixed 8 bytes
            if data.len() < offset + 8 {
                return None;
            }
            next = data[offset];
            offset += 8;
        } else {
            if data.len() < offset + 8 {
                return None;
            }
            let ext_len = (data[offset + 1] as usize + 1) * 8;
            if data.len() < offset + ext_len {
                return None;
            }
            next = data[offset];
            offset += ext_len;
        }
        hops += 1;
    }
    finish(data, offset, transport_start(data, offset, next), next, true)
}

fn transport_start(data: &[u8], off: usize, protocol: u8) -> (Option<usize>, usize) {
    match protocol {
        PROTO_TCP => {
            if data.len() < off + 20 {
                return (None, 0);
            }
            let doff = ((data[off + 12] >> 4) as usize) * 4;
            let len = if doff < 20 || data.len() < off + doff { 0 } else { doff };
            (Some(off), len)
        }
        PROTO_UDP | PROTO_ICMP | PROTO_ICMPV6 => {
            if data.len() < off + 8 {
                (None, 0)
            } else {
                (Some(off), 8)
            }
        }
        _ => (None, 0),
    }
}

fn finish(
    data: &[u8],
    ip_hdr_len: usize,
    (transport, transport_len): (Option<usize>, usize),
    protocol: u8,
    ipv6: bool,
) -> Option<PacketView> {
    let (payload, payload_len) = match (transport, transport_len) {
        (Some(t), tl) if tl > 0 => {
            let start = t + tl;
            if start > data.len() {
                return None;
            }
            (Some(start), data.len() - start)
        }
        _ => (None, 0),
    };
    Some(PacketView {
        ipv6,
        ip_hdr_len,
        transport,
        transport_len,
        protocol,
        payload,
        payload_len,
    })
}

// ---------------------------------------------------------------- checksums

fn ones_complement_sum(bytes: &[u8], init: u32) -> u32 {
    let mut sum = init;
    let mut chunks = bytes.chunks_exact(2);
    for c in &mut chunks {
        sum += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [odd] = chunks.remainder() {
        sum += (*odd as u32) << 8;
    }
    sum
}

fn fold(sum: u32) -> u16 {
    let mut s = sum;
    while s >> 16 != 0 {
        s = (s & 0xFFFF) + (s >> 16);
    }
    !(s as u16)
}

/// Recompute all applicable checksums of a packet, like
/// WinDivertHelperCalcChecksums with flags 0. Handles IPv4 and IPv6.
pub fn calc_checksums(data: &mut [u8]) {
    let Some(view) = parse(data) else { return };
    if !view.ipv6 {
        // IPv4 header checksum
        let ihl = view.ip_hdr_len;
        if ihl >= 20 && data.len() >= ihl {
            // recompute over a copy of the header with the checksum zeroed
            let mut hdr = data[..ihl].to_vec();
            hdr[10] = 0;
            hdr[11] = 0;
            let ck = fold(ones_complement_sum(&hdr, 0));
            data[10] = (ck >> 8) as u8;
            data[11] = (ck & 0xFF) as u8;
        }
    }
    let Some(t) = view.transport else { return };
    match view.protocol {
        PROTO_TCP => {
            if view.transport_len < 20 {
                return;
            }
            let pseudo = pseudo_header(data, &view, PROTO_TCP);
            let seg = &mut data[t..];
            seg[16] = 0;
            seg[17] = 0;
            let sum = ones_complement_sum(seg, ones_complement_sum(&pseudo, 0));
            let ck = fold(sum);
            seg[16] = (ck >> 8) as u8;
            seg[17] = (ck & 0xFF) as u8;
        }
        PROTO_UDP => {
            if view.transport_len < 8 {
                return;
            }
            let pseudo = pseudo_header(data, &view, PROTO_UDP);
            let seg = &mut data[t..];
            seg[6] = 0;
            seg[7] = 0;
            let sum = ones_complement_sum(seg, ones_complement_sum(&pseudo, 0));
            let ck = fold(sum);
            // UDP checksum 0x0000 means "no checksum"; transmit 0xFFFF instead.
            let ck = if ck == 0 { 0xFFFF } else { ck };
            seg[6] = (ck >> 8) as u8;
            seg[7] = (ck & 0xFF) as u8;
        }
        PROTO_ICMP => {
            if data.len() < t + 4 {
                return;
            }
            let seg = &mut data[t..];
            seg[2] = 0;
            seg[3] = 0;
            let sum = ones_complement_sum(seg, 0);
            let ck = fold(sum);
            seg[2] = (ck >> 8) as u8;
            seg[3] = (ck & 0xFF) as u8;
        }
        PROTO_ICMPV6 => {
            if data.len() < t + 4 {
                return;
            }
            let pseudo = pseudo_header(data, &view, PROTO_ICMPV6);
            let seg = &mut data[t..];
            seg[2] = 0;
            seg[3] = 0;
            let sum = ones_complement_sum(seg, ones_complement_sum(&pseudo, 0));
            let ck = fold(sum);
            seg[2] = (ck >> 8) as u8;
            seg[3] = (ck & 0xFF) as u8;
        }
        _ => {}
    }
}

/// IPv4/IPv6 pseudo header bytes used for TCP/UDP checksums.
fn pseudo_header(data: &[u8], view: &PacketView, protocol: u8) -> Vec<u8> {
    let mut v = Vec::with_capacity(40);
    if view.ipv6 {
        v.extend_from_slice(&data[8..24]); // src
        v.extend_from_slice(&data[24..40]); // dst
        let upper_len = (data.len() - view.ip_hdr_len) as u32;
        v.extend_from_slice(&upper_len.to_be_bytes());
        v.push(0);
        v.push(0);
        v.push(0);
        v.push(protocol);
    } else {
        v.extend_from_slice(&data[12..16]); // src
        v.extend_from_slice(&data[16..20]); // dst
        v.push(0);
        v.push(protocol);
        let upper_len = (data.len() - view.ip_hdr_len) as u16;
        v.extend_from_slice(&upper_len.to_be_bytes());
    }
    v
}

/// TCP flag tuple (fin, syn, rst, psh, ack, urg); used by the reset module and tests.
#[allow(dead_code)]
pub fn tcp_flags(data: &[u8], view: &PacketView) -> Option<(bool, bool, bool, bool, bool, bool)> {
    let t = view.transport?;
    if view.protocol != PROTO_TCP || data.len() < t + 20 {
        return None;
    }
    let b13 = data[t + 13];
    Some((
        b13 & 0x01 != 0, // fin
        b13 & 0x02 != 0, // syn
        b13 & 0x04 != 0, // rst
        b13 & 0x08 != 0, // psh
        b13 & 0x10 != 0, // ack
        b13 & 0x20 != 0, // urg
    ))
}

pub fn set_rst_flag(data: &mut [u8], view: &PacketView) -> bool {
    let Some(t) = view.transport else { return false };
    if view.protocol != PROTO_TCP || data.len() < t + 20 {
        return false;
    }
    data[t + 13] |= 0x04;
    true
}

/// Swap source/destination IP addresses (used when resending a failed inbound
/// ICMP packet as outbound, mirroring clumsy's divert.c workaround).
pub fn swap_ip_addrs(data: &mut [u8]) {
    if data.len() >= 20 && data[0] >> 4 == 4 {
        let src: [u8; 4] = data[12..16].try_into().unwrap();
        let dst: [u8; 4] = data[16..20].try_into().unwrap();
        data[12..16].copy_from_slice(&dst);
        data[16..20].copy_from_slice(&src);
    } else if data.len() >= 40 && data[0] >> 4 == 6 {
        let src: [u8; 16] = data[8..24].try_into().unwrap();
        let dst: [u8; 16] = data[24..40].try_into().unwrap();
        data[8..24].copy_from_slice(&dst);
        data[24..40].copy_from_slice(&src);
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid IPv4 + TCP packet (no payload).
    fn ipv4_tcp() -> Vec<u8> {
        let mut d = vec![0u8; 40];
        d[0] = 0x45; // version 4, ihl 5
        d[1] = 0;
        let total = 40u16;
        d[2..4].copy_from_slice(&total.to_be_bytes());
        d[8] = 64; // ttl
        d[9] = PROTO_TCP;
        d[12..16].copy_from_slice(&[192, 168, 1, 2]);
        d[16..20].copy_from_slice(&[192, 168, 1, 3]);
        // tcp: srcport 1234 dstport 80, doff 5
        d[20..22].copy_from_slice(&1234u16.to_be_bytes());
        d[22..24].copy_from_slice(&80u16.to_be_bytes());
        d[32] = 0x50; // data offset 5
        d[33] = 0x18; // psh|ack
        d
    }

    /// Build a minimal valid IPv4 + UDP packet with 6 payload bytes.
    fn ipv4_udp(payload: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 20 + 8 + payload.len()];
        d[0] = 0x45;
        let total = d.len() as u16;
        d[2..4].copy_from_slice(&total.to_be_bytes());
        d[8] = 64;
        d[9] = PROTO_UDP;
        d[12..16].copy_from_slice(&[10, 0, 0, 1]);
        d[16..20].copy_from_slice(&[10, 0, 0, 2]);
        d[20..22].copy_from_slice(&5000u16.to_be_bytes());
        d[22..24].copy_from_slice(&6000u16.to_be_bytes());
        let udp_len = (8 + payload.len()) as u16;
        d[24..26].copy_from_slice(&udp_len.to_be_bytes());
        d[28..].copy_from_slice(payload);
        d
    }

    #[test]
    fn parse_ipv4_tcp() {
        let d = ipv4_tcp();
        let v = parse(&d).unwrap();
        assert!(!v.ipv6);
        assert_eq!(v.ip_hdr_len, 20);
        assert_eq!(v.transport, Some(20));
        assert_eq!(v.transport_len, 20);
        assert_eq!(v.payload, Some(40));
        assert_eq!(v.payload_len, 0);
        assert_eq!(v.protocol, PROTO_TCP);
    }

    #[test]
    fn parse_ipv4_udp_payload() {
        let d = ipv4_udp(&[1, 2, 3, 4, 5, 6]);
        let v = parse(&d).unwrap();
        assert_eq!(v.transport, Some(20));
        assert_eq!(v.transport_len, 8);
        assert_eq!(v.payload, Some(28));
        assert_eq!(v.payload_len, 6);
        assert_eq!(v.protocol, PROTO_UDP);
    }

    #[test]
    fn checksums_roundtrip_tcp() {
        let mut d = ipv4_tcp();
        calc_checksums(&mut d);
        // header checksum of good packet is 0 when summed (incl. its own field)
        let sum = ones_complement_sum(&d[..20], 0);
        assert_eq!(fold(sum), 0);
        // tcp segment checksum incl. pseudo header must verify to 0
        let v = parse(&d).unwrap();
        let pseudo = pseudo_header(&d, &v, PROTO_TCP);
        let t = v.transport.unwrap();
        let sum = ones_complement_sum(&d[t..], ones_complement_sum(&pseudo, 0));
        assert_eq!(fold(sum), 0);
    }

    #[test]
    fn checksums_detect_mutation() {
        let mut d = ipv4_udp(&[9, 9, 9, 9]);
        calc_checksums(&mut d);
        // verify udp checksum ok
        let v = parse(&d).unwrap();
        let pseudo = pseudo_header(&d, &v, PROTO_UDP);
        let t = v.transport.unwrap();
        let sum = ones_complement_sum(&d[t..], ones_complement_sum(&pseudo, 0));
        assert_eq!(fold(sum), 0);
        // mutate payload -> checksum no longer verifies
        d[30] ^= 0xFF;
        let sum = ones_complement_sum(&d[t..], ones_complement_sum(&pseudo, 0));
        assert_ne!(fold(sum), 0);
        // recompute -> verifies again
        calc_checksums(&mut d);
        let v = parse(&d).unwrap();
        let pseudo = pseudo_header(&d, &v, PROTO_UDP);
        let sum = ones_complement_sum(&d[t..], ones_complement_sum(&pseudo, 0));
        assert_eq!(fold(sum), 0);
    }

    #[test]
    fn ipv6_tcp_parse_and_checksum() {
        let mut d = vec![0u8; 40 + 20];
        d[0] = 0x60; // version 6
        let payload = 20u16;
        d[4..6].copy_from_slice(&payload.to_be_bytes());
        d[6] = PROTO_TCP;
        d[7] = 64;
        d[8..24].copy_from_slice(&[0x20; 16]);
        d[24..40].copy_from_slice(&[0x40; 16]);
        d[40 + 12] = 0x50;
        d[40 + 13] = 0x02; // syn
        calc_checksums(&mut d);
        let v = parse(&d).unwrap();
        assert!(v.ipv6);
        assert_eq!(v.transport, Some(40));
        let pseudo = pseudo_header(&d, &v, PROTO_TCP);
        let t = v.transport.unwrap();
        let sum = ones_complement_sum(&d[t..], ones_complement_sum(&pseudo, 0));
        assert_eq!(fold(sum), 0);
    }

    #[test]
    fn rst_flag_roundtrip() {
        let mut d = ipv4_tcp();
        calc_checksums(&mut d);
        let v = parse(&d).unwrap();
        let before = tcp_flags(&d, &v).unwrap();
        assert!(!before.2);
        assert!(set_rst_flag(&mut d, &v));
        calc_checksums(&mut d);
        let v = parse(&d).unwrap();
        let after = tcp_flags(&d, &v).unwrap();
        assert!(after.2);
    }

    #[test]
    fn icmp_checksum_roundtrip() {
        // IPv4 ICMP Echo Request: 20-byte IP header + 8-byte ICMP
        let mut d = vec![0u8; 28];
        d[0] = 0x45;
        d[2..4].copy_from_slice(&28u16.to_be_bytes());
        d[8] = 64;
        d[9] = PROTO_ICMP;
        d[12..16].copy_from_slice(&[192, 168, 1, 10]);
        d[16..20].copy_from_slice(&[192, 168, 1, 1]);
        d[20] = 8; // Echo request
        d[21] = 0; // Code
        // Identifier & sequence
        d[24..26].copy_from_slice(&1u16.to_be_bytes());
        d[26..28].copy_from_slice(&1u16.to_be_bytes());

        calc_checksums(&mut d);
        let v = parse(&d).unwrap();
        assert_eq!(v.protocol, PROTO_ICMP);
        let t = v.transport.unwrap();
        let sum = ones_complement_sum(&d[t..], 0);
        assert_eq!(fold(sum), 0, "ICMP checksum must verify");
    }

    #[test]
    fn icmpv6_checksum_roundtrip() {
        // IPv6 ICMPv6 Echo Request: 40-byte IPv6 header + 8-byte ICMPv6
        let mut d = vec![0u8; 48];
        d[0] = 0x60;
        d[4..6].copy_from_slice(&8u16.to_be_bytes());
        d[6] = PROTO_ICMPV6;
        d[7] = 64;
        d[8..24].copy_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        d[24..40].copy_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
        d[40] = 128; // Echo request
        d[41] = 0;   // Code
        d[44..46].copy_from_slice(&1u16.to_be_bytes());
        d[46..48].copy_from_slice(&1u16.to_be_bytes());

        calc_checksums(&mut d);
        let v = parse(&d).unwrap();
        assert!(v.ipv6);
        assert_eq!(v.protocol, PROTO_ICMPV6);
        let pseudo = pseudo_header(&d, &v, PROTO_ICMPV6);
        let t = v.transport.unwrap();
        let sum = ones_complement_sum(&d[t..], ones_complement_sum(&pseudo, 0));
        assert_eq!(fold(sum), 0, "ICMPv6 checksum must verify");
    }
}
