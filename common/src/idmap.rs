use std::io;
use std::os::fd::AsRawFd;
use std::ptr;

use anyhow::bail;
use memmap2::{Mmap, MmapMut};

pub const UID_MIN: u32 = 10000;
pub const UID_MAX: u32 = 20000;
pub const IDMAP_SIZE: u64 = ((UID_MAX - UID_MIN) as u64).div_ceil(8);

fn uid_to_index(uid: u32) -> Option<usize> {
    (UID_MIN..UID_MAX)
        .contains(&uid)
        .then(|| (uid - UID_MIN) as usize)
}

/// Read-only idmap backed by a shared memory mapping.
///
/// Uses `ptr::read_volatile` for reads to prevent the compiler from
/// optimizing away repeated loads — the underlying file may be modified
/// by another process (the daemon) at any time.
pub struct IdmapReader {
    mmap: Mmap,
}

impl IdmapReader {
    /// # Safety
    ///
    /// The file descriptor must refer to a valid idmap file.
    pub unsafe fn from_fd(fd: &impl AsRawFd) -> io::Result<Self> {
        let mmap = unsafe { Mmap::map(fd.as_raw_fd())? };
        Ok(Self { mmap })
    }

    /// Returns the access bit for `uid`, or `None` if `uid` is out of
    /// the valid range (`UID_MIN..UID_MAX`).
    pub fn get(&self, uid: u32) -> Option<bool> {
        let index = uid_to_index(uid)?;
        let byte = unsafe { ptr::read_volatile(self.mmap.as_ptr().add(index >> 3)) };
        Some(byte & (1 << (index & 7)) != 0)
    }
}

/// Read-write idmap backed by a mutable memory mapping.
pub struct IdmapWriter {
    mmap: MmapMut,
}

impl IdmapWriter {
    /// # Safety
    ///
    /// The file descriptor must refer to a valid idmap file opened for
    /// reading and writing.
    pub unsafe fn from_fd(fd: &impl AsRawFd) -> io::Result<Self> {
        let mmap = unsafe { MmapMut::map_mut(fd.as_raw_fd())? };
        Ok(Self { mmap })
    }

    /// Returns the access bit for `uid`, or `None` if `uid` is out of
    /// the valid range (`UID_MIN..UID_MAX`).
    pub fn get(&self, uid: u32) -> Option<bool> {
        let index = uid_to_index(uid)?;
        let byte = self.mmap[index >> 3];
        Some(byte & (1 << (index & 7)) != 0)
    }

    /// Sets the access bit for `uid` and flushes the mapping.
    pub fn set(&mut self, uid: u32, value: bool) -> anyhow::Result<()> {
        let Some(index) = uid_to_index(uid) else {
            bail!("uid {uid} out of range ({UID_MIN}..{UID_MAX})");
        };

        let byte_index = index >> 3;
        let bit_mask = 1u8 << (index & 7);

        if value {
            self.mmap[byte_index] |= bit_mask;
        } else {
            self.mmap[byte_index] &= !bit_mask;
        }

        self.mmap.flush()?;
        Ok(())
    }

    /// Returns all UIDs that currently have their access bit set.
    pub fn get_all(&self) -> Vec<u32> {
        let mut result = Vec::new();

        for (byte_idx, &byte) in self.mmap.iter().enumerate() {
            if byte == 0 {
                continue;
            }

            for bit in 0..8u8 {
                let index = byte_idx * 8 + bit as usize;

                if index >= (UID_MAX - UID_MIN) as usize {
                    break;
                }

                if byte & (1 << bit) != 0 {
                    result.push(index as u32 + UID_MIN);
                }
            }
        }

        result
    }

    /// Clears all access bits and flushes the mapping.
    pub fn clear(&mut self) -> anyhow::Result<()> {
        self.mmap.fill(0);
        self.mmap.flush()?;
        Ok(())
    }
}
