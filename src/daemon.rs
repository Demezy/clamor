use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::protocol::{
    recv_message_async, send_message_async, ClientMessage, DaemonAgent, DaemonMessage,
};

/// Buffers output between DEC 2026 synchronized update markers (BSU/ESU).
///
/// vt100 0.16.x doesn't support mode 2026. Claude Code (Ink) wraps each render
/// in `\x1b[?2026h` (BSU) and `\x1b[?2026l` (ESU). Instead of stripping them
/// and forwarding partial frames (which causes prompt jumping), we buffer all
/// output between BSU and ESU and forward the complete render atomically.
///
/// Handles markers split across PTY read boundaries: trailing bytes that could
/// be the start of a marker are saved and prepended to the next call.
struct SyncOutputBuffer {
    buf: Vec<u8>,
    syncing: bool,
    /// Trailing bytes from the previous read that could be a marker prefix.
    trail: Vec<u8>,
}

/// The 7-byte prefix shared by BSU (`\x1b[?2026h`) and ESU (`\x1b[?2026l`).
const SYNC_MARKER_PREFIX: &[u8] = b"\x1b[?2026";

impl SyncOutputBuffer {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            syncing: false,
            trail: Vec::new(),
        }
    }

    /// Process incoming PTY data. Returns output chunks to forward.
    ///
    /// Outside BSU/ESU: passes through immediately.
    /// Inside BSU/ESU: buffers until ESU, then emits the complete frame.
    fn process(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        // Prepend any trailing bytes from the previous call.
        let mut combined_buf;
        let input = if self.trail.is_empty() {
            data
        } else {
            combined_buf = std::mem::take(&mut self.trail);
            combined_buf.extend_from_slice(data);
            &combined_buf
        };

        let mut outputs = Vec::new();
        let mut passthrough = Vec::new();
        let mut i = 0;

        while i < input.len() {
            if i + 8 <= input.len() {
                let window = &input[i..i + 8];
                if window == b"\x1b[?2026h" {
                    // BSU: flush any passthrough, start buffering
                    if !self.syncing && !passthrough.is_empty() {
                        outputs.push(std::mem::take(&mut passthrough));
                    }
                    self.syncing = true;
                    i += 8;
                    continue;
                }
                if window == b"\x1b[?2026l" {
                    // ESU: flush the synchronized frame
                    if self.syncing {
                        self.buf.extend_from_slice(&passthrough);
                        passthrough.clear();
                        if !self.buf.is_empty() {
                            outputs.push(std::mem::take(&mut self.buf));
                        }
                        self.syncing = false;
                    }
                    i += 8;
                    continue;
                }
            } else if input[i] == 0x1b {
                // Fewer than 8 bytes remaining, starting with ESC.
                // Check if they could be the start of a BSU/ESU marker.
                let remaining = &input[i..];
                if SYNC_MARKER_PREFIX.starts_with(remaining) {
                    // Potential marker prefix — save for next call.
                    if !passthrough.is_empty() {
                        if self.syncing {
                            self.buf.extend(std::mem::take(&mut passthrough));
                        } else {
                            outputs.push(std::mem::take(&mut passthrough));
                        }
                    }
                    self.trail = remaining.to_vec();
                    return outputs;
                }
            }
            passthrough.push(input[i]);
            i += 1;
        }

        if !passthrough.is_empty() {
            if self.syncing {
                self.buf.extend(passthrough);
            } else {
                outputs.push(passthrough);
            }
        }

        outputs
    }
}

pub fn daemon_socket_path() -> Result<PathBuf> {
    Ok(crate::config::ClamorConfig::runtime_dir()?.join("clamor.sock"))
}

pub fn daemon_pid_path() -> Result<PathBuf> {
    Ok(crate::config::ClamorConfig::runtime_dir()?.join("clamor.pid"))
}

pub fn daemon_log_path() -> Result<PathBuf> {
    Ok(crate::config::ClamorConfig::runtime_dir()?.join("daemon.log"))
}

/// Append a timestamped line to the daemon log. Best-effort; never fails the caller.
fn daemon_log(msg: &str) {
    let Ok(path) = daemon_log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = chrono::Utc::now().to_rfc3339();
        let pid = std::process::id();
        let _ = writeln!(f, "{ts} pid={pid} {msg}");
    }
}

pub fn is_daemon_running() -> bool {
    let pid_path = match daemon_pid_path() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    unsafe { libc::kill(pid, 0) == 0 }
}

