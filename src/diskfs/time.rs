//! MS-DOS-style packed directory timestamps.
//!
//! time = `hour<<11 | minute<<5 | second/2`, date = `year<<9 | month<<5 | day`, both
//! little-endian words. The PC-1600 clock has no year; the ROM always writes 6 in the
//! year field (see [`NEW_FILE_YEAR_FIELD`]).

/// Year field of a new file: the PC-1600 ROM writes 6 whatever the date (checked in
/// Calc-U-1600 with `DATE$ = "09/22"`, and on `dw.img`).
pub const NEW_FILE_YEAR_FIELD: u8 = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DosTimestamp {
    pub year_field: u8,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DosTimestamp {
    /// A timestamp for a new file, with the ROM's year field.
    pub fn new(month: u8, day: u8, hour: u8, minute: u8, second: u8) -> DosTimestamp {
        DosTimestamp { year_field: NEW_FILE_YEAR_FIELD, month, day, hour, minute, second }
    }

    /// `(time, date)` words.
    pub fn pack(self) -> (u16, u16) {
        let time = (u16::from(self.hour & 0x1F) << 11)
            | (u16::from(self.minute & 0x3F) << 5)
            | u16::from((self.second / 2) & 0x1F);
        let date = (u16::from(self.year_field & 0x7F) << 9)
            | (u16::from(self.month & 0x0F) << 5)
            | u16::from(self.day & 0x1F);
        (time, date)
    }

    /// The current UTC time (for embedders without a local clock of their own).
    pub fn now_utc() -> DosTimestamp {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (_, month, day) = crate::floppy_image::civil_from_days(secs.div_euclid(86_400));
        let rem = secs.rem_euclid(86_400);
        DosTimestamp::new(month as u8, day as u8, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8)
    }

    pub fn unpack(time: u16, date: u16) -> DosTimestamp {
        DosTimestamp {
            year_field: (date >> 9) as u8,
            month: ((date >> 5) & 0x0F) as u8,
            day: (date & 0x1F) as u8,
            hour: (time >> 11) as u8,
            minute: ((time >> 5) & 0x3F) as u8,
            second: ((time & 0x1F) * 2) as u8,
        }
    }
}

impl std::fmt::Display for DosTimestamp {
    /// `MM-DD HH:MM:SS` (no year: the PC-1600 clock has none).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}-{:02} {:02}:{:02}:{:02}", self.month, self.day, self.hour, self.minute, self.second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dw_img_entry() {
        // GLOBUS.BAS on dw.img: time 0x0049, date 0x0C21.
        let ts = DosTimestamp::unpack(0x0049, 0x0C21);
        assert_eq!(ts, DosTimestamp { year_field: 6, month: 1, day: 1, hour: 0, minute: 2, second: 18 });
        assert_eq!(ts.pack(), (0x0049, 0x0C21));
        assert_eq!(ts.to_string(), "01-01 00:02:18");
    }

    #[test]
    fn roundtrip() {
        let ts = DosTimestamp::new(12, 31, 23, 59, 58);
        assert_eq!(DosTimestamp::unpack(ts.pack().0, ts.pack().1), ts);
    }
}
