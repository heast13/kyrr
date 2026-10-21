//! Voice memos as Ogg Opus files (RFC 7845): one mono stream of about
//! 24 kbit/s (some 11 MB an hour), written page by page while it is
//! recorded. A page ends every second and goes to the disk every few, so a
//! file is whole up to then even when kyrr or the computer stops halfway;
//! players take such a file as it is.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};

/// Opus counts time in 48 kHz samples, whatever rate went in.
pub const GRANULE_RATE: u32 = 48_000;
/// The rates the encoder takes; others are converted to 48 kHz first.
pub const RATES: [u32; 5] = [8_000, 12_000, 16_000, 24_000, 48_000];
/// Clear speech, a fraction of the size of MP3.
const BITRATE: i32 = 24_000;
/// 20 ms per packet.
const PACKETS_PER_SECOND: u32 = 50;
/// A page ends after a second of packets: at most that is lost.
const PACKETS_PER_PAGE: u32 = 50;
/// The written pages go to the disk itself every this many (power loss).
const PAGES_PER_SYNC: u32 = 5;
/// Longest packet Opus makes, 120 ms, in 48 kHz samples.
const MAX_PACKET: usize = 5760;

/// Writes a recording into an Ogg Opus file as it comes in.
pub struct Writer {
    out: PacketWriter<'static, BufWriter<File>>,
    encoder: opus::Encoder,
    serial: u32,
    rate: u32,
    frame: usize,
    pending: Vec<f32>,
    /// The encoder's delay, in 48 kHz samples, which players drop.
    pre_skip: u64,
    lookahead: usize,
    /// Samples encoded, and samples of the recording itself, at `rate`.
    encoded: u64,
    recorded: u64,
    packets: u32,
    pages: u32,
    packet: Vec<u8>,
}

impl Writer {
    /// Starts the file at `path` (it must not exist yet) for mono audio at
    /// `rate`, one of [`RATES`].
    pub fn create(path: &Path, rate: u32) -> std::io::Result<Writer> {
        let err = |e: opus::Error| std::io::Error::other(e.to_string());
        let mut encoder = opus::Encoder::new(rate, opus::Channels::Mono, opus::Application::Voip).map_err(err)?;
        encoder.set_bitrate(opus::Bitrate::Bits(BITRATE)).map_err(err)?;
        encoder.set_signal(opus::Signal::Voice).map_err(err)?;
        let lookahead = encoder.get_lookahead().map_err(err)?.max(0) as usize;
        let pre_skip = lookahead as u64 * GRANULE_RATE as u64 / rate as u64;
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        let serial = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
        let mut writer = Writer {
            out: PacketWriter::new(BufWriter::new(file)),
            encoder,
            serial,
            rate,
            frame: (rate / PACKETS_PER_SECOND) as usize,
            pending: Vec::new(),
            pre_skip,
            lookahead,
            encoded: 0,
            recorded: 0,
            packets: 0,
            pages: 0,
            packet: vec![0; 1500],
        };
        // Both headers on pages of their own, as the format asks.
        let mut head = b"OpusHead".to_vec();
        head.push(1);
        head.push(1);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&rate.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        let vendor = b"kyrr";
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        writer.out.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;
        writer.out.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;
        writer.out.inner_mut().flush()?;
        Ok(writer)
    }

    /// Seconds of the recording so far.
    pub fn seconds(&self) -> f64 {
        (self.recorded + self.pending.len() as u64) as f64 / self.rate as f64
    }

    /// Adds mono samples at the file's rate.
    pub fn push(&mut self, samples: &[f32]) -> std::io::Result<()> {
        self.recorded += samples.len() as u64;
        self.pending.extend_from_slice(samples);
        let mut at = 0;
        while self.pending.len() - at >= self.frame {
            self.encode(at, false)?;
            at += self.frame;
        }
        self.pending.drain(..at);
        Ok(())
    }

