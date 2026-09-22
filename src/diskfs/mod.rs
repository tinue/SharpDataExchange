//! The PC-1600 FAT-style filesystem on one CE-1600F floppy side.
//!
//! Sharp's format, documented in `SharpPC1500Reference/PC-1600/PC-1600-Filesystem.md` §5
//! and `PC-1600-Peripherals-Hardware.md` §2; nothing here knows about the container file
//! the side came from (see [`crate::floppy_image`]). A [`Volume`] works on a caller-owned
//! 64 KB buffer, so it serves the CLI and the C ABI alike.
//!
//! Layout of a side (512-byte logical sectors, sector = track × 8 + sector):
//!
//! | Sector | Content |
//! |---|---|
//! | 0 | boot sector — all zero on emulator-formatted disks, never checked |
//! | 1 | FAT: byte 0 = media id `F2`, bytes 1..=122 = clusters 1..=122 |
//! | 2 | FAT copy — kept identical to the FAT (as the ROM does); never read |
//! | 3–5 | directory, 48 × 32-byte entries |
//! | 6–127 | data: cluster *c* is sector *c* + 5 |
//!
//! FAT entry `00` = free, `01..=7A` = next cluster, `F0` = last cluster (the RAM disk uses
//! `FF`). Like the ROM, sde writes only FAT bytes 0..=122, to both copies; the rest of each
//! sector holds leftover buffer RAM on real disks and is preserved.
//!
//! What a new file gets was checked against files the PC-1600 ROM wrote in Calc-U-1600
//! (`tools/e2e_floppy.sh`): attribute `20`, reserved bytes zero, seconds stored in the
//! time word, year field 6, first free directory slot (a `KILL`ed one is reused) and
//! first free clusters.

mod name;
mod time;

pub use name::{FileName, Pattern};
pub use time::{DosTimestamp, NEW_FILE_YEAR_FIELD};

/// Directory-entry byte 0 of a deleted entry.
const DELETED: u8 = 0xE5;
/// Attribute of a newly written file (bit 5 is always set; nothing else).
pub const NEW_FILE_ATTR: u8 = 0x20;
pub const ATTR_PROTECTED: u8 = 0x01;
pub const ATTR_UNKNOWN_I: u8 = 0x02;
pub const ATTR_HIDDEN: u8 = 0x04;
const DIR_ENTRY_LEN: usize = 32;

/// Volume geometry. Only the CE-1600F floppy side is implemented; a RAM-disk card would
/// read these values from its boot sector instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub sector_size: usize,
    pub total_sectors: usize,
    pub media_id: u8,
    pub fat_sector: usize,
    /// Sector of the FAT copy, kept identical to the FAT on every write.
    pub fat_copy_sector: Option<usize>,
    pub dir_sector: usize,
    pub dir_entries: usize,
    pub data_sector: usize,
    /// Highest valid cluster number (clusters are numbered from 1).
    pub max_cluster: u8,
    /// FAT value marking the last cluster of a file.
    pub end_of_chain: u8,
}

impl Geometry {
    /// A CE-1600F side: ROM geometry row F2 (MAXCLS 7B → clusters 1..=122).
    pub const CE1600F: Geometry = Geometry {
        sector_size: 512,
        total_sectors: 128,
        media_id: 0xF2,
        fat_sector: 1,
        fat_copy_sector: Some(2),
        dir_sector: 3,
        dir_entries: 48,
        data_sector: 6,
        max_cluster: 122,
        end_of_chain: 0xF0,
    };

    pub fn volume_size(&self) -> usize {
        self.sector_size * self.total_sectors
    }

    fn cluster_offset(&self, cluster: u8) -> usize {
        (self.data_sector + cluster as usize - 1) * self.sector_size
    }

    fn fat_offset(&self) -> usize {
        self.fat_sector * self.sector_size
    }