pub fn start_daemon_background() -> Result<()> {
    let exe = std::env::current_exe().context("resolving clamor executable path")?;

    // Send daemon stderr to the log file so panics and `eprintln!`s are not
    // silently dropped. Fall back to /dev/null only if the log file can't be
    // opened.
    let stderr = match daemon_log_path() {
        Ok(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map(std::process::Stdio::from)
                .unwrap_or_else(|_| std::process::Stdio::null())
        }
        Err(_) => std::process::Stdio::null(),
    };

    std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .spawn()
        .context("spawning daemon process")?;

    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

enum PtyEvent {
    /// Raw data from PTY reader. All processing (query detection, sync buffering,
    /// parser updates, CPR responses) happens daemon-side for correct ordering.
    RawData {
        id: String,
        data: Vec<u8>,
    },
    Exited {
        id: String,
    },
}

/// Detects terminal capability queries in PTY output and generates responses.
///
/// Claude Code sends DA1, DSR, and DECRQM queries to detect terminal capabilities.
/// Without responses, it may fall back to degraded rendering paths.
struct TerminalQueryResponder {
    partial: Vec<u8>,
    cpr_requested: bool,
}

impl TerminalQueryResponder {
    fn new() -> Self {
        Self {
            partial: Vec::new(),
            cpr_requested: false,
        }
    }

    /// Scan output data for terminal queries and return responses to write back.
    /// CPR (cursor position) queries set `cpr_requested` — the caller handles
    /// the response after feeding the parser up to the CPR byte offset.
    fn scan_for_queries(&mut self, data: &[u8]) -> Vec<u8> {
        self.cpr_requested = false;
        let mut responses = Vec::new();
        let mut combined = std::mem::take(&mut self.partial);
        combined.extend_from_slice(data);

        let mut i = 0;
        while i < combined.len() {
            if combined[i] == 0x1b {
                if i + 1 >= combined.len() {
                    // Lone ESC at end — could be start of any escape sequence
                    self.partial = combined[i..].to_vec();
                    return responses;
                }
                if combined[i + 1] == b'[' {
                    // CPR check: ESC [ 6 n — set flag for deferred response
                    if i + 3 < combined.len() && combined[i + 2] == b'6' && combined[i + 3] == b'n'
                    {
                        self.cpr_requested = true;
                        i += 4;
                        continue;
                    }
                    if let Some((seq_len, response)) = Self::parse_csi_query(&combined[i..]) {
                        if let Some(resp) = response {
                            responses.extend_from_slice(&resp);
                        }
                        i += seq_len;
                        continue;
                    } else {
                        // Incomplete sequence at end — buffer for next call
                        self.partial = combined[i..].to_vec();
                        return responses;
                    }
                }
            }
            i += 1;
        }

        responses
    }

    /// Try to parse a CSI query. Returns (length, optional_response).
    /// Returns None if the sequence appears incomplete.
    fn parse_csi_query(data: &[u8]) -> Option<(usize, Option<Vec<u8>>)> {
        if data.len() < 3 {
            return None;
        }

        // DA1: ESC [ c
        if data[2] == b'c' {
            return Some((3, Some(b"\x1b[?62;22c".to_vec())));
        }
        // DA1: ESC [ 0 c
        if data.len() >= 4 && data[2] == b'0' && data[3] == b'c' {
            return Some((4, Some(b"\x1b[?62;22c".to_vec())));
        }

        // DSR CPR (ESC [ 6 n) is handled in scan_for_queries via cpr_requested flag.

        // DECRQM: ESC [ ? <digits> $ p
        if data.len() >= 4 && data[2] == b'?' {
            for j in 3..data.len().min(20) {
                if data[j] == b'$' && j + 1 < data.len() && data[j + 1] == b'p' {
                    let mode_str = std::str::from_utf8(&data[3..j]).unwrap_or("");
                    let mode_num = mode_str.parse::<u32>().unwrap_or(0);
                    // Report mode 2026 (synchronized output) as supported
                    let status = if mode_num == 2026 { 1 } else { 0 };
                    let resp = format!("\x1b[?{};{}$y", mode_num, status);
                    return Some((j + 2, Some(resp.into_bytes())));
                }
                if !data[j].is_ascii_digit() && data[j] != b'$' {
                    return Some((1, None)); // Not a query we handle
                }
            }
            return None; // Possibly incomplete
        }

        // Unknown CSI — scan for a final byte (0x40-0x7E) to determine
        // if the sequence is complete. Without a final byte, it could be
        // a partially-received query (e.g. \x1b[6 waiting for 'n').
        for (j, &b) in data.iter().enumerate().take(64).skip(2) {
            if (0x40..=0x7e).contains(&b) {
                return Some((j + 1, None)); // Complete non-query, skip it
            }
        }
        None // No final byte yet — incomplete
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;

    fn responder() -> TerminalQueryResponder {
        TerminalQueryResponder::new()
    }

    // ── CPR detection across split boundaries ──────────────────────────

    #[test]
    fn cpr_not_split() {
        let mut r = responder();
        let _ = r.scan_for_queries(b"Hello\x1b[6n world");
        assert!(r.cpr_requested);
    }

    #[test]
    fn cpr_split_esc() {
        // \x1b | [6n
        let mut r = responder();
        let _ = r.scan_for_queries(b"Hello\x1b");
        assert!(!r.cpr_requested);
        assert!(!r.partial.is_empty(), "lone ESC must be saved");
        let _ = r.scan_for_queries(b"[6n rest");
        assert!(r.cpr_requested);
    }

    #[test]
    fn cpr_split_esc_bracket() {
        // \x1b[ | 6n
        let mut r = responder();
        let _ = r.scan_for_queries(b"Hello\x1b[");
        assert!(!r.cpr_requested);
        assert!(!r.partial.is_empty());
        let _ = r.scan_for_queries(b"6n rest");
        assert!(r.cpr_requested);
    }

    #[test]
    fn cpr_split_esc_bracket_6() {
        // \x1b[6 | n — the originally-broken case
        let mut r = responder();
        let _ = r.scan_for_queries(b"Hello\x1b[6");
        assert!(!r.cpr_requested);
        assert!(!r.partial.is_empty(), "partial must be saved");
        let _ = r.scan_for_queries(b"n world");
        assert!(r.cpr_requested);
    }

    #[test]
    fn cpr_at_very_end_of_data() {
        let mut r = responder();
        let _ = r.scan_for_queries(b"\x1b[6n");
        assert!(r.cpr_requested);
        assert!(r.partial.is_empty());
    }

    // ── DA1 detection ──────────────────────────────────────────────────

    #[test]
    fn da1_basic() {
        let mut r = responder();
        let resp = r.scan_for_queries(b"\x1b[c");
        assert_eq!(resp, b"\x1b[?62;22c");
    }

    #[test]
    fn da1_variant_0c() {
        let mut r = responder();
        let resp = r.scan_for_queries(b"\x1b[0c");
        assert_eq!(resp, b"\x1b[?62;22c");
    }

    #[test]
    fn da1_split_esc_bracket() {
        // \x1b[ | c — partial saved, then completed
        let mut r = responder();
        let resp1 = r.scan_for_queries(b"\x1b[");
        assert!(resp1.is_empty(), "no response yet");
        assert!(!r.partial.is_empty());
        let resp2 = r.scan_for_queries(b"c");
        assert_eq!(resp2, b"\x1b[?62;22c");
    }

    #[test]
    fn da1_split_esc() {
        // \x1b | [c
        let mut r = responder();
        let _ = r.scan_for_queries(b"text\x1b");
        assert!(!r.partial.is_empty());
        let resp = r.scan_for_queries(b"[c more");
        assert_eq!(resp, b"\x1b[?62;22c");
    }

    // ── DECRQM detection ───────────────────────────────────────────────

    #[test]
    fn decrqm_mode_2026() {
        let mut r = responder();
        let resp = r.scan_for_queries(b"\x1b[?2026$p");
        // Mode 2026 → status 1 (supported)
        assert_eq!(resp, b"\x1b[?2026;1$y");
    }

    #[test]
    fn decrqm_unknown_mode() {
        let mut r = responder();
        let resp = r.scan_for_queries(b"\x1b[?9999$p");
        // Unknown mode → status 0
        assert_eq!(resp, b"\x1b[?9999;0$y");
    }

    #[test]
    fn decrqm_split_at_dollar() {
        // \x1b[?2026$ | p — partial saved at $, completed next read
        let mut r = responder();
        let resp1 = r.scan_for_queries(b"\x1b[?2026$");
        assert!(resp1.is_empty());
        assert!(!r.partial.is_empty());
        let resp2 = r.scan_for_queries(b"p");
        assert_eq!(resp2, b"\x1b[?2026;1$y");
    }

    #[test]
    fn decrqm_split_mid_digits() {
        // \x1b[?20 | 26$p
        let mut r = responder();
        let resp1 = r.scan_for_queries(b"\x1b[?20");
        assert!(resp1.is_empty());
        assert!(!r.partial.is_empty());
        let resp2 = r.scan_for_queries(b"26$p");
        assert_eq!(resp2, b"\x1b[?2026;1$y");
    }

    // ── Multiple queries in one read ───────────────────────────────────

    #[test]
    fn multiple_queries_one_read() {
        let mut r = responder();
        // DA1 + CPR + DECRQM all in one chunk
        let resp = r.scan_for_queries(b"\x1b[c\x1b[6n\x1b[?2026$p");
        assert!(r.cpr_requested);
        // Response should contain DA1 response + DECRQM response (CPR is deferred)
        let expected_da1 = b"\x1b[?62;22c";
        let expected_decrqm = b"\x1b[?2026;1$y";
        assert_eq!(resp.len(), expected_da1.len() + expected_decrqm.len());
        assert_eq!(&resp[..expected_da1.len()], expected_da1.as_slice());
        assert_eq!(&resp[expected_da1.len()..], expected_decrqm.as_slice());
    }

    #[test]
    fn cpr_between_normal_csi_sequences() {
        // SGR + CPR + cursor-move — CPR detected, non-queries skipped
        let mut r = responder();
        let resp = r.scan_for_queries(b"\x1b[31m\x1b[6n\x1b[H");
        assert!(r.cpr_requested);
        assert!(resp.is_empty(), "no DA1/DECRQM queries present");
    }

    // ── Incomplete CSI handling ────────────────────────────────────────

    #[test]
    fn incomplete_csi_saved_as_partial() {
        let mut r = responder();
        let _ = r.scan_for_queries(b"text\x1b[31");
        assert!(!r.partial.is_empty(), "incomplete CSI must be buffered");
    }

    #[test]
    fn complete_csi_not_saved() {
        let mut r = responder();
        let _ = r.scan_for_queries(b"text\x1b[31m");
        assert!(r.partial.is_empty(), "complete CSI must not leave partial");
    }

    #[test]
    fn incomplete_csi_completes_next_read() {
        // \x1b[31 | m — partial restored, sequence completed
        let mut r = responder();
        let _ = r.scan_for_queries(b"\x1b[31");
        assert!(!r.partial.is_empty());
        let _ = r.scan_for_queries(b"m text");
        assert!(r.partial.is_empty());
    }

    #[test]
    fn incomplete_csi_turns_into_cpr() {
        // \x1b[ at end, next read is 6n — reassembles as CPR
        let mut r = responder();
        let _ = r.scan_for_queries(b"content\x1b[");
        assert!(!r.partial.is_empty());
        let _ = r.scan_for_queries(b"6n");
        assert!(r.cpr_requested);
    }

    // ── Non-CSI escapes ────────────────────────────────────────────────

    #[test]
    fn non_csi_escape_not_saved_as_partial() {
        // \x1b] (OSC start) — not CSI, should not trigger partial save
        let mut r = responder();
        let _ = r.scan_for_queries(b"text\x1b]0;title\x07");
        assert!(r.partial.is_empty());
    }

    #[test]
    fn lone_esc_before_non_csi() {
        // \x1b at end, followed by ] — should save ESC then discard
        let mut r = responder();
        let _ = r.scan_for_queries(b"text\x1b");
        assert!(!r.partial.is_empty());
        let _ = r.scan_for_queries(b"]0;title\x07");
        assert!(r.partial.is_empty(), "non-CSI should clear partial");
    }

    // ── Edge cases ─────────────────────────────────────────────────────

    #[test]
    fn empty_data() {
        let mut r = responder();
        let resp = r.scan_for_queries(b"");
        assert!(resp.is_empty());
        assert!(!r.cpr_requested);
        assert!(r.partial.is_empty());
    }

    #[test]
    fn no_escape_sequences() {
        let mut r = responder();
        let resp = r.scan_for_queries(b"Hello world, no escapes here!");
        assert!(resp.is_empty());
        assert!(!r.cpr_requested);
        assert!(r.partial.is_empty());
    }

    #[test]
    fn cpr_resets_each_call() {
        let mut r = responder();
        let _ = r.scan_for_queries(b"\x1b[6n");
        assert!(r.cpr_requested);
        // Next call without CPR should reset the flag
        let _ = r.scan_for_queries(b"no cpr here");
        assert!(!r.cpr_requested);
    }

    #[test]
    fn partial_cleared_on_clean_data() {
        let mut r = responder();
        let _ = r.scan_for_queries(b"text\x1b[");
        assert!(!r.partial.is_empty());
        // Next call: partial + "H" → complete CSI (cursor home), partial cleared
        let _ = r.scan_for_queries(b"H more");
        assert!(r.partial.is_empty());
    }

    #[test]
    fn bsu_esc_sequence_not_false_cpr() {
        // BSU (\x1b[?2026h) contains no CPR — should not set cpr_requested
        let mut r = responder();
        let _ = r.scan_for_queries(b"\x1b[?2026h content \x1b[?2026l");
        assert!(!r.cpr_requested);
    }
}

#[cfg(test)]
mod sync_buf_tests {
    use super::*;

    fn buf() -> SyncOutputBuffer {
        SyncOutputBuffer::new()
    }

    fn concat(chunks: &[Vec<u8>]) -> Vec<u8> {
        chunks.iter().flatten().copied().collect()
    }

    const BSU: &[u8] = b"\x1b[?2026h";
    const ESU: &[u8] = b"\x1b[?2026l";

    // ── Basic framing ──────────────────────────────────────────────────

    #[test]
    fn passthrough_without_markers() {
        let mut b = buf();
        let out = b.process(b"Hello world");
        assert_eq!(concat(&out), b"Hello world");
    }

    #[test]
    fn single_frame() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(b"content");
        data.extend(ESU);
        let out = b.process(&data);
        assert_eq!(concat(&out), b"content");
    }

    #[test]
    fn content_before_frame() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(b"before ");
        data.extend(BSU);
        data.extend(b"inside");
        data.extend(ESU);
        let out = b.process(&data);
        // Two chunks: passthrough "before " + synced "inside"
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], b"before ");
        assert_eq!(out[1], b"inside");
    }

    #[test]
    fn content_after_frame() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(b"inside");
        data.extend(ESU);
        data.extend(b" after");
        let out = b.process(&data);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], b"inside");
        assert_eq!(out[1], b" after");
    }

    #[test]
    fn multiple_frames() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(b"frame1");
        data.extend(ESU);
        data.extend(BSU);
        data.extend(b"frame2");
        data.extend(ESU);
        let out = b.process(&data);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], b"frame1");
        assert_eq!(out[1], b"frame2");
    }

    #[test]
    fn empty_frame() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(ESU);
        let out = b.process(&data);
        // Empty frame produces no output
        assert!(out.is_empty() || concat(&out).is_empty());
    }

    // ── Split markers across reads ─────────────────────────────────────

    #[test]
    fn bsu_split_at_every_byte() {
        // Split BSU (\x1b[?2026h) at each of the 7 internal boundaries
        let bsu = b"\x1b[?2026h";
        for split in 1..8 {
            let mut b = buf();
            let mut read1: Vec<u8> = Vec::new();
            read1.extend(b"before");
            read1.extend(&bsu[..split]);

            let mut read2: Vec<u8> = Vec::new();
            read2.extend(&bsu[split..]);
            read2.extend(b"inside");

            let out1 = b.process(&read1);
            // "before" should be emitted (or trail saved)
            let out2 = b.process(&read2);

            // After both reads, we should be in syncing mode (BSU detected)
            // and "inside" should be buffered (no ESU yet)
            let total = concat(&out1).len() + concat(&out2).len();
            // "before" is 6 bytes; "inside" should be buffered
            assert!(
                total <= 6,
                "split={split}: 'inside' should be buffered, got total={total}"
            );
            assert!(
                b.syncing,
                "split={split}: should be in syncing mode after BSU"
            );
        }
    }

    #[test]
    fn esu_split_at_every_byte() {
        // Start with BSU, buffer content, then split ESU
        let esu = b"\x1b[?2026l";
        for split in 1..8 {
            let mut b = buf();
            // First: BSU + content
            let mut read1: Vec<u8> = Vec::new();
            read1.extend(BSU);
            read1.extend(b"content");
            read1.extend(&esu[..split]);

            let out1 = b.process(&read1);

            // Content should still be buffered (ESU incomplete)
            assert!(
                concat(&out1).is_empty(),
                "split={split}: content should be buffered until ESU completes"
            );

            // Second: rest of ESU
            let out2 = b.process(&esu[split..]);
            assert_eq!(
                concat(&out2),
                b"content",
                "split={split}: content should be emitted after ESU completes"
            );
        }
    }

    // ── Trail false positives ──────────────────────────────────────────

    #[test]
    fn trail_esc_followed_by_non_marker() {
        // \x1b at end, next read is [31m (SGR, not BSU/ESU)
        let mut b = buf();
        let out1 = b.process(b"text\x1b");
        // "text" emitted, \x1b saved as trail
        let out2 = b.process(b"[31m more");
        // Trail combined: \x1b[31m → not BSU/ESU → passthrough
        let total: Vec<u8> = [concat(&out1), concat(&out2)].concat();
        assert_eq!(total, b"text\x1b[31m more");
    }

    #[test]
    fn trail_esc_bracket_followed_by_non_marker() {
        // \x1b[ at end, next read starts with 2004h (bracketed paste, not 2026)
        let mut b = buf();
        let out1 = b.process(b"text\x1b[");
        let out2 = b.process(b"?2004h more");
        let total: Vec<u8> = [concat(&out1), concat(&out2)].concat();
        assert_eq!(total, b"text\x1b[?2004h more");
    }

    #[test]
    fn trail_esc_bracket_question_not_2026() {
        // \x1b[? at end, next read is 1049h (alternate screen)
        let mut b = buf();
        let out1 = b.process(b"text\x1b[?");
        let out2 = b.process(b"1049h");
        let total: Vec<u8> = [concat(&out1), concat(&out2)].concat();
        assert_eq!(total, b"text\x1b[?1049h");
    }

    // ── Orphan ESU ─────────────────────────────────────────────────────

    #[test]
    fn orphan_esu_stripped() {
        // ESU without preceding BSU — ESU bytes stripped, content passes through
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(b"before");
        data.extend(ESU);
        data.extend(b"after");
        let out = b.process(&data);
        assert_eq!(concat(&out), b"beforeafter");
    }

    // ── Nested BSU ─────────────────────────────────────────────────────

    #[test]
    fn nested_bsu_content_not_lost() {
        // BSU content1 BSU content2 ESU — content1 must not be dropped
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(b"content1");
        data.extend(BSU);
        data.extend(b"content2");
        data.extend(ESU);
        let out = b.process(&data);
        assert_eq!(concat(&out), b"content1content2");
    }

    // ── BSU without ESU ────────────────────────────────────────────────

    #[test]
    fn bsu_without_esu_buffers_indefinitely() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(b"still waiting");
        let out1 = b.process(&data);
        assert!(concat(&out1).is_empty(), "content buffered until ESU");
        assert!(b.syncing);

        // More data without ESU
        let out2 = b.process(b" more data");
        assert!(concat(&out2).is_empty());
        assert!(b.syncing);

        // Finally ESU
        let out3 = b.process(ESU);
        assert_eq!(concat(&out3), b"still waiting more data");
        assert!(!b.syncing);
    }

    // ── Data integrity ─────────────────────────────────────────────────

    #[test]
    fn markers_stripped_content_preserved() {
        // All BSU/ESU bytes must be stripped; all other bytes preserved
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(b"A");
        data.extend(BSU);
        data.extend(b"B");
        data.extend(ESU);
        data.extend(b"C");
        data.extend(BSU);
        data.extend(b"D");
        data.extend(ESU);
        data.extend(b"E");
        let out = b.process(&data);
        assert_eq!(concat(&out), b"ABCDE");
    }

    #[test]
    fn empty_data() {
        let mut b = buf();
        let out = b.process(b"");
        assert!(out.is_empty());
    }

    #[test]
    fn frame_split_across_three_reads() {
        // BSU in read 1, content in read 2, ESU in read 3
        let mut b = buf();
        let out1 = b.process(BSU);
        assert!(concat(&out1).is_empty());
        let out2 = b.process(b"the content");
        assert!(concat(&out2).is_empty());
        let out3 = b.process(ESU);
        assert_eq!(concat(&out3), b"the content");
    }

    #[test]
    fn passthrough_between_frames() {
        let mut b = buf();
        let mut data = Vec::new();
        data.extend(BSU);
        data.extend(b"F1");
        data.extend(ESU);
        data.extend(b"GAP");
        data.extend(BSU);
        data.extend(b"F2");
        data.extend(ESU);
        let out = b.process(&data);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], b"F1");
        assert_eq!(out[1], b"GAP");
        assert_eq!(out[2], b"F2");
    }
}

