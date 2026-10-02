//! Minimal read-only UDF reader (ECMA-167 / OSTA UDF), enough to walk directories and locate
//! files on installation media, which commonly use UDF 1.02 to store files > 4 GiB.

use anyhow::{Context, Result, bail};

use crate::image::{Data, Entry, Extent, FileSystem, Image, SECTOR_SIZE, ensure, u16_le, u32_le, u64_le};

const TAG_AVDP: u16 = 2;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_TD: u16 = 8;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_FE: u16 = 261;
const TAG_EFE: u16 = 266;

const MAX_DIR_SIZE: u64 = 16 * 1024 * 1024;
const MAX_EXTENTS: usize = 64 * 1024;

pub struct Udf {
    /// byte offset of the partition start
    partition_offset: u64,
    block_size: u64,
    label: String,
    root: Entry,
}

/// Check for a UDF volume recognition sequence (NSR02/NSR03 descriptor).
pub fn detect(image: &Image) -> bool {
    for sector in 16..16 + 64 {
        let Ok(desc) = image.read_sector(sector) else {
            return false;
        };
        match &desc[1..6] {
            b"NSR02" | b"NSR03" => return true,
            b"BEA01" | b"CD001" | b"CDW02" | b"BOOT2" => continue,
            _ => return false,
        }
    }
    false
}

fn tag_id(buf: &[u8]) -> u16 {
    u16_le(buf, 0)
}

impl Udf {
    pub fn open(image: &Image) -> Result<Self> {
        let avdp = image.read_sector(256)?;
        if tag_id(&avdp) != TAG_AVDP {
            bail!("no UDF anchor volume descriptor at sector 256");
        }
        let vds_len = u32_le(&avdp, 16) as u64;
        let vds_loc = u32_le(&avdp, 20) as u64;

        let mut partitions = Vec::new(); // (number, start sector)
        let mut lvd = None;
        for i in 0..(vds_len / SECTOR_SIZE).min(64) {
            let desc = image.read_sector(vds_loc + i)?;
            match tag_id(&desc) {
                TAG_PD => partitions.push((u16_le(&desc, 22), u32_le(&desc, 188) as u64)),
                TAG_LVD => lvd = Some(desc),
                TAG_TD => break,
                _ => {}
            }
        }
        let lvd = lvd.context("no UDF logical volume descriptor")?;

        let block_size = u32_le(&lvd, 212) as u64;
        if block_size != SECTOR_SIZE {
            bail!("unsupported UDF block size {block_size}");
        }

        // only type 1 partition maps are supported (no metadata or virtual partitions)
        let num_maps = u32_le(&lvd, 268);
        ensure(&lvd, 440, 6, "partition map")?;
        if num_maps < 1 || lvd[440] != 1 {
            bail!("unsupported UDF partition map (type {})", lvd[440]);
        }
        let partition_number = u16_le(&lvd, 444);
        let partition_start = partitions
            .iter()
            .find(|(number, _)| *number == partition_number)
            .map(|(_, start)| *start)
            .context("UDF partition not found")?;

        // file set descriptor (long_ad at 248)
        let fsd_lbn = u32_le(&lvd, 252) as u64;

        let mut udf = Self {
            partition_offset: partition_start * SECTOR_SIZE,
            block_size,
            label: decode_dstring(dstring(&lvd[84..212])),
            root: Entry { name: String::new(), is_dir: true, size: 0, data: Data::Embedded(Vec::new()) },
        };

        let fsd = udf.read_block(image, fsd_lbn)?;
        if tag_id(&fsd) != TAG_FSD {
            bail!("no UDF file set descriptor");
        }
        // root directory ICB (long_ad at 400)
        udf.root = udf.file_entry(image, u32_le(&fsd, 404) as u64, String::new())?;
        if !udf.root.is_dir {
            bail!("UDF root is not a directory");
        }

        Ok(udf)
    }

    fn block_offset(&self, lbn: u64) -> Result<u64> {
        lbn.checked_mul(self.block_size)
            .and_then(|o| o.checked_add(self.partition_offset))
            .context("UDF block offset overflow")
    }

    fn read_block(&self, image: &Image, lbn: u64) -> Result<Vec<u8>> {
        image.read_at(self.block_offset(lbn)?, self.block_size)
    }

