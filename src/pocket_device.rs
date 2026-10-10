//! The four `get`/`put` transport targets.
//!
//! This is a strict superset of [`crate::registry::Device`]: the pure tokenizer/header
//! core only needs to know "CE-158 flavor" vs. "PC-1600 flavor" (`registry::Device`),
//! but the serial layer additionally needs to know real hardware vs. the pseudo-terminal
//! emulator, since that changes baud/flow-control/pacing even when the header flavor is
//! identical. Kept out of `registry`/`header` so the C ABI (`ffi.rs`) never has to know
//! about serial transport at all.

use crate::registry::Device;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PocketDevice {
    Pc1500,
    Pc1500a,
    Pc1600,
    Pc1600Emul,
}

impl PocketDevice {
    pub fn is_pc1500_family(self) -> bool {
        matches!(self, Self::Pc1500 | Self::Pc1500a)
    }

    pub fn is_pc1600_family(self) -> bool {
        matches!(self, Self::Pc1600 | Self::Pc1600Emul)
    }

    /// True for the pseudo-terminal emulator link, which has no modem-control lines.
    pub fn is_emulator(self) -> bool {
        self == Self::Pc1600Emul
    }

    /// Reject `--no-flowcontrol` for every device but a real PC-1600: the others never
    /// use RTS/CTS (the CE-158 has no handshake lines, the emulator's pseudo-terminal has
    /// none either), so there is nothing to turn off.
    pub fn check_no_flow_control(self, no_flow_control: bool) -> anyhow::Result<()> {
        if no_flow_control && self != Self::Pc1600 {
            anyhow::bail!("--no-flowcontrol only applies to --device pc1600");
        }
        Ok(())
    }

    /// True when the OS-level serial port is opened with RTS/CTS handshaking: a real
    /// PC-1600 unless `--no-flowcontrol` was given.
    pub fn uses_hardware_flow_control(self, no_flow_control: bool) -> bool {
        self == Self::Pc1600 && !no_flow_control
    }

    /// True when the sender must pace bytes itself because nothing reliably throttles
    /// it: the PC-1500 family (CE-158 has no handshake), the emulator (pseudo-terminal
    /// has no handshake lines), and a real PC-1600 with `--no-flowcontrol`.
    pub fn is_paced_send(self, no_flow_control: bool) -> bool {
        !self.uses_hardware_flow_control(no_flow_control)
    }

    pub fn baud_rate(self) -> u32 {
        if self.is_pc1500_family() {
            19200
        } else {
            9600
        }
    }

    /// Idle-timeout fallback for `get`'s end-of-transfer watchdog (only used when the
    /// transfer length can't be determined from a header).
    pub fn idle_timeout_ms(self) -> u64 {
        if self.is_pc1500_family() {
            5000
        } else {
            500
        }
    }

    /// Maps down to the 2-variant device used by the pure tokenizer/header core.
    pub fn to_registry_device(self) -> Device {
        if self.is_pc1500_family() {
            Device::Pc1500
        } else {
            Device::Pc1600
        }
    }

    /// Hex-digit width for verbose address reporting: 4 for PC-1500 family (16-bit
    /// addresses), 6 for PC-1600 (24-bit addresses).
    pub fn addr_hex_width(self) -> usize {
        self.to_registry_device().addr_hex_width()
    }

    /// The CLI's spelling of this device, for narration/error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pc1500 => "pc1500",
            Self::Pc1500a => "pc1500a",
            Self::Pc1600 => "pc1600",
            Self::Pc1600Emul => "pc1600emul",
        }
    }
}

impl std::fmt::Display for PocketDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc1500_family_facts() {
        for d in [PocketDevice::Pc1500, PocketDevice::Pc1500a] {
            assert!(d.is_pc1500_family());
            assert!(!d.is_pc1600_family());
            assert!(!d.is_emulator());
            for no_fc in [false, true] {
                assert!(!d.uses_hardware_flow_control(no_fc));
                assert!(d.is_paced_send(no_fc));
            }
            assert!(d.check_no_flow_control(true).is_err());
            assert_eq!(d.baud_rate(), 19200);
            assert_eq!(d.idle_timeout_ms(), 5000);
            assert_eq!(d.to_registry_device(), Device::Pc1500);
            assert_eq!(d.addr_hex_width(), 4);
        }
    }

    #[test]
    fn pc1600_hardware_facts() {
        let d = PocketDevice::Pc1600;
        assert!(d.is_pc1600_family());
        assert!(!d.is_emulator());
        assert!(d.uses_hardware_flow_control(false));
        assert!(!d.is_paced_send(false));
        assert!(!d.uses_hardware_flow_control(true));
        assert!(d.is_paced_send(true));
        assert!(d.check_no_flow_control(true).is_ok());
        assert_eq!(d.baud_rate(), 9600);
        assert_eq!(d.idle_timeout_ms(), 500);
        assert_eq!(d.to_registry_device(), Device::Pc1600);
        assert_eq!(d.addr_hex_width(), 6);
    }

    #[test]
    fn pc1600_emulator_facts() {
        let d = PocketDevice::Pc1600Emul;
        assert!(d.is_pc1600_family());
        assert!(d.is_emulator());
        assert!(!d.uses_hardware_flow_control(false));
        assert!(d.is_paced_send(false));
        assert!(d.check_no_flow_control(true).is_err());
        assert_eq!(d.baud_rate(), 9600);
        assert_eq!(d.idle_timeout_ms(), 500);
        assert_eq!(d.to_registry_device(), Device::Pc1600);
        assert_eq!(d.addr_hex_width(), 6);
    }

    #[test]
    fn display_matches_cli_spelling() {
        assert_eq!(PocketDevice::Pc1500.to_string(), "pc1500");
        assert_eq!(PocketDevice::Pc1500a.to_string(), "pc1500a");
        assert_eq!(PocketDevice::Pc1600.to_string(), "pc1600");
        assert_eq!(PocketDevice::Pc1600Emul.to_string(), "pc1600emul");
    }
}