#[cfg(test)]
mod find_cpr_tests {
    use super::*;

    #[test]
    fn finds_cpr_at_start() {
        assert_eq!(find_cpr_offset(b"\x1b[6n rest"), Some(0));
    }

    #[test]
    fn finds_cpr_in_middle() {
        assert_eq!(find_cpr_offset(b"before\x1b[6n after"), Some(6));
    }

    #[test]
    fn no_cpr_in_data() {
        assert_eq!(find_cpr_offset(b"no cpr here"), None);
    }

    #[test]
    fn data_too_short() {
        assert_eq!(find_cpr_offset(b"\x1b[6"), None);
        assert_eq!(find_cpr_offset(b"\x1b["), None);
        assert_eq!(find_cpr_offset(b"\x1b"), None);
        assert_eq!(find_cpr_offset(b""), None);
    }

    #[test]
    fn finds_first_occurrence() {
        assert_eq!(find_cpr_offset(b"\x1b[6n\x1b[6n"), Some(0));
    }
}

/// Find the byte offset of `\x1b[6n` (CPR query) in data.
fn find_cpr_offset(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    data.windows(4).position(|w| w == b"\x1b[6n")
}

const RING_BUFFER_CAP: usize = 4 * 1024 * 1024; // 4MB for scrollback history