    /// Ends the stream: the rest, the encoder's delay, and the file on the
    /// disk.
    pub fn finish(mut self) -> std::io::Result<()> {
        // The encoder holds back its delay: silence pushes it out.
        self.pending.resize(self.pending.len() + self.lookahead, 0.0);
        let frames = self.pending.len().div_ceil(self.frame).max(1);
        self.pending.resize(frames * self.frame, 0.0);
        for i in 0..frames {
            self.encode(i * self.frame, i + 1 == frames)?;
        }
        let mut file = self.out.into_inner().into_inner().map_err(|e| e.into_error())?;
        file.flush()?;
        file.sync_all()
    }

    fn encode(&mut self, at: usize, last: bool) -> std::io::Result<()> {
        let input = &self.pending[at..at + self.frame];
        let n = self
            .encoder
            .encode_float(input, &mut self.packet)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.encoded += self.frame as u64;
        self.packets += 1;
        let to_granule = |samples: u64| samples * GRANULE_RATE as u64 / self.rate as u64 + self.pre_skip;
        // The end of the recording, not of the padding after it.
        let granule = to_granule(self.encoded).min(to_granule(self.recorded));
        let end = if last {
            PacketWriteEndInfo::EndStream
        } else if self.packets.is_multiple_of(PACKETS_PER_PAGE) {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        self.out
            .write_packet(self.packet[..n].to_vec(), self.serial, end, granule)?;
        if end == PacketWriteEndInfo::EndPage {
            self.out.inner_mut().flush()?;
            self.pages += 1;
            if self.pages.is_multiple_of(PAGES_PER_SYNC) {
                self.out.inner_mut().get_ref().sync_data()?;
            }
        }
        Ok(())
    }
}

/// Reads an Ogg Opus file back as mono 48 kHz samples, a packet at a time.
pub struct Reader {
    packets: PacketReader<BufReader<File>>,
    decoder: opus::Decoder,
    channels: usize,
    /// Samples still to drop at the start (the encoder's delay).
    skip: usize,
    /// Samples given out so far, and how many the stream has in all once
    /// its last page says so.
    made: u64,
    total: Option<u64>,
    pre_skip: u64,
    buffer: Vec<f32>,
}

impl Reader {
    pub fn open(path: &Path) -> Result<Reader, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut packets = PacketReader::new(BufReader::new(file));
        let head = packets
            .read_packet()
            .map_err(|e| e.to_string())?
            .ok_or("an empty file")?;
        let (channels, pre_skip) = parse_head(&head.data).ok_or("not an Opus file")?;
        // The comments, unused.
        packets.read_packet().map_err(|e| e.to_string())?;
        let layout = if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };
        let decoder = opus::Decoder::new(GRANULE_RATE, layout).map_err(|e| e.to_string())?;
        Ok(Reader {
            packets,
            decoder,
            channels: channels.min(2),
            skip: pre_skip as usize,
            made: 0,
            total: None,
            pre_skip,
            buffer: vec![0.0; MAX_PACKET * 2],
        })
    }

    /// Appends the next packet's samples to `out`; false at the end. A file
    /// cut off in the middle ends where it was cut.
    pub fn next(&mut self, out: &mut Vec<f32>) -> bool {
        loop {
            if self.total.is_some_and(|t| self.made >= t) {
                return false;
            }
            let packet = match self.packets.read_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) | Err(_) => return false,
            };
            if packet.last_in_stream() {
                self.total = Some(packet.absgp_page().saturating_sub(self.pre_skip));
            }
            let Ok(frames) = self.decoder.decode_float(&packet.data, &mut self.buffer, false) else {
                continue;
            };
            let from = self.skip.min(frames);
            self.skip -= from;
            let mut take = frames - from;
            if let Some(total) = self.total {
                take = take.min(total.saturating_sub(self.made) as usize);
            }
            let samples = &self.buffer[from * self.channels..(from + take) * self.channels];
            if self.channels == 1 {
                out.extend_from_slice(samples);
            } else {
                out.extend(samples.as_chunks::<2>().0.iter().map(|[l, r]| (l + r) / 2.0));
            }
            self.made += take as u64;
            if take > 0 {
                return true;
            }
        }
    }
}

