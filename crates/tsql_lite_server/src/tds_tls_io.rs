use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

const TDS_HEADER_SIZE: usize = 8;
const TDS_PRELOGIN: u8 = 0x12;
const TDS_STATUS_EOM: u8 = 0x01;

/// IO adapter that wraps/unwraps TDS framing around TLS handshake bytes.
///
/// During TLS handshake, TLS records are tunneled inside TDS PRELOGIN (0x12)
/// packets. After handshake completes, call `set_raw_mode()` to switch to
/// direct passthrough — post-handshake traffic is raw TLS over TCP.
pub struct TdsTlsIo {
    stream: TcpStream,
    raw_mode: Arc<AtomicBool>,
    // Read buffering
    read_buf: Vec<u8>,
    read_pos: usize,
    // Header reading state
    header_buf: [u8; TDS_HEADER_SIZE],
    header_read: usize,
    payload_remaining: usize,
    reading_header: bool,
    // Write buffering for partial writes
    write_buf: Vec<u8>,
    write_pos: usize,
}

impl TdsTlsIo {
    pub fn new(stream: TcpStream, raw_mode: Arc<AtomicBool>) -> Self {
        Self {
            stream,
            raw_mode,
            read_buf: Vec::with_capacity(4096),
            read_pos: 0,
            header_buf: [0u8; TDS_HEADER_SIZE],
            header_read: 0,
            payload_remaining: 0,
            reading_header: true,
            write_buf: Vec::new(),
            write_pos: 0,
        }
    }

    pub fn raw_mode_flag(&self) -> Arc<AtomicBool> {
        self.raw_mode.clone()
    }
}