    /// Read the (extended) file entry at `lbn`.
    fn file_entry(&self, image: &Image, lbn: u64, name: String) -> Result<Entry> {
        let fe = self.read_block(image, lbn)?;
        let (ea_off, ad_off) = match tag_id(&fe) {
            TAG_FE => (168, 176),
            TAG_EFE => (208, 216),
            id => bail!("unexpected UDF tag {id} for file entry of '{name}'"),
        };

        // ICB tag starts at 16: file type at +11, flags at +18
        let file_type = fe[16 + 11];
        let flags = u16_le(&fe, 16 + 18);
        let size = u64_le(&fe, 56);
        let ea_len = u32_le(&fe, ea_off) as usize;
        let ad_len = u32_le(&fe, ea_off + 4) as usize;
        let start = ad_off + ea_len;
        ensure(&fe, start, ad_len, "UDF allocation descriptors")?;
        let ads = &fe[start..start + ad_len];

        let data = match flags & 0x07 {
            0 => Data::Extents(self.extents(ads, 8)?),
            1 => Data::Extents(self.extents(ads, 16)?),
            3 => {
                if (ad_len as u64) < size {
                    bail!("UDF embedded data of '{name}' is too short");
                }
                Data::Embedded(ads[..size as usize].to_vec())
            }
            t => bail!("unsupported UDF allocation type {t} for '{name}'"),
        };

        Ok(Entry { name, is_dir: file_type == 4, size, data })
    }

    /// Parse short_ad (8 bytes) or long_ad (16 bytes) allocation descriptors.
    fn extents(&self, ads: &[u8], ad_size: usize) -> Result<Vec<Extent>> {
        let mut res = Vec::new();
        for ad in ads.chunks_exact(ad_size) {
            let raw_len = u32_le(ad, 0);
            let len = (raw_len & 0x3fff_ffff) as u64;
            let kind = raw_len >> 30;
            if len == 0 {
                break;
            }
            match kind {
                0 => res.push(Extent { offset: self.block_offset(u32_le(ad, 4) as u64)?, len }),
                3 => bail!("UDF allocation descriptor continuation is not supported"),
                // allocated but not recorded / not allocated: holes, should not exist on media
                _ => bail!("sparse UDF files are not supported"),
            }
            if res.len() > MAX_EXTENTS {
                bail!("too many UDF extents");
            }
        }
        Ok(res)
    }
}

impl FileSystem for Udf {
    fn label(&self) -> &str {
        &self.label
    }

    fn root(&self) -> Result<Entry> {
        Ok(self.root.clone())
    }

    fn list(&self, image: &Image, dir: &Entry) -> Result<Vec<Entry>> {
        let data = dir.read_all(image, MAX_DIR_SIZE)?;
        let mut res = Vec::new();
        let mut off = 0usize;

        while off + 38 <= data.len() {
            let fid = &data[off..];
            if tag_id(fid) != TAG_FID {
                break;
            }
            let characteristics = fid[18];
            let name_len = fid[19] as usize;
            let icb_lbn = u32_le(fid, 24) as u64;
            let iu_len = u16_le(fid, 36) as usize;
            let total = (38 + iu_len + name_len + 3) & !3;
            ensure(fid, 0, 38 + iu_len + name_len, "UDF file identifier")?;
            off += total;

            // skip deleted entries and the parent directory
            if characteristics & 0x0c != 0 || name_len == 0 {
                continue;
            }
            let name = decode_dstring(&fid[38 + iu_len..38 + iu_len + name_len]);
            res.push(self.file_entry(image, icb_lbn, name)?);
        }

        Ok(res)
    }
}

/// Fixed size dstring, the last byte holds the used length.
fn dstring(buf: &[u8]) -> &[u8] {
    let len = *buf.last().unwrap_or(&0) as usize;
    &buf[..len.min(buf.len().saturating_sub(1))]
}

/// Decode an OSTA compressed unicode string (compression ID 8 or 16).
fn decode_dstring(buf: &[u8]) -> String {
    match buf.first() {
        Some(8) => buf[1..].iter().map(|&b| b as char).collect::<String>().trim_end_matches('\0').to_string(),
        Some(16) => {
            let units: Vec<u16> =
                buf[1..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::new(),
    }
}