struct AgentSlot {
    #[allow(dead_code)]
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child_pid: Option<u32>,
    /// Raw output history — provides scrollback when client attaches.
    ring_buffer: VecDeque<u8>,
    /// Daemon-side vt100 parser — always holds the correct screen state.
    /// Appended after ring buffer in catch-up to fix the visible area.
    parser: vt100::Parser,
    alive: bool,
    /// Per-agent sync output buffer (moved from reader thread for CPR accuracy).
    sync_buf: SyncOutputBuffer,
    /// Per-agent terminal query responder.
    responder: TerminalQueryResponder,
}

impl AgentSlot {
    /// Push sync-buffered output to the ring buffer (no parser update).
    fn push_ring_buffer(&mut self, data: &[u8]) {
        let overflow = (self.ring_buffer.len() + data.len()).saturating_sub(RING_BUFFER_CAP);
        if overflow > 0 {
            self.ring_buffer.drain(..overflow);
            skip_partial_escape(&mut self.ring_buffer);
        }
        self.ring_buffer.extend(data);
    }

    /// Rebuild the parser from scratch by replaying the ring buffer.
    /// Fixes accumulated rendering issues (parser state corruption, etc.).
    fn rebuild_parser(&mut self) {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let mut new_parser = vt100::Parser::new(rows, cols, 0);
        let buf: Vec<u8> = self.ring_buffer.iter().copied().collect();
        new_parser.process(&buf);
        self.parser = new_parser;
    }