    fn dir_offset(&self, slot: usize) -> usize {
        self.dir_sector * self.sector_size + slot * DIR_ENTRY_LEN
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskError {
    /// The FAT does not start with the expected media id (e.g. a blank side).
    NotFormatted,
    NotFound(String),
    Exists(String),
    Protected(String),
    DiskFull { needed: usize, free: usize },
    DirectoryFull,
    BadName(String),
    Corrupt(String),
}

impl std::fmt::Display for DiskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiskError::NotFormatted => write!(f, "side is not formatted (no CE-1600F FAT)"),
            DiskError::NotFound(n) => write!(f, "file not found: {n}"),
            DiskError::Exists(n) => write!(f, "{n} already exists (use --force to replace it)"),
            DiskError::Protected(n) => write!(f, "{n} is write-protected (use --force)"),
            DiskError::DiskFull { needed, free } => {
                write!(f, "disk full: need {needed} bytes, {free} bytes free")
            }
            DiskError::DirectoryFull => write!(f, "directory full (48 files per side)"),
            DiskError::BadName(n) => write!(f, "invalid file name {n}"),
            DiskError::Corrupt(why) => write!(f, "filesystem is corrupt: {why}"),
        }
    }
}

impl std::error::Error for DiskError {}

/// One used directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// Directory slot (0-based).
    pub slot: usize,
    pub name: FileName,
    pub attr: u8,
    pub time: u16,
    pub date: u16,
    pub start_cluster: u8,
    /// File size in bytes, including a 16-byte header if the file has one.
    pub size: u32,
}

impl DirEntry {
    pub fn is_protected(&self) -> bool {
        self.attr & ATTR_PROTECTED != 0
    }

    pub fn is_hidden(&self) -> bool {
        self.attr & ATTR_HIDDEN != 0
    }

    pub fn timestamp(&self) -> DosTimestamp {
        DosTimestamp::unpack(self.time, self.date)
    }

    /// `P`, `I`, `H` for the protect / unknown / hidden attribute bits, `-` when clear.
    pub fn attr_letters(&self) -> String {
        [(ATTR_PROTECTED, 'P'), (ATTR_UNKNOWN_I, 'I'), (ATTR_HIDDEN, 'H')]
            .iter()
            .map(|&(bit, c)| if self.attr & bit != 0 { c } else { '-' })
            .collect()
    }
}

/// A mounted volume over a byte buffer holding exactly one side.
pub struct Volume<B> {
    geo: Geometry,
    buf: B,
}

impl<B: AsRef<[u8]>> Volume<B> {
    /// Mount a CE-1600F side. Fails with [`DiskError::NotFormatted`] unless FAT byte 0 is
    /// the media id `F2`.
    pub fn open_floppy_side(buf: B) -> Result<Self, DiskError> {
        Self::open(buf, Geometry::CE1600F)
    }

    fn open(buf: B, geo: Geometry) -> Result<Self, DiskError> {
        let data = buf.as_ref();
        if data.len() != geo.volume_size() {
            return Err(DiskError::Corrupt(format!(
                "side is {} bytes, expected {}",
                data.len(),
                geo.volume_size()
            )));
        }
        if data[geo.fat_offset()] != geo.media_id {
            return Err(DiskError::NotFormatted);
        }
        Ok(Volume { geo, buf })
    }

    pub fn geometry(&self) -> &Geometry {
        &self.geo
    }

    pub fn into_inner(self) -> B {
        self.buf
    }

    fn bytes(&self) -> &[u8] {
        self.buf.as_ref()
    }

    fn fat(&self, cluster: u8) -> u8 {
        self.bytes()[self.geo.fat_offset() + cluster as usize]
    }

    fn raw_entry(&self, slot: usize) -> &[u8] {
        let off = self.geo.dir_offset(slot);
        &self.bytes()[off..off + DIR_ENTRY_LEN]
    }

