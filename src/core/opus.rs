//! Bounded inspection of one mono Opus stream; FFmpeg separately decodes every archive.
use crate::core::contracts::*;
use std::{io::Read, path::Path};
fn invalid() -> OmmError {
    OmmError::new(
        ErrorCode::Corrupt,
        false,
        "Ogg/Opus framing or duration is invalid",
    )
}
pub(crate) fn metadata(path: &Path, frames: u64) -> Result<(u16, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut packet = Vec::new();
    let (mut serial, mut sequence, mut packets, mut decoded, mut skip, mut last, mut eos) =
        (None, 0u32, 0usize, 0u64, 0u16, 0u64, false);
    loop {
        let mut h = [0u8; 27];
        let n = file.read(&mut h[..1])?;
        if n == 0 {
            break;
        }
        file.read_exact(&mut h[1..]).map_err(|_| invalid())?;
        if &h[..4] != b"OggS" || h[4] != 0 || eos || h[5] & !7 != 0 {
            return Err(invalid());
        }
        let id = u32::from_le_bytes(h[14..18].try_into().unwrap());
        if serial.is_some_and(|s| s != id)
            || u32::from_le_bytes(h[18..22].try_into().unwrap()) != sequence
            || ((h[5] & 1) != 0) != !packet.is_empty()
        {
            return Err(invalid());
        }
        serial = Some(id);
        sequence = sequence.checked_add(1).ok_or_else(invalid)?;
        let mut lacing = vec![0; h[26] as usize];
        file.read_exact(&mut lacing).map_err(|_| invalid())?;
        for len in lacing {
            let start = packet.len();
            if start + len as usize > 65536 {
                return Err(invalid());
            }
            packet.resize(start + len as usize, 0);
            file.read_exact(&mut packet[start..])
                .map_err(|_| invalid())?;
            if len == 255 {
                continue;
            }
            match packets {
                0 => {
                    if packet.len() != 19
                        || &packet[..8] != b"OpusHead"
                        || packet[8] != 1
                        || packet[9] != 1
                        || packet[18] != 0
                        || u32::from_le_bytes(packet[12..16].try_into().unwrap()) != 48000
                    {
                        return Err(invalid());
                    }
                    skip = u16::from_le_bytes(packet[10..12].try_into().unwrap());
                }
                1 => {
                    if !packet.starts_with(b"OpusTags") {
                        return Err(invalid());
                    }
                }
                _ => {
                    let toc = *packet.first().ok_or_else(invalid)?;
                    let config = toc >> 3;
                    let samples = if config >= 16 {
                        120u64 << (config & 3)
                    } else if config >= 12 {
                        480u64 << (config & 1)
                    } else if config & 3 == 3 {
                        2880
                    } else {
                        480u64 << (config & 3)
                    };
                    let count = match toc & 3 {
                        0 => 1,
                        1 | 2 => 2,
                        _ => u64::from(*packet.get(1).ok_or_else(invalid)? & 63),
                    };
                    let duration = samples * count;
                    if duration == 0 || duration > 5760 {
                        return Err(invalid());
                    }
                    decoded = decoded.checked_add(duration).ok_or_else(invalid)?;
                }
            }
            packets += 1;
            packet.clear();
        }
        let granule = u64::from_le_bytes(h[6..14].try_into().unwrap());
        if granule != u64::MAX {
            last = granule;
        }
        eos = h[5] & 4 != 0;
    }
    if !eos
        || !packet.is_empty()
        || packets < 3
        || last != frames.checked_add(u64::from(skip)).ok_or_else(invalid)?
        || decoded < last
        || decoded - last >= 5760
    {
        return Err(invalid());
    }
    Ok((skip, decoded - last))
}