    /// Ring buffer (scrollback) + contents_formatted (clean visible screen).
    /// Client processes both: ring buffer creates scrollback, then
    /// contents_formatted clears and repaints the visible area cleanly.
    fn catch_up_data(&self) -> Vec<u8> {
        let mut data: Vec<u8> = Vec::with_capacity(self.ring_buffer.len() + 256);
        data.extend(self.ring_buffer.iter());
        // CAN (0x18) aborts any in-progress escape sequence left at the end
        // of the ring buffer (from PTY read splitting mid-sequence).
        // SGR reset + cursor home + screen clear ensure contents_formatted()
        // starts from a known-good state and fully repaints the visible area.
        data.extend_from_slice(b"\x18\x1b[m\x1b[H\x1b[2J");
        data.extend(self.parser.screen().contents_formatted());
        data
    }

    /// Process raw PTY data: detect queries, update parser (with CPR-aware
    /// splitting), sync-buffer for ring buffer + client output.
    ///
    /// Returns sync-buffered output chunks to forward to the client.
    fn process_raw_data(&mut self, raw: &[u8]) -> Vec<Vec<u8>> {
        // 1. Detect terminal queries (DA1, DECRQM, CPR)
        let responses = self.responder.scan_for_queries(raw);
        if !responses.is_empty() {
            let _ = self.writer.write_all(&responses);
            let _ = self.writer.flush();
        }

        // 2. Update parser — split at CPR offset for accurate cursor position
        if self.responder.cpr_requested {
            if let Some(cpr_off) = find_cpr_offset(raw) {
                // Feed data up to the CPR query into the parser
                self.parser.process(&raw[..cpr_off]);
                // Respond with cursor position at the CPR query point
                let (row, col) = self.parser.screen().cursor_position();
                let response = format!("\x1b[{};{}R", row + 1, col + 1);
                let _ = self.writer.write_all(response.as_bytes());
                let _ = self.writer.flush();
                // Feed remaining data (CPR bytes are harmless — DSR is ignored)
                self.parser.process(&raw[cpr_off..]);
            } else {
                // CPR sequence spans reads — respond with current parser state
                // (parser was already updated by previous RawData events)
                let (row, col) = self.parser.screen().cursor_position();
                let response = format!("\x1b[{};{}R", row + 1, col + 1);
                let _ = self.writer.write_all(response.as_bytes());
                let _ = self.writer.flush();
                // Then process this read's data
                self.parser.process(raw);
            }
        } else {
            self.parser.process(raw);
        }

        // 3. Sync-buffer the raw data for ring buffer + client (strips BSU/ESU)
        let chunks = self.sync_buf.process(raw);
        for chunk in &chunks {
            self.push_ring_buffer(chunk);
        }
        chunks
    }
}