    /// Used entries in directory order. The scan stops at the first never-used entry
    /// (first byte `00`); deleted entries (`E5`) are skipped.
    pub fn list(&self) -> Vec<DirEntry> {
        let mut out = Vec::new();
        for slot in 0..self.geo.dir_entries {
            let e = self.raw_entry(slot);
            match e[0] {
                0x00 => break,
                DELETED => continue,
                _ => {}
            }
            out.push(DirEntry {
                slot,
                name: FileName::from_raw(&e[0..11]),
                attr: e[0x0B],
                time: u16::from_le_bytes([e[0x16], e[0x17]]),
                date: u16::from_le_bytes([e[0x18], e[0x19]]),
                start_cluster: e[0x1A],
                size: u32::from_le_bytes([e[0x1C], e[0x1D], e[0x1E], e[0x1F]]),
            });
        }
        out
    }

    pub fn find(&self, name: &FileName) -> Option<DirEntry> {
        self.list().into_iter().find(|e| e.name == *name)
    }

    pub fn glob(&self, pattern: &Pattern) -> Vec<DirEntry> {
        self.list().into_iter().filter(|e| pattern.matches(&e.name)).collect()
    }

    /// The cluster chain of `entry`, validated (no loops, no free or out-of-range links,
    /// long enough for the file's size).
    pub fn chain(&self, entry: &DirEntry) -> Result<Vec<u8>, DiskError> {
        let needed = clusters_for(entry.size as usize, self.geo.sector_size);
        let mut chain = Vec::new();
        if entry.size == 0 && entry.start_cluster == 0 {
            return Ok(chain);
        }
        let mut c = entry.start_cluster;
        loop {
            if c == 0 || c > self.geo.max_cluster {
                return Err(DiskError::Corrupt(format!("{}: cluster {c:#04X} out of range", entry.name)));
            }
            if chain.contains(&c) {
                return Err(DiskError::Corrupt(format!("{}: cluster chain loops at {c:#04X}", entry.name)));
            }
            chain.push(c);
            let next = self.fat(c);
            if next == self.geo.end_of_chain {
                break;
            }
            if next == 0 {
                return Err(DiskError::Corrupt(format!("{}: chain runs into free cluster", entry.name)));
            }
            c = next;
        }
        if chain.len() < needed {
            return Err(DiskError::Corrupt(format!(
                "{}: {} clusters for {} bytes",
                entry.name,
                chain.len(),
                entry.size
            )));
        }
        Ok(chain)
    }

    /// The file's bytes, exactly `entry.size` of them.
    pub fn read(&self, entry: &DirEntry) -> Result<Vec<u8>, DiskError> {
        let cs = self.geo.sector_size;
        let mut out = Vec::with_capacity(entry.size as usize);
        for c in self.chain(entry)? {
            let off = self.geo.cluster_offset(c);
            out.extend_from_slice(&self.bytes()[off..off + cs]);
            if out.len() >= entry.size as usize {
                break;
            }
        }
        out.truncate(entry.size as usize);
        Ok(out)
    }

    pub fn free_clusters(&self) -> usize {
        (1..=self.geo.max_cluster).filter(|&c| self.fat(c) == 0).count()
    }

    /// Free space in bytes (what `DSKF` reports).
    pub fn free_bytes(&self) -> usize {
        self.free_clusters() * self.geo.sector_size
    }

    fn free_slot(&self) -> Option<usize> {
        (0..self.geo.dir_entries).find(|&s| matches!(self.raw_entry(s)[0], 0x00 | DELETED))
    }
}

impl<B: AsRef<[u8]> + AsMut<[u8]>> Volume<B> {
    fn bytes_mut(&mut self) -> &mut [u8] {
        self.buf.as_mut()
    }

    fn set_fat(&mut self, cluster: u8, value: u8) {
        let off = self.geo.fat_offset() + cluster as usize;
        self.bytes_mut()[off] = value;
        if let Some(copy) = self.geo.fat_copy_sector {
            let off = copy * self.geo.sector_size + cluster as usize;
            self.bytes_mut()[off] = value;
        }
    }

