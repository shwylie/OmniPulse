// MPEG transport streams (HLS segments, as SoundCloud sends them): the audio is pulled out of the
// 188-byte packets as its plain stream, AAC in ADTS frames or MP3 frames, which symphonia reads.
//
// One pass per segment: the PAT gives the PMT, the PMT gives the audio stream's PID, and that PID's
// PES packets carry the frames.

const PACKET: usize = 188;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Aac,
    Mp3,
}

impl Codec {
    /// The file extension symphonia knows the plain stream by
    pub fn ext(self) -> &'static str {
        match self {
            Codec::Aac => "aac",
            Codec::Mp3 => "mp3",
        }
    }
}

/// Remembers the audio PID between segments (they repeat the PAT/PMT, but needn't be read again)
#[derive(Default, Debug)]
pub struct Demux {
    pmt_pid: Option<u16>,
    audio_pid: Option<u16>,
    pub codec: Option<Codec>,
}

impl Demux {
    /// A segment's audio, appended to `out`
    pub fn segment(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        let start = data.iter().position(|&b| b == 0x47).ok_or("not a transport stream")?;
        for pkt in data[start..].as_chunks::<PACKET>().0 {
            if pkt[0] != 0x47 {
                continue; // lost sync on this one; the next may be fine
            }
            let pusi = pkt[1] & 0x40 != 0;
            let pid = (((pkt[1] & 0x1f) as u16) << 8) | pkt[2] as u16;
            let afc = (pkt[3] >> 4) & 0x3;
            let mut p = 4usize;
            if afc == 2 || afc == 0 {
                continue; // adaptation field only (or reserved): no payload
            }
            if afc == 3 {
                p += 1 + pkt[4] as usize;
                if p >= PACKET {
                    continue;
                }
            }
            let payload = &pkt[p..];
            if pid == 0 {
                self.read_pat(payload, pusi);
            } else if Some(pid) == self.pmt_pid {
                self.read_pmt(payload, pusi);
            } else if Some(pid) == self.audio_pid {
                if pusi {
                    // a PES header first: 00 00 01, stream id, length, flags, header length
                    if payload.len() < 9 || payload[0..3] != [0, 0, 1] {
                        continue;
                    }
                    let skip = 9 + payload[8] as usize;
                    if skip < payload.len() {
                        out.extend_from_slice(&payload[skip..]);
                    }
                } else {
                    out.extend_from_slice(payload);
                }
            }
        }
        if self.audio_pid.is_none() {
            return Err("there's no audio in this stream".into());
        }
        Ok(())
    }

    fn table(payload: &[u8], pusi: bool) -> Option<&[u8]> {
        let p = if pusi { 1 + *payload.first()? as usize } else { 0 };
        let t = payload.get(p..)?;
        let len = (((*t.get(1)? & 0x0f) as usize) << 8) | *t.get(2)? as usize;
        t.get(..(3 + len).min(t.len()))
    }

    fn read_pat(&mut self, payload: &[u8], pusi: bool) {
        let Some(t) = Self::table(payload, pusi) else { return };
        // programs from byte 8 to the CRC
        let mut i = 8;
        while i + 4 <= t.len().saturating_sub(4) {
            let program = ((t[i] as u16) << 8) | t[i + 1] as u16;
            let pid = (((t[i + 2] & 0x1f) as u16) << 8) | t[i + 3] as u16;
            if program != 0 {
                self.pmt_pid = Some(pid);
                return;
            }
            i += 4;
        }
    }

    fn read_pmt(&mut self, payload: &[u8], pusi: bool) {
        let Some(t) = Self::table(payload, pusi) else { return };
        if t.len() < 12 {
            return;
        }
        let info_len = (((t[10] & 0x0f) as usize) << 8) | t[11] as usize;
        let mut i = 12 + info_len;
        while i + 5 <= t.len().saturating_sub(4) {
            let stream_type = t[i];
            let pid = (((t[i + 1] & 0x1f) as u16) << 8) | t[i + 2] as u16;
            let es_len = (((t[i + 3] & 0x0f) as usize) << 8) | t[i + 4] as usize;
            let codec = match stream_type {
                0x0f | 0x11 => Some(Codec::Aac),
                0x03 | 0x04 => Some(Codec::Mp3),
                _ => None,
            };
            if let Some(c) = codec {
                self.audio_pid = Some(pid);
                self.codec = Some(c);
                return;
            }
            i += 5 + es_len;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // a tiny transport stream around some payload bytes: PAT, PMT (AAC on PID 0x101), then PES packets
    pub(crate) fn wrap(payload: &[u8], stream_type: u8) -> Vec<u8> {
        let mut out = Vec::new();
        let packet = |pid: u16, pusi: bool, cc: u8, body: &[u8]| -> Vec<u8> {
            let mut p = vec![0x47, ((pusi as u8) << 6) | ((pid >> 8) as u8 & 0x1f), pid as u8];
            let room = PACKET - 4;
            if body.len() < room {
                // stuffing in an adaptation field
                let stuff = room - body.len();
                p.push(0x30 | (cc & 0xf));
                p.push((stuff - 1) as u8);
                if stuff > 1 {
                    p.push(0);
                    p.extend(std::iter::repeat_n(0xff, stuff - 2));
                }
            } else {
                p.push(0x10 | (cc & 0xf));
            }
            p.extend_from_slice(body);
            p
        };
        // PAT: program 1 -> PMT 0x100
        let pat = [0, 0x00, 0xb0, 13, 0, 1, 0xc1, 0, 0, 0, 1, 0xe1, 0x00, 0, 0, 0, 0];
        out.extend(packet(0, true, 0, &pat));
        // PMT: stream on 0x101
        let pmt = [0, 0x02, 0xb0, 18, 0, 1, 0xc1, 0, 0, 0xe1, 0x01, 0xf0, 0, stream_type, 0xe1, 0x01, 0xf0, 0, 0, 0, 0, 0];
        out.extend(packet(0x100, true, 0, &pmt));
        // PES: header then the payload, split over packets
        let mut pes = vec![0, 0, 1, 0xc0, 0, 0, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1];
        pes.extend_from_slice(payload);
        let mut cc = 0u8;
        for (k, chunk) in pes.chunks(PACKET - 4).enumerate() {
            out.extend(packet(0x101, k == 0, cc, chunk));
            cc = cc.wrapping_add(1);
        }
        out
    }

    #[test]
    fn pulls_the_audio_out() {
        let audio: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let ts = wrap(&audio, 0x0f);
        let mut d = Demux::default();
        let mut out = Vec::new();
        d.segment(&ts, &mut out).unwrap();
        assert_eq!(d.codec, Some(Codec::Aac));
        assert_eq!(out, audio);
        // the next segment uses what was learned (and MP3 is told apart)
        let mut d = Demux::default();
        let mut out = Vec::new();
        d.segment(&wrap(&audio, 0x03), &mut out).unwrap();
        assert_eq!(d.codec, Some(Codec::Mp3));
        assert!(Demux::default().segment(&[0u8; 10], &mut Vec::new()).is_err());
    }
}
