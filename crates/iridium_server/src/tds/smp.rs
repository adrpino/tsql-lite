//! `[MC-SMP]` Session Multiplex Protocol (TDS MARS) framing and per-session state.

use std::collections::{HashMap, VecDeque};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::packet::{PacketHeader, HEADER_SIZE, STATUS_EOM};

pub const SMP_MARKER: u8 = 0x53;
pub const SMP_HEADER_SIZE: usize = 16;

pub const SMP_SYN: u8 = 0x01;
pub const SMP_ACK: u8 = 0x02;
pub const SMP_FIN: u8 = 0x04;
pub const SMP_DATA: u8 = 0x08;

const DEFAULT_WINDOW_CREDIT: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmpHeader {
    pub flags: u8,
    pub sid: u16,
    pub length: u32,
    pub seqnum: u32,
    pub wndw: u32,
}

impl SmpHeader {
    pub fn from_bytes(buf: &[u8; SMP_HEADER_SIZE]) -> io::Result<Self> {
        if buf[0] != SMP_MARKER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Expected SMP marker 0x53, got 0x{:02X}", buf[0]),
            ));
        }
        let flags = buf[1];
        let sid = u16::from_le_bytes([buf[2], buf[3]]);
        let length = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let seqnum = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let wndw = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        if (length as usize) < SMP_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid SMP frame length {}", length),
            ));
        }
        Ok(Self {
            flags,
            sid,
            length,
            seqnum,
            wndw,
        })
    }

    pub fn to_bytes(self) -> [u8; SMP_HEADER_SIZE] {
        let mut buf = [0u8; SMP_HEADER_SIZE];
        buf[0] = SMP_MARKER;
        buf[1] = self.flags;
        buf[2..4].copy_from_slice(&self.sid.to_le_bytes());
        buf[4..8].copy_from_slice(&self.length.to_le_bytes());
        buf[8..12].copy_from_slice(&self.seqnum.to_le_bytes());
        buf[12..16].copy_from_slice(&self.wndw.to_le_bytes());
        buf
    }
}

pub async fn read_smp_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> io::Result<(SmpHeader, Vec<u8>)> {
    let mut hdr_buf = [0u8; SMP_HEADER_SIZE];
    reader.read_exact(&mut hdr_buf).await?;
    let header = SmpHeader::from_bytes(&hdr_buf)?;
    let payload_len = (header.length as usize) - SMP_HEADER_SIZE;
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        reader.read_exact(&mut payload).await?;
    }
    Ok((header, payload))
}

pub async fn write_smp_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    header: SmpHeader,
    payload: &[u8],
) -> io::Result<()> {
    let mut h = header;
    h.length = (SMP_HEADER_SIZE + payload.len()) as u32;
    writer.write_all(&h.to_bytes()).await?;
    if !payload.is_empty() {
        writer.write_all(payload).await?;
    }
    writer.flush().await?;
    Ok(())
}

/// Split a concatenated buffer of framed TDS packets (produced by `packet::write_packet`)
/// back into individual wire TDS packets so each can be wrapped in its own `SMP_DATA` frame.
pub fn split_framed_tds_packets(mut buf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while buf.len() >= HEADER_SIZE {
        let mut hdr_arr = [0u8; HEADER_SIZE];
        hdr_arr.copy_from_slice(&buf[..HEADER_SIZE]);
        let hdr = PacketHeader::from_bytes(&hdr_arr);
        let pkt_len = (hdr.length as usize).max(HEADER_SIZE).min(buf.len());
        out.push(buf[..pkt_len].to_vec());
        buf = &buf[pkt_len..];
    }
    out
}

#[derive(Debug, Default)]
pub struct SmpLogicalSession {
    pub sid: u16,
    pub server_tx_seq: u32,
    pub client_rx_seq: u32,
    pub client_wndw: u32,
    pub pending_header: Option<PacketHeader>,
    pub pending_data: Vec<u8>,
    pub pending_outbound: VecDeque<Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct SmpMultiplexer {
    sessions: HashMap<u16, SmpLogicalSession>,
}

impl SmpMultiplexer {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn on_syn(&mut self, hdr: SmpHeader) -> SmpHeader {
        let sess = SmpLogicalSession {
            sid: hdr.sid,
            server_tx_seq: 0,
            client_rx_seq: 0,
            client_wndw: hdr.wndw.max(DEFAULT_WINDOW_CREDIT),
            pending_header: None,
            pending_data: Vec::new(),
            pending_outbound: VecDeque::new(),
        };
        self.sessions.insert(hdr.sid, sess);
        SmpHeader {
            flags: SMP_ACK,
            sid: hdr.sid,
            length: SMP_HEADER_SIZE as u32,
            seqnum: 0,
            wndw: DEFAULT_WINDOW_CREDIT,
        }
    }

    pub fn on_fin(&mut self, hdr: SmpHeader) -> SmpHeader {
        let (tx_seq, rx_seq) = self
            .sessions
            .remove(&hdr.sid)
            .map(|s| (s.server_tx_seq, s.client_rx_seq))
            .unwrap_or((0, 0));
        SmpHeader {
            flags: SMP_FIN,
            sid: hdr.sid,
            length: SMP_HEADER_SIZE as u32,
            seqnum: tx_seq,
            wndw: rx_seq + DEFAULT_WINDOW_CREDIT,
        }
    }