    /// Store `data` as `name`. An existing file of that name is replaced only with
    /// `force` (which also overrides write protection). Everything is checked before
    /// anything changes, so an error leaves the volume untouched.
    pub fn write(
        &mut self,
        name: &FileName,
        data: &[u8],
        ts: DosTimestamp,
        force: bool,
    ) -> Result<DirEntry, DiskError> {
        let existing = self.find(name);
        if let Some(e) = &existing {
            if !force {
                return Err(if e.is_protected() {
                    DiskError::Protected(name.to_string())
                } else {
                    DiskError::Exists(name.to_string())
                });
            }
        }
        let old_chain = match &existing {
            Some(e) => self.chain(e)?,
            None => Vec::new(),
        };
        let cs = self.geo.sector_size;
        // Even an empty file gets one cluster, so its entry never points at cluster 0.
        let needed = clusters_for(data.len(), cs).max(1);
        let available = self.free_clusters() + old_chain.len();
        if needed > available {
            return Err(DiskError::DiskFull { needed: data.len(), free: available * cs });
        }
        let slot = match &existing {
            Some(e) => e.slot,
            None => self.free_slot().ok_or(DiskError::DirectoryFull)?,
        };

        for &c in &old_chain {
            self.set_fat(c, 0);
        }
        let clusters: Vec<u8> =
            (1..=self.geo.max_cluster).filter(|&c| self.fat(c) == 0).take(needed).collect();
        for (i, &c) in clusters.iter().enumerate() {
            let next = clusters.get(i + 1).copied().unwrap_or(self.geo.end_of_chain);
            self.set_fat(c, next);
            let off = self.geo.cluster_offset(c);
            let chunk = data.get(i * cs..).map_or(&[][..], |d| &d[..d.len().min(cs)]);
            let dst = &mut self.bytes_mut()[off..off + cs];
            dst.fill(0);
            dst[..chunk.len()].copy_from_slice(chunk);
        }

        let (time, date) = ts.pack();
        let entry = DirEntry {
            slot,
            name: *name,
            attr: NEW_FILE_ATTR,
            time,
            date,
            start_cluster: clusters[0],
            size: data.len() as u32,
        };
        let off = self.geo.dir_offset(slot);
        let e = &mut self.bytes_mut()[off..off + DIR_ENTRY_LEN];
        e.fill(0);
        e[0..11].copy_from_slice(&name.raw());
        e[0x0B] = entry.attr;
        e[0x16..0x18].copy_from_slice(&time.to_le_bytes());
        e[0x18..0x1A].copy_from_slice(&date.to_le_bytes());
        e[0x1A] = entry.start_cluster;
        e[0x1C..0x20].copy_from_slice(&entry.size.to_le_bytes());
        Ok(entry)
    }

    /// Delete `entry`: mark it `E5` and free its clusters. Write-protected files need
    /// `force`.
    pub fn delete(&mut self, entry: &DirEntry, force: bool) -> Result<(), DiskError> {
        if entry.is_protected() && !force {
            return Err(DiskError::Protected(entry.name.to_string()));
        }
        for c in self.chain(entry)? {
            self.set_fat(c, 0);
        }
        let off = self.geo.dir_offset(entry.slot);
        self.bytes_mut()[off] = DELETED;
        Ok(())
    }
}

