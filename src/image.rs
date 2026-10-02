//! Bounds checked access to the image file and the file system abstraction shared by the
//! ISO 9660 and UDF readers.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

use anyhow::{Context, Result, bail};

pub const SECTOR_SIZE: u64 = 2048;

/// Upper bound for a single read, all structures we read are small.
const MAX_READ: u64 = 64 * 1024 * 1024;

pub struct Image {
    file: File,
    size: u64,
}

impl Image {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("unable to open {path:?}"))?;
        let size = file.metadata()?.len();
        Ok(Self { file, size })
    }

    pub fn read_at(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        if len > MAX_READ {
            bail!("read of {len} bytes is too large");
        }
        let end = offset.checked_add(len).context("read offset overflow")?;
        if end > self.size {
            bail!("read beyond the end of the image ({end} > {})", self.size);
        }
        let mut buf = vec![0u8; len as usize];
        self.file.read_exact_at(&mut buf, offset)?;
        Ok(buf)
    }

    pub fn read_sector(&self, sector: u64) -> Result<Vec<u8>> {
        let offset = sector.checked_mul(SECTOR_SIZE).context("sector offset overflow")?;
        self.read_at(offset, SECTOR_SIZE)
    }
}

/// A contiguous piece of a file in the image.
#[derive(Clone, Debug)]
pub struct Extent {
    /// byte offset in the image
    pub offset: u64,
    pub len: u64,
}

#[derive(Clone, Debug)]
pub enum Data {
    Extents(Vec<Extent>),
    /// small files can be stored inside the UDF file entry
    Embedded(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub data: Data,
}

impl Entry {
    /// Read `len` bytes at `offset` of the file content.
    pub fn read(&self, image: &Image, offset: u64, len: u64) -> Result<Vec<u8>> {
        let end = offset.checked_add(len).context("read offset overflow")?;
        if end > self.size {
            bail!("read beyond the end of '{}' ({end} > {})", self.name, self.size);
        }
        match &self.data {
            Data::Embedded(data) => Ok(data[offset as usize..end as usize].to_vec()),
            Data::Extents(extents) => {
                let mut res = Vec::with_capacity(len as usize);
                let mut pos = 0u64; // position in the file of the current extent
                for extent in extents {
                    let ext_end = pos + extent.len;
                    if ext_end > offset && pos < end {
                        let from = offset.max(pos);
                        let to = end.min(ext_end);
                        res.extend(image.read_at(extent.offset + (from - pos), to - from)?);
                    }
                    pos = ext_end;
                    if pos >= end {
                        break;
                    }
                }
                if res.len() as u64 != len {
                    bail!("'{}' is shorter than its recorded size", self.name);
                }
                Ok(res)
            }
        }
    }

    pub fn read_all(&self, image: &Image, max: u64) -> Result<Vec<u8>> {
        if self.size > max {
            bail!("'{}' is too large ({} bytes)", self.name, self.size);
        }
        self.read(image, 0, self.size)
    }
}

pub trait FileSystem {
    /// volume label
    fn label(&self) -> &str;
    fn root(&self) -> Result<Entry>;
    fn list(&self, image: &Image, dir: &Entry) -> Result<Vec<Entry>>;

    /// Case insensitive lookup of a slash separated path.
    fn lookup(&self, image: &Image, path: &str) -> Result<Option<Entry>> {
        let mut entry = self.root()?;
        for component in path.split('/').filter(|c| !c.is_empty()) {
            if !entry.is_dir {
                return Ok(None);
            }
            let found =
                self.list(image, &entry)?.into_iter().find(|e| e.name.eq_ignore_ascii_case(component));
            match found {
                Some(e) => entry = e,
                None => return Ok(None),
            }
        }
        Ok(Some(entry))
    }
}

pub fn u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

pub fn u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

pub fn u64_le(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

/// Check that `buf` holds at least `len` bytes from `off`, so the helpers above do not panic.
pub fn ensure(buf: &[u8], off: usize, len: usize, what: &str) -> Result<()> {
    match off.checked_add(len) {
        Some(end) if end <= buf.len() => Ok(()),
        _ => bail!("truncated {what}"),
    }
}
