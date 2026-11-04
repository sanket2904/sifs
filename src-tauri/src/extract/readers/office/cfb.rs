//! Just enough of the OLE compound file format ([MS-CFB]) to list the
//! streams in one: Office wraps a password-protected file in one, with the
//! streams EncryptionInfo and EncryptedPackage, and old Word, Excel and
//! PowerPoint files (doc, xls, ppt) are compound files too.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub(crate) const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// Directory sectors followed at most, so a chain that loops ends.
const MAX_DIRECTORY_SECTORS: usize = 1000;
/// Sector numbers from here up are markers (end of chain, free, ...).
const MAX_SECTOR: u32 = 0xFFFF_FFFA;
/// The FAT sectors the header lists; a file needing more is not followed.
const HEADER_DIFAT: usize = 109;

/// The names of the entries in the compound file's directory, or None if
/// its header or directory can't be followed.
pub(crate) fn stream_names(file: &mut File) -> Option<Vec<String>> {
    let mut header = [0u8; 512];
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_exact(&mut header).ok()?;
    if header[..8] != MAGIC {
        return None;
    }
    let shift = u16::from_le_bytes([header[0x1E], header[0x1F]]);
    if shift != 9 && shift != 12 {
        return None;
    }
    let size = 1u64 << shift;
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let difat: Vec<u32> = (0..HEADER_DIFAT).map(|i| u32_at(&header, 0x4C + 4 * i)).collect();
    let mut sector = vec![0u8; size as usize];
    let read_sector = |file: &mut File, sid: u32, into: &mut [u8]| -> Option<()> {
        file.seek(SeekFrom::Start((u64::from(sid) + 1) * size)).ok()?;
        file.read_exact(into).ok()
    };
    // The next sector after sid, from the FAT.
    let per_fat = (size / 4) as u32;
    let mut fat = vec![0u8; size as usize];
    let mut next = |file: &mut File, sid: u32| -> Option<u32> {
        let fat_sector = *difat.get((sid / per_fat) as usize)?;
        if fat_sector >= MAX_SECTOR {
            return None;
        }
        file.seek(SeekFrom::Start((u64::from(fat_sector) + 1) * size + u64::from(sid % per_fat) * 4)).ok()?;
        file.read_exact(&mut fat[..4]).ok()?;
        Some(u32_at(&fat, 0))
    };
    let mut names = Vec::new();
    let mut sid = u32_at(&header, 0x30);
    for _ in 0..MAX_DIRECTORY_SECTORS {
        if sid >= MAX_SECTOR {
            return Some(names);
        }
        read_sector(file, sid, &mut sector)?;
        for entry in sector.as_chunks::<128>().0 {
            // The name's length in bytes, with its terminating zero.
            let len = (u16::from_le_bytes([entry[0x40], entry[0x41]]) as usize).min(64);
            if len >= 2 && entry[0x42] != 0 {
                let units: Vec<u16> =
                    entry[..len - 2].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                names.push(String::from_utf16_lossy(&units));
            }
        }
        sid = next(file, sid)?;
    }
    Some(names)
}
