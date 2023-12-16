use crate::DeviceType;
use byteorder::{LittleEndian, ReadBytesExt};
use std::fmt::Formatter;
use std::io;
use std::io::Cursor;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FirmwareInfo {
    pub device: DeviceType,
    pub version: VersionNumber,
}

pub fn check_firmware(path: PathBuf) -> Result<FirmwareInfo, String> {
    load_firmware_file(&path)
}

fn load_firmware_file(file: &PathBuf) -> Result<FirmwareInfo, String> {
    if let Ok(firmware) = std::fs::read(file) {
        // I'm going to assume that if the firmware is < 64 bytes, it doesn't contain the
        // full firmware header.
        if firmware.len() < 64 {
            return Err(String::from("Invalid GoXLR Firmware File"));
        }

        // Is this a Mini, or a full?
        let device_name = get_firmware_name(&firmware[0..16]);
        let device_type = if device_name == "GoXLR Firmware" {
            DeviceType::Full
        } else if device_name == "GoXLR-Mini" {
            DeviceType::Mini
        } else {
            return Err(String::from("Unknown Device in Firmware Headers"));
        };

        // Next, grab the version for this firmware..
        let device_version = if let Ok(version) = get_firmware_version(&firmware[24..32]) {
            version
        } else {
            return Err(String::from("Unable to extract firmware version"));
        };

        Ok(FirmwareInfo {
            device: device_type,
            version: device_version,
        })
    } else {
        Err(String::from("Unable to open file"))
    }
}

fn get_firmware_name(src: &[u8]) -> String {
    let mut end_index = 0;
    for byte in src {
        if *byte == 0x00 {
            break;
        }
        end_index += 1;
    }
    return String::from_utf8_lossy(&src[0..end_index]).to_string();
}

fn get_firmware_version(src: &[u8]) -> Result<VersionNumber, io::Error> {
    println!("{}", src.len());
    println!("{:x?}", src);

    // Unpack the firmware version..
    let mut cursor = Cursor::new(src);
    let firmware_packed = cursor.read_u32::<LittleEndian>()?;
    let firmware_build = cursor.read_u32::<LittleEndian>()?;
    let firmware = VersionNumber(
        firmware_packed >> 12,
        (firmware_packed >> 8) & 0xF,
        firmware_packed & 0xFF,
        firmware_build,
    );

    Ok(firmware)
}

// Tentatively Stolen :D
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VersionNumber(pub u32, pub u32, pub u32, pub u32);

impl std::fmt::Display for VersionNumber {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0, self.1, self.2, self.3)
    }
}

impl std::fmt::Debug for VersionNumber {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0, self.1, self.2, self.3)
    }
}