/// After byte-level drain, skip past any partial escape sequence at the front.
///
/// Scans forward to find the first "safe" byte to start parsing from:
/// a newline, an ESC (start of a new sequence), or a byte after a CSI
/// final byte (0x40-0x7E) that terminates the partial sequence.
fn skip_partial_escape(buf: &mut VecDeque<u8>) {
    if buf.is_empty() {
        return;
    }
    // If the front byte is ESC, we're at a sequence boundary — nothing to skip.
    if buf.front() == Some(&0x1b) {
        return;
    }
    // If the front byte is a normal printable char or control that isn't
    // part of a CSI parameter/intermediate range, it's probably safe.
    // CSI parameters are 0x30-0x3F, intermediates are 0x20-0x2F.
    // If we see something outside those ranges (and not ESC), we're likely
    // at normal text already.
    if let Some(&front) = buf.front() {
        if front == 0x0a || front == 0x0d {
            return; // newline boundary
        }
        // If it doesn't look like mid-CSI, leave it alone
        if front >= 0x40 && front != 0x5b {
            // 0x40-0x7E are CSI final bytes or uppercase letters.
            // If we land on one, it terminates whatever partial sequence
            // preceded it — skip it and we're clean.
            buf.pop_front();
            return;
        }
    }
    // Likely mid-CSI (parameters/intermediates). Scan forward to the end
    // of the partial sequence or the next safe boundary.
    let is_csi_final = |b: u8| (0x40..=0x7e).contains(&b) && b != 0x5b;
    let skip_to = buf
        .iter()
        .position(|&b| b == 0x1b || b == 0x0a || b == 0x0d || is_csi_final(b));
    if let Some(pos) = skip_to {
        let skip = if buf.get(pos).is_some_and(|&b| is_csi_final(b)) {
            pos + 1 // skip past the final byte too
        } else {
            pos // stop before ESC/newline
        };
        buf.drain(..skip);
    }
}

