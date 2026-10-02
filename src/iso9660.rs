//! Minimal read-only ISO 9660 reader with Joliet and Rock Ridge (NM) names.

use anyhow::{Result, bail};

use crate::image::{Data, Entry, Extent, FileSystem, Image, SECTOR_SIZE, ensure, u32_le};

const MAX_DIR_SIZE: u64 = 16 * 1024 * 1024;

pub struct Iso9660 {
    label: String,
    root: Entry,
    joliet: bool,
}

impl Iso9660 {
    pub fn open(image: &Image) -> Result<Option<Self>> {
        let mut primary = None;
        let mut joliet = None;

        for sector in 16..16 + 64 {
            let Ok(desc) = image.read_sector(sector) else {
                break;
            };
            if &desc[1..6] != b"CD001" {
                break;
            }
            match desc[0] {
                1 => primary = Some(desc),
                // supplementary descriptor with a UCS-2 escape sequence is Joliet
                2 if desc[88] == 0x25 && desc[89] == 0x2f && matches!(desc[90], 0x40 | 0x43 | 0x45) => {
                    joliet = Some(desc)
                }
                255 => break,
                _ => {}
            }
        }

        let label = primary
            .as_ref()
            .map(|desc| String::from_utf8_lossy(&desc[40..72]).trim().to_string())
            .unwrap_or_default();

        let (desc, is_joliet) = match (joliet, primary) {
            (Some(desc), _) => (desc, true),
            (None, Some(desc)) => (desc, false),
            (None, None) => return Ok(None),
        };

        // the root directory record is at offset 156 of the volume descriptor
        let record = &desc[156..190];
        let size = u32_le(record, 10) as u64;
        if record[25] & 0x02 == 0 {
            bail!("invalid root directory record");
        }
        let root = Entry {
            name: String::new(),
            is_dir: true,
            size,
            data: Data::Extents(vec![Extent { offset: u32_le(record, 2) as u64 * SECTOR_SIZE, len: size }]),
        };

        Ok(Some(Self { label, root, joliet: is_joliet }))
    }
}

impl FileSystem for Iso9660 {
    fn label(&self) -> &str {
        &self.label
    }

    fn root(&self) -> Result<Entry> {
        Ok(self.root.clone())
    }

    fn list(&self, image: &Image, dir: &Entry) -> Result<Vec<Entry>> {
        let data = dir.read_all(image, MAX_DIR_SIZE)?;
        let mut res: Vec<Entry> = Vec::new();
        let mut off = 0usize;
        // multi-extent files consist of several records with the same name
        let mut continued = false;

        while off < data.len() {
            let len = data[off] as usize;
            if len == 0 {
                // records do not cross sector boundaries, continue with the next sector
                off = (off / SECTOR_SIZE as usize + 1) * SECTOR_SIZE as usize;
                continue;
            }
            ensure(&data, off, len, "directory record")?;
            let record = &data[off..off + len];
            off += len;

            let Some((entry, more)) = parse_record(record, self.joliet)? else {
                continue; // '.' and '..'
            };

            match (continued, res.last_mut()) {
                (true, Some(last)) => {
                    last.size += entry.size;
                    if let (Data::Extents(all), Data::Extents(new)) = (&mut last.data, entry.data) {
                        all.extend(new);
                    }
                }
                _ => res.push(entry),
            }
            continued = more;
        }

        Ok(res)
    }
}

/// Parse a directory record, returns the entry and whether more extents follow.
fn parse_record(record: &[u8], joliet: bool) -> Result<Option<(Entry, bool)>> {
    ensure(record, 0, 33, "directory record")?;
    let name_len = record[32] as usize;
    ensure(record, 33, name_len, "directory record name")?;
    let raw_name = &record[33..33 + name_len];

    if name_len == 1 && (raw_name[0] == 0 || raw_name[0] == 1) {
        return Ok(None); // '.' and '..'
    }

    let flags = record[25];
    let lba = u32_le(record, 2) as u64;
    let size = u32_le(record, 10) as u64;

    let mut name = if joliet {
        let units: Vec<u16> = raw_name.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        // system use area with Rock Ridge entries follows the (even padded) name
        let su_start = 33 + name_len + (1 - name_len % 2);
        rock_ridge_name(record.get(su_start..).unwrap_or(&[]))
            .unwrap_or_else(|| String::from_utf8_lossy(raw_name).into_owned())
    };

    if let Some(pos) = name.find(';') {
        name.truncate(pos);
    }
    if !joliet && name.ends_with('.') {
        name.pop();
    }

    Ok(Some((
        Entry {
            name,
            is_dir: flags & 0x02 != 0,
            size,
            data: Data::Extents(vec![Extent { offset: lba * SECTOR_SIZE, len: size }]),
        },
        flags & 0x80 != 0,
    )))
}

fn rock_ridge_name(mut su: &[u8]) -> Option<String> {
    let mut name = Vec::new();
    while su.len() >= 4 {
        let len = su[2] as usize;
        if len < 4 || len > su.len() {
            break;
        }
        if &su[0..2] == b"NM" && len >= 5 {
            let flags = su[4];
            // skip '.' / '..' references
            if flags & 0x06 == 0 {
                name.extend_from_slice(&su[5..len]);
            }
        }
        su = &su[len..];
    }
    if name.is_empty() { None } else { Some(String::from_utf8_lossy(&name).into_owned()) }
}