    pub fn on_ack(&mut self, hdr: SmpHeader) -> Vec<(SmpHeader, Vec<u8>)> {
        let Some(sess) = self.sessions.get_mut(&hdr.sid) else {
            return Vec::new();
        };
        sess.client_wndw = hdr.wndw;
        drain_within_window(sess)
    }

    /// Feed an `SMP_DATA` payload (one TDS packet). Returns:
    /// - `Ok(Some((tds_header, full_message)))` when `STATUS_EOM` completes the TDS message,
    /// - `Ok(None)` when more fragments are expected (caller should send the optional ACK).
    pub fn on_data(
        &mut self,
        hdr: SmpHeader,
        payload: &[u8],
    ) -> io::Result<(Option<(PacketHeader, Vec<u8>)>, Option<SmpHeader>)> {
        let sess = self
            .sessions
            .entry(hdr.sid)
            .or_insert_with(|| SmpLogicalSession {
                sid: hdr.sid,
                client_wndw: hdr.wndw.max(DEFAULT_WINDOW_CREDIT),
                ..Default::default()
            });
        sess.client_rx_seq = hdr.seqnum;
        sess.client_wndw = hdr.wndw;

        if payload.len() < HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SMP DATA payload shorter than TDS header",
            ));
        }
        let mut tds_hdr_arr = [0u8; HEADER_SIZE];
        tds_hdr_arr.copy_from_slice(&payload[..HEADER_SIZE]);
        let tds_hdr = PacketHeader::from_bytes(&tds_hdr_arr);
        let tds_body_len = (tds_hdr.length as usize)
            .saturating_sub(HEADER_SIZE)
            .min(payload.len() - HEADER_SIZE);
        let tds_body = &payload[HEADER_SIZE..HEADER_SIZE + tds_body_len];

        if sess.pending_header.is_none() {
            sess.pending_header = Some(tds_hdr.clone());
        }
        sess.pending_data.extend_from_slice(tds_body);

        if (tds_hdr.status & STATUS_EOM) != 0 {
            let mut first_hdr = sess.pending_header.take().unwrap_or(tds_hdr.clone());
            first_hdr.status = tds_hdr.status;
            let data = std::mem::take(&mut sess.pending_data);
            Ok((Some((first_hdr, data)), None))
        } else {
            let ack = SmpHeader {
                flags: SMP_ACK,
                sid: hdr.sid,
                length: SMP_HEADER_SIZE as u32,
                seqnum: sess.server_tx_seq,
                wndw: sess.client_rx_seq + DEFAULT_WINDOW_CREDIT,
            };
            Ok((None, Some(ack)))
        }
    }

    pub fn enqueue_response(
        &mut self,
        sid: u16,
        framed_tds_buf: &[u8],
    ) -> Vec<(SmpHeader, Vec<u8>)> {
        let Some(sess) = self.sessions.get_mut(&sid) else {
            return Vec::new();
        };
        for pkt in split_framed_tds_packets(framed_tds_buf) {
            sess.pending_outbound.push_back(pkt);
        }
        drain_within_window(sess)
    }
}

fn drain_within_window(sess: &mut SmpLogicalSession) -> Vec<(SmpHeader, Vec<u8>)> {
    let mut ready = Vec::new();
    while sess.server_tx_seq < sess.client_wndw {
        let Some(pkt) = sess.pending_outbound.pop_front() else {
            break;
        };
        sess.server_tx_seq = sess.server_tx_seq.wrapping_add(1);
        let hdr = SmpHeader {
            flags: SMP_DATA,
            sid: sess.sid,
            length: (SMP_HEADER_SIZE + pkt.len()) as u32,
            seqnum: sess.server_tx_seq,
            wndw: sess.client_rx_seq + DEFAULT_WINDOW_CREDIT,
        };
        ready.push((hdr, pkt));
    }
    ready
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smp_header_roundtrip() {
        let h = SmpHeader {
            flags: SMP_DATA,
            sid: 3,
            length: 48,
            seqnum: 7,
            wndw: 11,
        };
        let bytes = h.to_bytes();
        assert_eq!(bytes[0], SMP_MARKER);
        assert_eq!(SmpHeader::from_bytes(&bytes).unwrap(), h);
    }

    #[test]
    fn smp_syn_data_window_and_fin() {
        let mut mux = SmpMultiplexer::new();
        let ack = mux.on_syn(SmpHeader {
            flags: SMP_SYN,
            sid: 1,
            length: 16,
            seqnum: 0,
            wndw: 4,
        });
        assert_eq!(ack.flags, SMP_ACK);
        assert_eq!(ack.sid, 1);
        assert_eq!(ack.wndw, 4);

        let tds_hdr = PacketHeader::new(0x01, 10).to_bytes();
        let mut pkt = tds_hdr.to_vec();
        pkt.extend_from_slice(&[0x41, 0x00]);
        let (msg, mid_ack) = mux
            .on_data(
                SmpHeader {
                    flags: SMP_DATA,
                    sid: 1,
                    length: (16 + pkt.len()) as u32,
                    seqnum: 1,
                    wndw: 4,
                },
                &pkt,
            )
            .unwrap();
        assert!(mid_ack.is_none());
        let (hdr, body) = msg.unwrap();
        assert_eq!(hdr.packet_type, 0x01);
        assert_eq!(body, vec![0x41, 0x00]);

        let fin = mux.on_fin(SmpHeader {
            flags: SMP_FIN,
            sid: 1,
            length: 16,
            seqnum: 1,
            wndw: 4,
        });
        assert_eq!(fin.flags, SMP_FIN);
    }
}