fn send_sigint(child_pid: u32) {
    if let Ok(output) = std::process::Command::new("pgrep")
        .args(["-P", &child_pid.to_string()])
        .output()
    {
        let children_str = String::from_utf8_lossy(&output.stdout);
        for line in children_str.lines() {
            if let Ok(cpid) = line.trim().parse::<i32>() {
                let pgid = unsafe { libc::getpgid(cpid) };
                if pgid > 0 {
                    unsafe { libc::kill(-pgid, libc::SIGINT) };
                    return;
                }
            }
        }
    }
    unsafe { libc::kill(-(child_pid as i32), libc::SIGINT) };
}

async fn send_to_client(stream: &mut tokio::net::UnixStream, msg: &DaemonMessage) -> bool {
    tokio::time::timeout(Duration::from_secs(5), send_message_async(stream, msg))
        .await
        .is_ok_and(|r| r.is_ok())
}

pub async fn run_daemon() -> Result<()> {
    let sock_path = daemon_socket_path()?;
    let pid_path = daemon_pid_path()?;

    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent).context("creating ~/.clamor directory")?;
    }

    if sock_path.exists() {
        if is_daemon_running() {
            daemon_log("startup aborted: daemon already running");
            bail!("daemon already running (socket exists and PID is alive)");
        }
        let _ = std::fs::remove_file(&sock_path);
    }

    std::fs::write(&pid_path, std::process::id().to_string()).context("writing PID file")?;

    let listener = UnixListener::bind(&sock_path).context("binding Unix domain socket")?;

    daemon_log(&format!(
        "daemon started version={} sock={}",
        env!("CARGO_PKG_VERSION"),
        sock_path.display()
    ));

    install_daemon_panic_hook();

    let (pty_tx, mut pty_rx) = mpsc::channel::<PtyEvent>(1024);

    let mut agents: HashMap<String, AgentSlot> = HashMap::new();
    let mut client: Option<tokio::net::UnixStream> = None;
    let mut subscriptions: HashSet<String> = HashSet::new();
    let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        // Build a future that reads one client message, or pends forever if no client
        let client_read = async {
            match client {
                Some(ref mut stream) => recv_message_async::<ClientMessage, _>(stream).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        if client.is_some() {
                            daemon_log(
                                "client connected; previous client dropped (only one client supported)"
                            );
                        } else {
                            daemon_log("client connected");
                        }
                        subscriptions.clear();
                        client = Some(stream);
                    }
                    Err(e) => {
                        daemon_log(&format!("accept error: {e}"));
                        eprintln!("clamor-daemon: accept error: {e}");
                    }
                }
            }

            Some(evt) = pty_rx.recv() => {
                match evt {
                    PtyEvent::RawData { id, data } => {
                        // All output processing happens here: query detection,
                        // parser update (split at CPR offset), sync buffering,
                        // ring buffer, and client forwarding.
                        let chunks = if let Some(slot) = agents.get_mut(&id) {
                            slot.process_raw_data(&data)
                        } else {
                            Vec::new()
                        };
                        if subscriptions.contains(&id) {
                            let mut disconnect = false;
                            for chunk in chunks {
                                if let Some(ref mut stream) = client {
                                    let msg = DaemonMessage::Output {
                                        id: id.clone(),
                                        data: chunk,
                                    };
                                    if !send_to_client(stream, &msg).await {
                                        disconnect = true;
                                        break;
                                    }
                                }
                            }
                            if disconnect {
                                client = None;
                                subscriptions.clear();
                            }
                        }
                    }
                    PtyEvent::Exited { id } => {
                        if let Some(slot) = agents.get_mut(&id) {
                            slot.alive = false;
                        }
                        let mut disconnect = false;
                        if let Some(ref mut stream) = client {
                            let msg = DaemonMessage::Exited { id };
                            if !send_to_client(stream, &msg).await {
                                disconnect = true;
                            }
                        }
                        if disconnect {
                            client = None;
                            subscriptions.clear();
                        }
                    }
                }
            }

            result = client_read => {
                match result {
                    Ok(msg) => {
                        let stream = client.as_mut().unwrap();
                        match handle_client_message(
                            msg, &mut agents, &mut subscriptions, stream, &pty_tx,
                        ).await {
                            HandleResult::Continue => {}
                            HandleResult::Shutdown => {
                                daemon_log("shutdown requested by client");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        daemon_log(&format!("client read ended: {e:#}"));
                        client = None;
                        subscriptions.clear();
                    }
                }
            }

            _ = heartbeat_interval.tick() => {
                let mut disconnect = false;
                if let Some(ref mut stream) = client {
                    if !send_to_client(stream, &DaemonMessage::Heartbeat).await {
                        disconnect = true;
                    }
                }
                if disconnect {
                    client = None;
                    subscriptions.clear();
                }
            }
        }
    }

    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_path);

    daemon_log("daemon stopped");

    Ok(())
}

fn install_daemon_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        daemon_log(&format!("PANIC: {info}"));
        original(info);
    }));
}

enum HandleResult {
    Continue,
    Shutdown,
}

