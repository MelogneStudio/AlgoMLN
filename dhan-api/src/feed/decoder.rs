use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub code: u8,
    pub len: u16,
    pub segment: u8,
    pub security_id: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthLevel {
    pub bid_qty: i32,
    pub ask_qty: i32,
    pub bid_orders: i16,
    pub ask_orders: i16,
    pub bid_price: f32,
    pub ask_price: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Packet {
    Ticker {
        ltp: f32,
        ltt: i32,
    },
    PrevClose {
        close: f32,
        oi: i32,
    },
    Oi {
        oi: i32,
    },
    Quote {
        ltp: f32,
        ltq: i16,
        ltt: i32,
        atp: f32,
        volume: i32,
        sell_qty: i32,
        buy_qty: i32,
        open: f32,
        close: f32,
        high: f32,
        low: f32,
    },
    Full {
        ltp: f32,
        ltq: i16,
        ltt: i32,
        atp: f32,
        volume: i32,
        sell_qty: i32,
        buy_qty: i32,
        oi: i32,
        high_oi: i32,
        low_oi: i32,
        open: f32,
        close: f32,
        high: f32,
        low: f32,
        depth: [DepthLevel; 5],
    },
    Disconnect {
        reason: i16,
    },
    Unknown {
        code: u8,
    },
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("buffer too short for header: expected at least 8 bytes, got {got}")]
    BufferTooShortHeader { got: usize },
    #[error("buffer too short for packet code {code}: expected {expected} bytes, got {got}")]
    BufferTooShortPacket {
        code: u8,
        expected: usize,
        got: usize,
    },
    #[error("length mismatch: header declares {expected} bytes, got buffer of {got} bytes")]
    LengthMismatch { expected: usize, got: usize },
}

#[inline]
fn read_i16(buf: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([buf[offset], buf[offset + 1]])
}

