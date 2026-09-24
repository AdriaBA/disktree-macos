//! Free space on the volume a path lives on.
//!
//! This is what makes "amount of crap I found" a real number rather than an
//! estimate: the app measures the volume before and after a removal, and shows
//! the projection while marking.

use std::io;
use std::path::Path;

/// A volume's capacity in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpaceInfo {
    /// Total size of the volume.
    pub total: u64,
    /// Free blocks, including the reserve only root may write to.
    pub free: u64,
    /// Free blocks this user may actually write; what `df -h` reports.
    pub available: u64,
}

impl SpaceInfo {
    /// Space in use, computed from `free` rather than `available` so the figure
    /// does not jump when a reserve is opened to root.
    pub const fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Share of the volume in use, `0.0..=1.0`.
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used() as f64 / self.total as f64) as f32
        }
    }

    /// Free space after `bytes` are removed, saturating at the volume size and
    /// never counting the same byte twice.
    #[must_use]
    pub fn after_removing(&self, bytes: u64) -> Self {
        let available = self.available.saturating_add(bytes).min(self.total);
        let free = self.free.saturating_add(bytes).min(self.total);
        Self {
            total: self.total,
            free,
            available,
        }
    }
}

/// Read the space on the volume containing `path`.
pub fn space_info(path: &Path) -> io::Result<SpaceInfo> {
    let stat = rustix::fs::statvfs(path)?;
    // `f_frsize` is the fragment size the block counts are expressed in;
    // `f_bsize` is only a hint for I/O. Some filesystems report zero for
    // `f_frsize`, so fall back rather than claiming a zero-sized volume.
    let block = if stat.f_frsize == 0 {
        stat.f_bsize
    } else {
        stat.f_frsize
    };
    Ok(SpaceInfo {
        total: stat.f_blocks.saturating_mul(block),
        free: stat.f_bfree.saturating_mul(block),
        available: stat.f_bavail.saturating_mul(block),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_volume_reports_plausible_numbers() {
        let temp = std::env::temp_dir();
        let space = space_info(&temp).expect("temp dir has a volume");
        assert!(space.total > 0, "{space:?}");
        assert!(space.free <= space.total, "{space:?}");
        assert!(space.available <= space.free, "{space:?}");
        assert!((0.0..=1.0).contains(&space.used_fraction()), "{space:?}");
    }

    #[test]
    fn a_missing_path_reports_the_io_error() {
        let error = space_info(Path::new("/definitely/not/here"))
            .expect_err("no volume");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn projecting_removal_cannot_exceed_the_volume() {
        let space = SpaceInfo {
            total: 1000,
            free: 100,
            available: 100,
        };
        let after = space.after_removing(50);
        assert_eq!(after.available, 150);
        assert_eq!(after.free, 150);
        assert_eq!(after.total, 1000);

        let capped = space.after_removing(10_000);
        assert_eq!(capped.available, 1000);
        assert_eq!(capped.used(), 0);
        assert!((capped.used_fraction() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn used_fraction_handles_an_empty_volume_report() {
        let space = SpaceInfo::default();
        assert!((space.used_fraction() - 0.0).abs() < f32::EPSILON);
    }
}