/// Channels and pre-skip of an `OpusHead` packet.
fn parse_head(data: &[u8]) -> Option<(usize, u64)> {
    if data.len() < 19 || &data[..8] != b"OpusHead" {
        return None;
    }
    let channels = data[9] as usize;
    let pre_skip = u16::from_le_bytes([data[10], data[11]]) as u64;
    (channels > 0).then_some((channels, pre_skip))
}

/// How long the recording in `path` is, in seconds, from the first and the
/// last page alone: quick, also for a long one.
pub fn seconds(path: &Path) -> Option<f64> {
    let mut file = File::open(path).ok()?;
    let mut first = [0u8; 27 + 255 + 19];
    let read = read_up_to(&mut file, &mut first).ok()?;
    let first = &first[..read];
    if first.len() < 28 || &first[..4] != b"OggS" {
        return None;
    }
    let segments = first[26] as usize;
    let (_, pre_skip) = parse_head(first.get(27 + segments..)?)?;
    let len = file.metadata().ok()?.len();
    // A page is at most about 64 KB: the last one starts in the last 64 KB.
    let tail = len.min(65_536 + 27 + 255);
    file.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut end = vec![0u8; tail as usize];
    read_up_to(&mut file, &mut end).ok()?;
    let granule = end
        .windows(4)
        .enumerate()
        .rev()
        .filter(|(_, w)| *w == b"OggS")
        .filter_map(|(i, _)| end.get(i + 6..i + 14))
        .map(|g| i64::from_le_bytes(g.try_into().unwrap_or_default()))
        .find(|&g| g >= 0)?;
    Some((granule as u64).saturating_sub(pre_skip) as f64 / GRANULE_RATE as f64)
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut at = 0;
    while at < buf.len() {
        match file.read(&mut buf[at..])? {
            0 => break,
            n => at += n,
        }
    }
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kyrr-opus-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tone(rate: u32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|i| 0.3 * (std::f32::consts::TAU * 440.0 * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn a_recording_comes_back_as_long_as_it_was() {
        let dir = folder("roundtrip");
        for rate in [16_000, 48_000] {
            let path = dir.join(format!("{rate}.opus"));
            let mut writer = Writer::create(&path, rate).unwrap();
            // In odd pieces, as the microphone delivers them.
            for piece in tone(rate, 3.3).chunks(777) {
                writer.push(piece).unwrap();
            }
            assert!((writer.seconds() - 3.3).abs() < 0.01);
            writer.finish().unwrap();
            let length = seconds(&path).unwrap();
            assert!((length - 3.3).abs() < 0.001, "{rate}: {length}");
            let mut reader = Reader::open(&path).unwrap();
            let mut out = Vec::new();
            while reader.next(&mut out) {}
            assert_eq!(out.len(), (3.3 * 48_000.0) as usize, "{rate}");
            // The tone, not silence.
            let rms = (out[24_000..48_000].iter().map(|s| s * s).sum::<f32>() / 24_000.0).sqrt();
            assert!(rms > 0.1, "{rate}: {rms}");
            // About 24 kbit/s.
            let size = std::fs::metadata(&path).unwrap().len();
            assert!(size < 3 * 4000 + 2000, "{rate}: {size} bytes");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_cut_off_halfway_still_plays() {
        let dir = folder("cut");
        let path = dir.join("cut.opus");
        let mut writer = Writer::create(&path, 48_000).unwrap();
        writer.push(&tone(48_000, 5.0)).unwrap();
        // kyrr ends here without finishing: what is on the disk is pages.
        drop(writer);
        let length = seconds(&path).unwrap();
        assert!((4.0..=5.0).contains(&length), "{length}");
        let mut reader = Reader::open(&path).unwrap();
        let mut out = Vec::new();
        while reader.next(&mut out) {}
        assert!(out.len() >= 4 * 48_000);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn other_files_are_no_opus() {
        let dir = folder("other");
        let path = dir.join("x.opus");
        std::fs::write(&path, b"RIFF....WAVEfmt ").unwrap();
        assert!(seconds(&path).is_none());
        assert!(Reader::open(&path).is_err());
        assert!(Writer::create(&path, 48_000).is_err(), "never over an existing file");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