impl AsyncRead for TdsTlsIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        // Always drain any already-buffered bytes first, even after raw_mode flips true.
        if this.read_pos < this.read_buf.len() {
            let available = &this.read_buf[this.read_pos..];
            let to_copy = available.len().min(buf.remaining());
            buf.put_slice(&available[..to_copy]);
            this.read_pos += to_copy;
            if this.read_pos >= this.read_buf.len() {
                this.read_buf.clear();
                this.read_pos = 0;
            }
            return Poll::Ready(Ok(()));
        }

        if this.raw_mode.load(Ordering::Acquire) {
            return Pin::new(&mut this.stream).poll_read(cx, buf);
        }

        // Read TDS PRELOGIN (0x12) header. If the first byte is not 0x12 (e.g. a
        // raw TLS 1.3 Application Data 0x17 record pipelined immediately after
        // the client's Finished handshake message), switch to raw passthrough
        // immediately without stripping or discarding those bytes.
        if this.reading_header {
            while this.header_read < TDS_HEADER_SIZE {
                let mut tmp = ReadBuf::new(&mut this.header_buf[this.header_read..]);
                match Pin::new(&mut this.stream).poll_read(cx, &mut tmp) {
                    Poll::Ready(Ok(())) => {
                        let n = tmp.filled().len();
                        if n == 0 {
                            if this.header_read == 0 {
                                return Poll::Ready(Ok(()));
                            }
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "TdsTlsIo: EOF reading TDS header",
                            )));
                        }
                        this.header_read += n;
                        if this.header_buf[0] != TDS_PRELOGIN {
                            let already_read = this.header_read;
                            this.header_read = 0;
                            this.reading_header = true;
                            this.raw_mode.store(true, Ordering::Release);
                            log::debug!(
                                "TdsTlsIo READ: non-PRELOGIN first byte 0x{:02X} ({} bytes), switching to raw TLS mode",
                                this.header_buf[0],
                                already_read
                            );
                            let to_copy = already_read.min(buf.remaining());
                            buf.put_slice(&this.header_buf[..to_copy]);
                            if to_copy < already_read {
                                this.read_buf.clear();
                                this.read_buf
                                    .extend_from_slice(&this.header_buf[to_copy..already_read]);
                                this.read_pos = 0;
                            }
                            return Poll::Ready(Ok(()));
                        }
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
            let pkt_type = this.header_buf[0];
            let total_len = u16::from_be_bytes([this.header_buf[2], this.header_buf[3]]) as usize;
            let payload_len = total_len.saturating_sub(TDS_HEADER_SIZE);
            log::debug!(
                "TdsTlsIo READ: TDS type=0x{:02X} total={} payload={}",
                pkt_type,
                total_len,
                payload_len
            );

            this.read_buf.clear();
            this.read_buf.resize(payload_len, 0);
            this.read_pos = 0;
            this.payload_remaining = payload_len;
            this.reading_header = false;
            this.header_read = 0;
        }

        // Read payload
        while this.payload_remaining > 0 {
            let offset = this.read_buf.len() - this.payload_remaining;
            let mut tmp = ReadBuf::new(&mut this.read_buf[offset..]);
            match Pin::new(&mut this.stream).poll_read(cx, &mut tmp) {
                Poll::Ready(Ok(())) => {
                    let n = tmp.filled().len();
                    if n == 0 {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "TdsTlsIo: EOF reading TDS payload",
                        )));
                    }
                    this.payload_remaining -= n;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        this.reading_header = true;
        this.read_pos = 0;

        // Return payload data
        let to_copy = this.read_buf.len().min(buf.remaining());
        buf.put_slice(&this.read_buf[..to_copy]);
        this.read_pos = to_copy;
        if this.read_pos >= this.read_buf.len() {
            this.read_buf.clear();
            this.read_pos = 0;
        }

        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for TdsTlsIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        // Always drain any pending TDS-wrapped handshake write before raw writes.
        if !this.write_buf.is_empty() {
            while this.write_pos < this.write_buf.len() {
                match Pin::new(&mut this.stream).poll_write(cx, &this.write_buf[this.write_pos..]) {
                    Poll::Ready(Ok(n)) => {
                        this.write_pos += n;
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
            this.write_buf.clear();
            this.write_pos = 0;
        }

        if this.raw_mode.load(Ordering::Acquire) {
            return Pin::new(&mut this.stream).poll_write(cx, buf);
        }

        // Wrap in TDS PRELOGIN packet
        log::debug!("TdsTlsIo WRITE: wrapping {} bytes in TDS 0x12", buf.len());
        let total_len = TDS_HEADER_SIZE + buf.len();
        this.write_buf = Vec::with_capacity(total_len);
        this.write_buf.push(TDS_PRELOGIN);
        this.write_buf.push(TDS_STATUS_EOM);
        this.write_buf
            .extend_from_slice(&(total_len as u16).to_be_bytes());
        this.write_buf.extend_from_slice(&[0u8; 2]); // spid
        this.write_buf.push(1); // packet_id
        this.write_buf.push(0); // window
        this.write_buf.extend_from_slice(buf);
        this.write_pos = 0;

        // Try to write as much as possible
        while this.write_pos < this.write_buf.len() {
            match Pin::new(&mut this.stream).poll_write(cx, &this.write_buf[this.write_pos..]) {
                Poll::Ready(Ok(n)) => {
                    this.write_pos += n;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {
                    if this.write_pos > 0 {
                        // Partial write — report all data consumed since we buffered it
                        return Poll::Ready(Ok(buf.len()));
                    }
                    return Poll::Pending;
                }
            }
        }

        this.write_buf.clear();
        this.write_pos = 0;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        while this.write_pos < this.write_buf.len() {
            match Pin::new(&mut this.stream).poll_write(cx, &this.write_buf[this.write_pos..]) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "TdsTlsIo: zero-byte write flushing handshake buffer",
                    )));
                }
                Poll::Ready(Ok(n)) => {
                    this.write_pos += n;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        this.write_buf.clear();
        this.write_pos = 0;
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client_fut = TcpStream::connect(addr);
        let server_fut = listener.accept();
        let (client, server) = tokio::join!(client_fut, server_fut);
        (client.unwrap(), server.unwrap().0)
    }

    fn wrap_tds_prelogin(payload: &[u8]) -> Vec<u8> {
        let total_len = TDS_HEADER_SIZE + payload.len();
        let mut frame = Vec::with_capacity(total_len);
        frame.push(TDS_PRELOGIN);
        frame.push(TDS_STATUS_EOM);
        frame.extend_from_slice(&(total_len as u16).to_be_bytes());
        frame.extend_from_slice(&[0u8; 2]);
        frame.push(1);
        frame.push(0);
        frame.extend_from_slice(payload);
        frame
    }

    #[tokio::test]
    async fn tds_tls_io_preserves_pipelined_raw_tls_app_data_before_raw_mode_flag() {
        let (mut peer, server_stream) = loopback_pair().await;
        let raw_mode = Arc::new(AtomicBool::new(false));
        let mut io = TdsTlsIo::new(server_stream, raw_mode.clone());

        let handshake_record = b"\x16\x03\x03\x00\x05hello";
        // Simulated TLS 1.3 Application Data record (0x17 0x03 0x03 <len> <ciphertext>)
        // pipelined right after Finished with NO 8-byte TDS 0x12 header.
        let mut raw_app_data = vec![0x17, 0x03, 0x03, 0x00, 0x10];
        raw_app_data.extend_from_slice(b"0123456789abcdef");

        let mut wire = wrap_tds_prelogin(handshake_record);
        wire.extend_from_slice(&raw_app_data);
        peer.write_all(&wire).await.unwrap();
        peer.shutdown().await.unwrap();

        let mut first = vec![0u8; handshake_record.len()];
        io.read_exact(&mut first).await.unwrap();
        assert_eq!(first, handshake_record);
        assert!(
            !raw_mode.load(Ordering::Acquire),
            "raw_mode should still be false after TDS-wrapped handshake record"
        );

        // Next read occurs BEFORE the caller flips raw_mode (as happens inside rustls accept()
        // when TLS 1.3 Finished and LOGIN7 arrive in the same TCP burst).
        let mut rest = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(2), io.read_to_end(&mut rest))
            .await
            .expect("TdsTlsIo must not hang on raw TLS Application Data")
            .expect("TdsTlsIo must read raw TLS Application Data without error");

        assert_eq!(rest, raw_app_data);
        assert!(
            raw_mode.load(Ordering::Acquire),
            "TdsTlsIo should auto-switch to raw_mode upon seeing non-0x12 TLS record"
        );
    }

    #[tokio::test]
    async fn tds_tls_io_drains_buffered_bytes_after_raw_mode_flip() {
        let (mut peer, server_stream) = loopback_pair().await;
        let raw_mode = Arc::new(AtomicBool::new(false));
        let mut io = TdsTlsIo::new(server_stream, raw_mode.clone());

        let handshake_record = b"ABCDEFGHIJ";
        let raw_tail = b"\x17\x03\x03\x00\x04TAIL";

        let mut wire = wrap_tds_prelogin(handshake_record);
        wire.extend_from_slice(raw_tail);
        peer.write_all(&wire).await.unwrap();
        peer.shutdown().await.unwrap();

        let mut part1 = [0u8; 4];
        io.read_exact(&mut part1).await.unwrap();
        assert_eq!(&part1, b"ABCD");

        // Caller flips raw_mode while 6 bytes of the unwrapped frame remain in read_buf.
        raw_mode.store(true, Ordering::Release);

        let mut rest = Vec::new();
        io.read_to_end(&mut rest).await.unwrap();
        let mut expected = b"EFGHIJ".to_vec();
        expected.extend_from_slice(raw_tail);
        assert_eq!(rest, expected);
    }
}