async fn handle_client_message(
    msg: ClientMessage,
    agents: &mut HashMap<String, AgentSlot>,
    subscriptions: &mut HashSet<String>,
    stream: &mut tokio::net::UnixStream,
    pty_tx: &mpsc::Sender<PtyEvent>,
) -> HandleResult {
    match msg {
        ClientMessage::Spawn {
            id,
            cwd,
            cmd,
            env,
            rows,
            cols,
        } => {
            match spawn_agent_pty(&id, &cwd, &cmd, &env, rows, cols, pty_tx) {
                Ok(slot) => {
                    agents.insert(id, slot);
                    let _ = send_to_client(stream, &DaemonMessage::Ok).await;
                }
                Err(e) => {
                    let _ = send_to_client(
                        stream,
                        &DaemonMessage::Error {
                            message: format!("{e:#}"),
                        },
                    )
                    .await;
                }
            }
            HandleResult::Continue
        }
        ClientMessage::Kill { id } => {
            if let Some(slot) = agents.get_mut(&id) {
                if let Some(pid) = slot.child_pid {
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
                slot.alive = false;
                let _ = send_to_client(stream, &DaemonMessage::Ok).await;
            } else {
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::Error {
                        message: format!("unknown agent: {id}"),
                    },
                )
                .await;
            }
            HandleResult::Continue
        }
        ClientMessage::Sigint { id } => {
            if let Some(slot) = agents.get(&id) {
                if let Some(pid) = slot.child_pid {
                    send_sigint(pid);
                }
                let _ = send_to_client(stream, &DaemonMessage::Ok).await;
            } else {
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::Error {
                        message: format!("unknown agent: {id}"),
                    },
                )
                .await;
            }
            HandleResult::Continue
        }
        ClientMessage::Input { id, data } => {
            if let Some(slot) = agents.get_mut(&id) {
                let _ = slot.writer.write_all(&data);
                let _ = slot.writer.flush();
            }
            HandleResult::Continue
        }
        ClientMessage::Resize { id, rows, cols } => {
            if let Some(slot) = agents.get_mut(&id) {
                let size = PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                };
                let _ = slot.master.resize(size);
                slot.parser.screen_mut().set_size(rows, cols);
                let _ = send_to_client(stream, &DaemonMessage::Ok).await;
            } else {
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::Error {
                        message: format!("unknown agent: {id}"),
                    },
                )
                .await;
            }
            HandleResult::Continue
        }
        ClientMessage::Subscribe { id } => {
            if let Some(slot) = agents.get(&id) {
                let catch_up_data = slot.catch_up_data();
                subscriptions.insert(id.clone());
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::CatchUp {
                        id,
                        data: catch_up_data,
                    },
                )
                .await;
            } else {
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::Error {
                        message: format!("unknown agent: {id}"),
                    },
                )
                .await;
            }
            HandleResult::Continue
        }
        ClientMessage::RefreshParser { id } => {
            if let Some(slot) = agents.get_mut(&id) {
                slot.rebuild_parser();
                let catch_up_data = slot.catch_up_data();
                subscriptions.insert(id.clone());
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::CatchUp {
                        id,
                        data: catch_up_data,
                    },
                )
                .await;
            } else {
                let _ = send_to_client(
                    stream,
                    &DaemonMessage::Error {
                        message: format!("unknown agent: {id}"),
                    },
                )
                .await;
            }
            HandleResult::Continue
        }
        ClientMessage::Unsubscribe { id } => {
            subscriptions.remove(&id);
            let _ = send_to_client(stream, &DaemonMessage::Ok).await;
            HandleResult::Continue
        }
        ClientMessage::List => {
            let list: Vec<DaemonAgent> = agents
                .iter()
                .map(|(id, slot)| DaemonAgent {
                    id: id.clone(),
                    alive: slot.alive,
                })
                .collect();
            let _ = send_to_client(stream, &DaemonMessage::AgentList { agents: list }).await;
            HandleResult::Continue
        }
        ClientMessage::Shutdown => {
            let _ = send_to_client(stream, &DaemonMessage::Ok).await;
            HandleResult::Shutdown
        }
        ClientMessage::Hello { version: _ } => {
            let _ = send_to_client(
                stream,
                &DaemonMessage::Hello {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            )
            .await;
            HandleResult::Continue
        }
        ClientMessage::Pong => HandleResult::Continue,
    }
}

fn spawn_agent_pty(
    id: &str,
    cwd: &str,
    cmd: &[String],
    env: &[(String, String)],
    rows: u16,
    cols: u16,
    pty_tx: &mpsc::Sender<PtyEvent>,
) -> Result<AgentSlot> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut cmd_builder = if cmd.is_empty() {
        CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()))
    } else {
        let mut cb = CommandBuilder::new(&cmd[0]);
        for arg in &cmd[1..] {
            cb.arg(arg);
        }
        cb
    };
    cmd_builder.cwd(cwd);
    for (key, val) in env {
        cmd_builder.env(key, val);
    }

    let child = pair
        .slave
        .spawn_command(cmd_builder)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let child_pid = child.process_id();
    drop(pair.slave);

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let tx = pty_tx.clone();
    let agent_id = id.to_string();
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Reader thread is now minimal — just reads and forwards raw bytes.
    // All processing (query detection, sync buffering, CPR handling)
    // happens daemon-side in AgentSlot::process_raw_data().
    tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.blocking_send(PtyEvent::Exited {
                        id: agent_id.clone(),
                    });
                    break;
                }
                Ok(n) => {
                    if tx
                        .blocking_send(PtyEvent::RawData {
                            id: agent_id.clone(),
                            data: buf[..n].to_vec(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });

    let _child = child;
    std::mem::forget(_child);

    Ok(AgentSlot {
        master: pair.master,
        writer,
        child_pid,
        ring_buffer: VecDeque::with_capacity(RING_BUFFER_CAP),
        parser: vt100::Parser::new(rows, cols, 0),
        alive: true,
        sync_buf: SyncOutputBuffer::new(),
        responder: TerminalQueryResponder::new(),
    })
}
