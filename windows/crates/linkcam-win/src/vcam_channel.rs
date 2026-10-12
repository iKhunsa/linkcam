//! File-backed frame channel for the virtual camera (see `linkcam_core::vcam`).
//!
//! Lives in a world-readable folder because the media source is loaded by
//! Windows' Frame Server, which runs under a different account.

use linkcam_core::vcam::{Header, FRAME_BYTES, HEADER_BYTES};
use linkcam_core::{CameraError, Result};
use std::fs::{File, OpenOptions};
use std::os::windows::fs::FileExt;
use std::path::PathBuf;
use std::time::Instant;

/// `C:\Users\Public\LinkCam\vcam.frame`
pub fn channel_path() -> PathBuf {
    let public =
        std::env::var_os("PUBLIC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
    public.join("LinkCam").join("vcam.frame")
}

fn io_error(error: std::io::Error) -> CameraError {
    CameraError::WriteFailed(error.to_string())
}

/// Producer side: the app writes every processed frame here.
pub struct FrameWriter {
    file: File,
    sequence: u32,
    started: Instant,
}

impl FrameWriter {
    pub fn create() -> Result<Self> {
        let path = channel_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_error)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(io_error)?;
        file.set_len((HEADER_BYTES + FRAME_BYTES) as u64).map_err(io_error)?;
        let mut writer = Self { file, sequence: 0, started: Instant::now() };
        writer.write_header(0)?;
        Ok(writer)
    }

    fn write_header(&mut self, sequence: u32) -> Result<()> {
        let timestamp_100ns = (self.started.elapsed().as_nanos() / 100) as u64;
        let header = Header { sequence, timestamp_100ns }.encode();
        self.file.seek_write(&header, 0).map(|_| ()).map_err(io_error)
    }

    /// Publishes one BGRA frame of exactly `FRAME_BYTES`.
    pub fn publish(&mut self, bgra: &[u8]) -> Result<()> {
        if bgra.len() != FRAME_BYTES {
            return Err(CameraError::InvalidArgument("frame has the wrong size for the channel".into()));
        }
        self.sequence = self.sequence.wrapping_add(1) | 1; // odd: being written
        self.write_header(self.sequence)?;
        self.file.seek_write(bgra, HEADER_BYTES as u64).map_err(io_error)?;
        self.sequence = self.sequence.wrapping_add(1); // even: stable
        self.write_header(self.sequence)
    }
}

/// Consumer side: the media source reads the latest stable frame.
pub struct FrameReader {
    file: File,
    last_sequence: u32,
}

impl FrameReader {
    pub fn open() -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .open(channel_path())
            .map_err(|e| CameraError::ReadFailed(e.to_string()))?;
        Ok(Self { file, last_sequence: 0 })
    }

    /// Copies the latest frame into `out` (resized to `FRAME_BYTES`). Returns
    /// whether it is newer than the previous call.
    pub fn latest(&mut self, out: &mut Vec<u8>) -> bool {
        const ATTEMPTS: usize = 4;
        out.resize(FRAME_BYTES, 0);
        for _ in 0..ATTEMPTS {
            let mut head = [0u8; HEADER_BYTES];
            if self.file.seek_read(&mut head, 0).is_err() {
                return false;
            }
            let Some(before) = Header::decode(&head).filter(|h| h.is_stable()) else { continue };
            if self.file.seek_read(out, HEADER_BYTES as u64).is_err() {
                return false;
            }
            let mut tail = [0u8; HEADER_BYTES];
            if self.file.seek_read(&mut tail, 0).is_err() {
                return false;
            }
            if Header::decode(&tail).map(|h| h.sequence) == Some(before.sequence) {
                let fresh = before.sequence != self.last_sequence;
                self.last_sequence = before.sequence;
                return fresh;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_writer_publishes_the_reader_sees() {
        let mut writer = FrameWriter::create().unwrap();
        let frame: Vec<u8> = (0..FRAME_BYTES).map(|i| (i % 251) as u8).collect();
        writer.publish(&frame).unwrap();

        let mut reader = FrameReader::open().unwrap();
        let mut got = Vec::new();
        assert!(reader.latest(&mut got), "first read is fresh");
        assert_eq!(got, frame);
        assert!(!reader.latest(&mut got), "second read has nothing new");

        writer.publish(&frame).unwrap();
        assert!(reader.latest(&mut got));
    }
}