#[inline]
fn read_i32(buf: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

#[inline]
fn read_f32(buf: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

pub fn decode(buf: &[u8]) -> Result<(Header, Packet), DecodeError> {
    if buf.len() < 8 {
        return Err(DecodeError::BufferTooShortHeader { got: buf.len() });
    }

    let code = buf[0];
    let len = read_i16(buf, 1);
    let len_usize = if len < 0 { 0 } else { len as usize };
    let segment = buf[3];
    let security_id = read_i32(buf, 4);

    let header = Header {
        code,
        len: len.max(0) as u16,
        segment,
        security_id,
    };

    if buf.len() != len_usize {
        return Err(DecodeError::LengthMismatch {
            expected: len_usize,
            got: buf.len(),
        });
    }

    let packet = match code {
        2 => {
            if buf.len() < 16 {
                return Err(DecodeError::BufferTooShortPacket {
                    code,
                    expected: 16,
                    got: buf.len(),
                });
            }
            let ltp = read_f32(buf, 8);
            let ltt = read_i32(buf, 12);
            Packet::Ticker { ltp, ltt }
        }
        6 => {
            if buf.len() < 16 {
                return Err(DecodeError::BufferTooShortPacket {
                    code,
                    expected: 16,
                    got: buf.len(),
                });
            }
            let close = read_f32(buf, 8);
            let oi = read_i32(buf, 12);
            Packet::PrevClose { close, oi }
        }
        5 => {
            if buf.len() < 12 {
                return Err(DecodeError::BufferTooShortPacket {
                    code,
                    expected: 12,
                    got: buf.len(),
                });
            }
            let oi = read_i32(buf, 8);
            Packet::Oi { oi }
        }
        4 => {
            if buf.len() < 50 {
                return Err(DecodeError::BufferTooShortPacket {
                    code,
                    expected: 50,
                    got: buf.len(),
                });
            }
            let ltp = read_f32(buf, 8);
            let ltq = read_i16(buf, 12);
            let ltt = read_i32(buf, 14);
            let atp = read_f32(buf, 18);
            let volume = read_i32(buf, 22);
            let sell_qty = read_i32(buf, 26);
            let buy_qty = read_i32(buf, 30);
            let open = read_f32(buf, 34);
            let close = read_f32(buf, 38);
            let high = read_f32(buf, 42);
            let low = read_f32(buf, 46);
            Packet::Quote {
                ltp,
                ltq,
                ltt,
                atp,
                volume,
                sell_qty,
                buy_qty,
                open,
                close,
                high,
                low,
            }
        }
        8 => {
            if buf.len() < 162 {
                return Err(DecodeError::BufferTooShortPacket {
                    code,
                    expected: 162,
                    got: buf.len(),
                });
            }
            let ltp = read_f32(buf, 8);
            let ltq = read_i16(buf, 12);
            let ltt = read_i32(buf, 14);
            let atp = read_f32(buf, 18);
            let volume = read_i32(buf, 22);
            let sell_qty = read_i32(buf, 26);
            let buy_qty = read_i32(buf, 30);
            let oi = read_i32(buf, 34);
            let high_oi = read_i32(buf, 38);
            let low_oi = read_i32(buf, 42);
            let open = read_f32(buf, 46);
            let close = read_f32(buf, 50);
            let high = read_f32(buf, 54);
            let low = read_f32(buf, 58);

            let mut depth = [DepthLevel {
                bid_qty: 0,
                ask_qty: 0,
                bid_orders: 0,
                ask_orders: 0,
                bid_price: 0.0,
                ask_price: 0.0,
            }; 5];

            for (i, item) in depth.iter_mut().enumerate() {
                let off = 62 + i * 20;
                *item = DepthLevel {
                    bid_qty: read_i32(buf, off),
                    ask_qty: read_i32(buf, off + 4),
                    bid_orders: read_i16(buf, off + 8),
                    ask_orders: read_i16(buf, off + 10),
                    bid_price: read_f32(buf, off + 12),
                    ask_price: read_f32(buf, off + 16),
                };
            }

            Packet::Full {
                ltp,
                ltq,
                ltt,
                atp,
                volume,
                sell_qty,
                buy_qty,
                oi,
                high_oi,
                low_oi,
                open,
                close,
                high,
                low,
                depth,
            }
        }
        50 => {
            if buf.len() < 10 {
                return Err(DecodeError::BufferTooShortPacket {
                    code,
                    expected: 10,
                    got: buf.len(),
                });
            }
            let reason = read_i16(buf, 8);
            Packet::Disconnect { reason }
        }
        _ => Packet::Unknown { code },
    };

    Ok((header, packet))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_ticker() {
        // Test vector from spec: ticker, security 2885, segment 1, LTP 1300.8, LTT 1700000000
        let bytes: [u8; 16] = [
            0x02, 0x10, 0x00, 0x01, 0x45, 0x0B, 0x00, 0x00,
            0x9A, 0x99, 0xA2, 0x44, 0x00, 0xF1, 0x53, 0x65,
        ];
        let (hdr, pkt) = decode(&bytes).expect("decode test vector");
        assert_eq!(hdr.code, 2);
        assert_eq!(hdr.len, 16);
        assert_eq!(hdr.segment, 1);
        assert_eq!(hdr.security_id, 2885);
        match pkt {
            Packet::Ticker { ltp, ltt } => {
                assert!((ltp - 1300.8).abs() < 1e-4);
                assert_eq!(ltt, 1700000000);
            }
            _ => panic!("expected Ticker packet, got {pkt:?}"),
        }
    }

    #[test]
    fn test_prev_close() {
        let mut buf = vec![0u8; 16];
        buf[0] = 6;
        buf[1..3].copy_from_slice(&16i16.to_le_bytes());
        buf[3] = 1;
        buf[4..8].copy_from_slice(&2885i32.to_le_bytes());
        buf[8..12].copy_from_slice(&1344.8f32.to_le_bytes());
        buf[12..16].copy_from_slice(&1000i32.to_le_bytes());

        let (hdr, pkt) = decode(&buf).expect("decode prev close");
        assert_eq!(hdr.code, 6);
        assert_eq!(hdr.security_id, 2885);
        match pkt {
            Packet::PrevClose { close, oi } => {
                assert!((close - 1344.8).abs() < 1e-3);
                assert_eq!(oi, 1000);
            }
            _ => panic!("expected PrevClose"),
        }
    }

    #[test]
    fn test_oi_packet() {
        let mut buf = vec![0u8; 12];
        buf[0] = 5;
        buf[1..3].copy_from_slice(&12i16.to_le_bytes());
        buf[3] = 2;
        buf[4..8].copy_from_slice(&4567i32.to_le_bytes());
        buf[8..12].copy_from_slice(&250000i32.to_le_bytes());

        let (hdr, pkt) = decode(&buf).expect("decode OI");
        assert_eq!(hdr.code, 5);
        assert_eq!(hdr.segment, 2);
        assert_eq!(hdr.security_id, 4567);
        match pkt {
            Packet::Oi { oi } => assert_eq!(oi, 250000),
            _ => panic!("expected Oi"),
        }
    }

    #[test]
    fn test_quote_packet() {
        let mut buf = vec![0u8; 50];
        buf[0] = 4;
        buf[1..3].copy_from_slice(&50i16.to_le_bytes());
        buf[3] = 1;
        buf[4..8].copy_from_slice(&1001i32.to_le_bytes());
        buf[8..12].copy_from_slice(&500.5f32.to_le_bytes()); // ltp
        buf[12..14].copy_from_slice(&50i16.to_le_bytes());   // ltq
        buf[14..18].copy_from_slice(&1700000100i32.to_le_bytes()); // ltt
        buf[18..22].copy_from_slice(&498.2f32.to_le_bytes()); // atp
        buf[22..26].copy_from_slice(&150000i32.to_le_bytes()); // volume
        buf[26..30].copy_from_slice(&20000i32.to_le_bytes()); // total sell qty
        buf[30..34].copy_from_slice(&30000i32.to_le_bytes()); // total buy qty
        buf[34..38].copy_from_slice(&490.0f32.to_le_bytes()); // open
        buf[38..42].copy_from_slice(&505.0f32.to_le_bytes()); // close
        buf[42..46].copy_from_slice(&510.0f32.to_le_bytes()); // high
        buf[46..50].copy_from_slice(&488.0f32.to_le_bytes()); // low

        let (hdr, pkt) = decode(&buf).expect("decode quote");
        assert_eq!(hdr.code, 4);
        assert_eq!(hdr.len, 50);
        assert_eq!(hdr.security_id, 1001);
        match pkt {
            Packet::Quote {
                ltp, ltq, ltt, atp, volume, sell_qty, buy_qty, open, close, high, low,
            } => {
                assert!((ltp - 500.5).abs() < 1e-4);
                assert_eq!(ltq, 50);
                assert_eq!(ltt, 1700000100);
                assert!((atp - 498.2).abs() < 1e-4);
                assert_eq!(volume, 150000);
                assert_eq!(sell_qty, 20000);
                assert_eq!(buy_qty, 30000);
                assert!((open - 490.0).abs() < 1e-4);
                assert!((close - 505.0).abs() < 1e-4);
                assert!((high - 510.0).abs() < 1e-4);
                assert!((low - 488.0).abs() < 1e-4);
            }
            _ => panic!("expected Quote"),
        }
    }

    #[test]
    fn test_full_packet() {
        let mut buf = vec![0u8; 162];
        buf[0] = 8;
        buf[1..3].copy_from_slice(&162i16.to_le_bytes());
        buf[3] = 1;
        buf[4..8].copy_from_slice(&2002i32.to_le_bytes());
        buf[8..12].copy_from_slice(&120.0f32.to_le_bytes()); // ltp
        buf[12..14].copy_from_slice(&10i16.to_le_bytes());   // ltq
        buf[14..18].copy_from_slice(&1700000200i32.to_le_bytes()); // ltt
        buf[18..22].copy_from_slice(&119.5f32.to_le_bytes()); // atp
        buf[22..26].copy_from_slice(&8000i32.to_le_bytes()); // volume
        buf[26..30].copy_from_slice(&1000i32.to_le_bytes()); // sell qty
        buf[30..34].copy_from_slice(&2000i32.to_le_bytes()); // buy qty
        buf[34..38].copy_from_slice(&50000i32.to_le_bytes()); // oi
        buf[38..42].copy_from_slice(&60000i32.to_le_bytes()); // high oi
        buf[42..46].copy_from_slice(&40000i32.to_le_bytes()); // low oi
        buf[46..50].copy_from_slice(&118.0f32.to_le_bytes()); // open
        buf[50..54].copy_from_slice(&121.0f32.to_le_bytes()); // close
        buf[54..58].copy_from_slice(&125.0f32.to_le_bytes()); // high
        buf[58..62].copy_from_slice(&115.0f32.to_le_bytes()); // low

        // Fill 5 depth levels
        for i in 0..5 {
            let off = 62 + i * 20;
            buf[off..off + 4].copy_from_slice(&((i as i32 + 1) * 100).to_le_bytes()); // bid_qty
            buf[off + 4..off + 8].copy_from_slice(&((i as i32 + 1) * 150).to_le_bytes()); // ask_qty
            buf[off + 8..off + 10].copy_from_slice(&(i as i16 + 1).to_le_bytes()); // bid_orders
            buf[off + 10..off + 12].copy_from_slice(&(i as i16 + 2).to_le_bytes()); // ask_orders
            buf[off + 12..off + 16].copy_from_slice(&(120.0f32 - i as f32).to_le_bytes()); // bid_price
            buf[off + 16..off + 20].copy_from_slice(&(121.0f32 + i as f32).to_le_bytes()); // ask_price
        }

        let (hdr, pkt) = decode(&buf).expect("decode full");
        assert_eq!(hdr.code, 8);
        assert_eq!(hdr.len, 162);
        match pkt {
            Packet::Full {
                ltp, oi, depth, ..
            } => {
                assert!((ltp - 120.0).abs() < 1e-4);
                assert_eq!(oi, 50000);
                assert_eq!(depth[0].bid_qty, 100);
                assert_eq!(depth[0].ask_qty, 150);
                assert_eq!(depth[0].bid_orders, 1);
                assert_eq!(depth[0].ask_orders, 2);
                assert!((depth[0].bid_price - 120.0).abs() < 1e-4);
                assert!((depth[0].ask_price - 121.0).abs() < 1e-4);

                assert_eq!(depth[4].bid_qty, 500);
                assert!((depth[4].bid_price - 116.0).abs() < 1e-4);
            }
            _ => panic!("expected Full"),
        }
    }

    #[test]
    fn test_disconnect_packet() {
        let mut buf = vec![0u8; 10];
        buf[0] = 50;
        buf[1..3].copy_from_slice(&10i16.to_le_bytes());
        buf[3] = 0;
        buf[4..8].copy_from_slice(&0i32.to_le_bytes());
        buf[8..10].copy_from_slice(&805i16.to_le_bytes());

        let (hdr, pkt) = decode(&buf).expect("decode disconnect");
        assert_eq!(hdr.code, 50);
        match pkt {
            Packet::Disconnect { reason } => assert_eq!(reason, 805),
            _ => panic!("expected Disconnect"),
        }
    }

    #[test]
    fn test_unknown_code() {
        let mut buf = vec![0u8; 8];
        buf[0] = 99;
        buf[1..3].copy_from_slice(&8i16.to_le_bytes());
        buf[3] = 1;
        buf[4..8].copy_from_slice(&123i32.to_le_bytes());

        let (hdr, pkt) = decode(&buf).expect("decode unknown code");
        assert_eq!(hdr.code, 99);
        match pkt {
            Packet::Unknown { code } => assert_eq!(code, 99),
            _ => panic!("expected Unknown"),
        }
    }

    #[test]
    fn test_truncated_buffers() {
        // Truncate at every length 0..7 (header)
        for len in 0..8 {
            let buf = vec![0u8; len];
            let err = decode(&buf).unwrap_err();
            assert_eq!(err, DecodeError::BufferTooShortHeader { got: len });
        }

        // Length declared as 16, but buffer shorter
        for len in 8..16 {
            let mut buf = vec![0u8; len];
            buf[0] = 2;
            buf[1..3].copy_from_slice(&16i16.to_le_bytes());
            let err = decode(&buf).unwrap_err();
            assert_eq!(err, DecodeError::LengthMismatch { expected: 16, got: len });
        }
    }

    #[test]
    fn test_length_mismatch() {
        // Buffer has 18 bytes, but header length field says 16
        let mut buf = vec![0u8; 18];
        buf[0] = 2;
        buf[1..3].copy_from_slice(&16i16.to_le_bytes());
        let err = decode(&buf).unwrap_err();
        assert_eq!(err, DecodeError::LengthMismatch { expected: 16, got: 18 });
    }
}