fn clusters_for(len: usize, cluster_size: usize) -> usize {
    len.div_ceil(cluster_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floppy_image::{self, Side, SIDE_SIZE};

    fn side(fixture: &str, side: Side) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floppy").join(fixture);
        floppy_image::read_path(&p).unwrap().side(side).to_vec()
    }

    fn name(s: &str) -> FileName {
        FileName::parse(s).unwrap()
    }

    const TS: DosTimestamp = DosTimestamp { year_field: 6, month: 9, day: 22, hour: 13, minute: 45, second: 30 };

    #[test]
    fn dw_listing() {
        let vol = Volume::open_floppy_side(side("dw.floppy.yaml", Side::A)).unwrap();
        let list = vol.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name.to_string(), "GLOBUS.BAS");
        assert_eq!((list[0].size, list[0].start_cluster, list[0].attr), (5967, 1, 0x20));
        assert_eq!(list[0].timestamp().to_string(), "01-01 00:02:18");
        assert_eq!(list[1].name.to_string(), "BIO.BAS");
        assert_eq!((list[1].size, list[1].start_cluster), (2059, 13));
        assert_eq!(vol.chain(&list[0]).unwrap(), (1..=12).collect::<Vec<u8>>());
        assert_eq!(vol.free_clusters(), 122 - 12 - 5);

        let data = vol.read(&list[0]).unwrap();
        assert_eq!(data.len(), 5967);
        assert_eq!(&data[..5], &[0xFF, 0x10, 0x00, 0x00, 0x21]);
        let h = crate::header::find(&data).unwrap();
        assert_eq!(h.payload_start() + h.length, data.len());
    }

    #[test]
    fn blank_side_is_not_formatted() {
        assert_eq!(Volume::open_floppy_side(side("dw.floppy.yaml", Side::B)).err(), Some(DiskError::NotFormatted));
        assert!(matches!(Volume::open_floppy_side(vec![0u8; 100]), Err(DiskError::Corrupt(_))));
    }

    #[test]
    fn formatted_side_free_space() {
        let vol = Volume::open_floppy_side(side("formatted.floppy.yaml", Side::A)).unwrap();
        assert!(vol.list().is_empty());
        assert_eq!(vol.free_bytes(), 62464);
    }

    #[test]
    fn write_touches_only_fat_heads_dir_slot_and_clusters() {
        let original = side("dw.floppy.yaml", Side::A);
        let mut vol = Volume::open_floppy_side(original.clone()).unwrap();
        let data: Vec<u8> = (0..1100).map(|i| i as u8).collect();
        let e = vol.write(&name("new.bin"), &data, TS, false).unwrap();
        assert_eq!((e.slot, e.start_cluster, e.size), (2, 18, 1100));
        assert_eq!(vol.chain(&e).unwrap(), vec![18, 19, 20]);
        assert_eq!(vol.read(&e).unwrap(), data);
        assert_eq!(vol.find(&name("NEW.BIN")).unwrap(), e);
        let after = vol.into_inner();
        for (i, (a, b)) in original.iter().zip(after.iter()).enumerate() {
            if a == b {
                continue;
            }
            let fat_head = 0x200 + 18..=0x200 + 20;
            let fat_copy = 0x400 + 18..=0x400 + 20;
            let slot = 0x600 + 2 * 32..0x600 + 3 * 32;
            let clusters = (6 + 17) * 512..(6 + 20) * 512;
            assert!(fat_head.contains(&i) || fat_copy.contains(&i) || slot.contains(&i) || clusters.contains(&i), "byte {i:#06X} changed");
        }
        // FAT copy follows the FAT; last cluster zero-padded.
        assert_eq!(&after[0x400..0x400 + 123], &after[0x200..0x200 + 123]);
        assert_eq!(after[(6 + 19) * 512 + 76], 0);
    }

    #[test]
    fn exists_force_and_protect() {
        let mut vol = Volume::open_floppy_side(side("dw.floppy.yaml", Side::A)).unwrap();
        assert!(matches!(vol.write(&name("BIO.BAS"), b"x", TS, false), Err(DiskError::Exists(_))));
        let free = vol.free_clusters();
        let e = vol.write(&name("BIO.BAS"), b"x", TS, true).unwrap();
        assert_eq!((e.slot, e.size), (1, 1));
        assert_eq!(vol.free_clusters(), free + 5 - 1);
        assert_eq!(vol.list().len(), 2);

        // Write-protect it by hand.
        let mut buf = vol.into_inner();
        buf[0x600 + 32 + 0x0B] |= ATTR_PROTECTED;
        let mut vol = Volume::open_floppy_side(buf).unwrap();
        let e = vol.find(&name("BIO.BAS")).unwrap();
        assert_eq!(e.attr_letters(), "P--");
        assert!(matches!(vol.write(&name("BIO.BAS"), b"y", TS, false), Err(DiskError::Protected(_))));
        assert!(matches!(vol.delete(&e, false), Err(DiskError::Protected(_))));
        vol.delete(&e, true).unwrap();
        assert!(vol.find(&name("BIO.BAS")).is_none());
    }

    #[test]
    fn delete_frees_and_slot_is_reused() {
        let mut vol = Volume::open_floppy_side(side("dw.floppy.yaml", Side::A)).unwrap();
        let globus = vol.find(&name("GLOBUS.BAS")).unwrap();
        vol.delete(&globus, false).unwrap();
        assert_eq!(vol.list().len(), 1);
        assert_eq!(vol.free_clusters(), 122 - 5);
        let e = vol.write(&name("A.TXT"), b"hello\r\n\x1A", TS, false).unwrap();
        assert_eq!((e.slot, e.start_cluster), (0, 1));
        let names: Vec<String> = vol.list().iter().map(|e| e.name.to_string()).collect();
        assert_eq!(names, vec!["A.TXT", "BIO.BAS"]);
    }

    #[test]
    fn disk_full_and_directory_full_change_nothing() {
        let original = side("formatted.floppy.yaml", Side::A);
        let mut vol = Volume::open_floppy_side(original.clone()).unwrap();
        let err = vol.write(&name("BIG"), &vec![1u8; 62464 + 1], TS, false).unwrap_err();
        assert_eq!(err, DiskError::DiskFull { needed: 62465, free: 62464 });
        vol.write(&name("FITS"), &vec![1u8; 62464], TS, false).unwrap();
        assert_eq!(vol.free_bytes(), 0);

        let mut vol = Volume::open_floppy_side(original.clone()).unwrap();
        for i in 0..48 {
            vol.write(&name(&format!("F{i}")), b"", TS, false).unwrap();
        }
        let before = vol.into_inner();
        let mut vol = Volume::open_floppy_side(before.clone()).unwrap();
        assert_eq!(vol.write(&name("ONEMORE"), b"", TS, false), Err(DiskError::DirectoryFull));
        assert_eq!(vol.into_inner(), before);
    }

    #[test]
    fn corrupt_chains_are_reported() {
        let base = side("dw.floppy.yaml", Side::A);
        let cases: [(usize, u8, &str); 3] = [
            (0x200 + 5, 3, "loops"),        // 1→…→5→3
            (0x200 + 5, 0x7F, "out of range"),
            (0x200 + 5, 0x00, "free cluster"),
        ];
        for (at, v, want) in cases {
            let mut buf = base.clone();
            buf[at] = v;
            let vol = Volume::open_floppy_side(buf).unwrap();
            let e = vol.find(&name("GLOBUS.BAS")).unwrap();
            let err = vol.read(&e).unwrap_err().to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
        let mut buf = base.clone();
        buf[0x200 + 5] = 0xF0; // chain ends after 5 clusters, size needs 12
        let vol = Volume::open_floppy_side(buf).unwrap();
        let e = vol.find(&name("GLOBUS.BAS")).unwrap();
        assert!(vol.read(&e).unwrap_err().to_string().contains("5 clusters"));
    }

    #[test]
    fn glob_and_sizes() {
        let vol = Volume::open_floppy_side(side("dw.floppy.yaml", Side::A)).unwrap();
        assert_eq!(vol.glob(&Pattern::parse("*.BAS").unwrap()).len(), 2);
        assert_eq!(vol.glob(&Pattern::parse("G*").unwrap()).len(), 0); // no '.': no extension
        assert_eq!(vol.glob(&Pattern::parse("G*.*").unwrap()).len(), 1);
        assert_eq!(SIDE_SIZE, Geometry::CE1600F.volume_size());
    }
}
