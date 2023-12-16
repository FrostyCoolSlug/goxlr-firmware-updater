use crate::firmware::VersionNumber;
use crate::PageMessages::{
    UpdateFirmwareComplete, UpdateFirmwareIsError, UpdateFirmwareMessage, UpdateFirmwarePercent,
    UpdateFirmwareStage,
};
use crate::{DeviceType, FirmwareDetails, Message, PageMessages};
use goxlr_usb::device::base::FullGoXLRDevice;
use goxlr_usb::device::{find_devices, from_device};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::sleep;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) struct GoXLR {
    sender: UnboundedSender<Message>,
    handles: HashMap<DeviceLocal, Arc<Mutex<Box<dyn FullGoXLRDevice>>>>,
}

impl GoXLR {
    pub(crate) fn new(sender: UnboundedSender<Message>) -> Self {
        GoXLR {
            sender,
            handles: HashMap::new(),
        }
    }

    pub fn find_devices(&mut self) {
        let devices = find_devices();
        let mut device_list: Vec<Device> = Vec::new();

        // Create handles for all devices..
        for device in devices {
            let local_device = DeviceLocal {
                bus_number: device.bus_number(),
                address: device.address(),
                identifier: device.identifier().clone(),
            };

            // Do we need a new handle, or to use an existing one?
            let mut handle = if self.handles.contains_key(&local_device) {
                self.handles
                    .get_mut(&local_device.clone())
                    .unwrap()
                    .lock()
                    .unwrap()
            } else {
                // We don't care about messages being sent out at this point, we're explicitly
                // going to ignore them and handle errors on-the-fly during the update.
                let (disconnect_sender, _) = mpsc::channel(32);
                let (event_sender, _) = mpsc::channel(32);

                // Create the Handle, the pause is only needed if we're waiting for the startup animation to finish, in this
                // context, we don't care.
                let handle = from_device(device.clone(), disconnect_sender, event_sender, true);
                if let Err(error) = &handle {
                    println!("Error: {}", error);
                    continue;
                }

                // Unwrap the Handle, and stop polling for events.
                let mut handle = handle.unwrap();
                handle.stop_polling();

                self.handles
                    .insert(local_device.clone(), Arc::new(Mutex::new(handle)));
                self.handles
                    .get_mut(&local_device.clone())
                    .unwrap()
                    .lock()
                    .unwrap()
            };

            if let Ok(descriptor) = handle.get_descriptor() {
                let device_type = match descriptor.product_id() {
                    goxlr_usb::PID_GOXLR_FULL => DeviceType::Full,
                    goxlr_usb::PID_GOXLR_MINI => DeviceType::Mini,
                    _ => continue,
                };
                if let Ok((device_serial, _)) = handle.get_serial_number() {
                    if device_serial.is_empty() {
                        println!("Nope.");
                        continue;
                    }
                    if let Ok(firmware) = handle.get_firmware_version() {
                        let version = VersionNumber(
                            firmware.firmware.0,
                            firmware.firmware.1,
                            firmware.firmware.2,
                            firmware.firmware.3,
                        );

                        device_list.push(Device {
                            device_type,
                            device_serial,
                            version,
                            goxlr_device: local_device.clone(),
                        });
                    }
                }
            } else {
                println!("Nope!");
            }
        }
        println!("{:?}", device_list);

        let _ = self
            .sender
            .send(Message::PageMessage(PageMessages::UpdateDeviceList(
                device_list,
            )));
    }

    pub fn do_update(&mut self, device: Device, firmware: FirmwareDetails) {
        // Firstly, pull out the handle, and load the firmware..
        let handle = self.handles.get_mut(&device.goxlr_device);
        if handle.is_none() {
            self.send_setup_error("Unable to retrieve GoXLR from Device");
            return;
        }

        // Grab the Handle..
        let arc = handle.unwrap().clone();
        let mut handle = arc.lock().unwrap();

        // Grab the Firmware as a byte array..
        let firmware = if let Ok(firmware) = std::fs::read(firmware.path) {
            firmware
        } else {
            self.send_setup_error("Unable to Load Firmware from Disk");
            return;
        };
        let firmware_length = firmware.len() as u32;

        // Ok, got the device, got the firmware, lets goooooooo..
        if let Err(e) = handle.begin_firmware_upload() {
            let error = format!("Failed to put device in Update Mode: {}", e);
            self.send_setup_error(error.as_str());
            return;
        }

        if let Err(e) = self.clear_nvr(&mut handle) {
            println!("Error: {}", e);
            self.reboot_goxlr(&mut handle);
            return;
        }

        if let Err(e) = self.upload_firmware(firmware, &mut handle) {
            println!("Error: {}", e);
            self.reboot_goxlr(&mut handle);
            return;
        }

        if let Err(e) = self.validate_upload(firmware_length, &mut handle) {
            println!("Error: {}", e);
            self.reboot_goxlr(&mut handle);
            return;
        }

        if let Err(e) = self.hardware_verify(&mut handle) {
            println!("Error: {}", e);
            self.reboot_goxlr(&mut handle);
            return;
        }

        if let Err(e) = self.device_finalise(&mut handle) {
            println!("Error: {}", e);
            self.reboot_goxlr(&mut handle);
            return;
        }

        self.send_finish_complete();
        self.reboot_goxlr(&mut handle);
    }

    fn clear_nvr(
        &mut self,
        device: &mut MutexGuard<Box<dyn FullGoXLRDevice>>,
    ) -> Result<(), String> {
        self.send_stage_update("Preparing Update Partition");

        if let Err(error) = device.begin_erase_nvr() {
            let message = format!("Unable to start NVR Clear: {}", error);
            self.send_finish_error(message.as_str());
            return Err(message);
        }

        // Now we simply sit, wait, and update until we're done.
        let mut last_percent = 0_u8;
        let mut progress = 0;
        while progress != 255 {
            sleep(Duration::from_millis(100));
            progress = match device.poll_erase_nvr() {
                Ok(progress) => progress,
                Err(error) => {
                    let message = format!("Error Polling NVR Clear: {}", error);
                    self.send_finish_error(message.as_str());
                    return Err(message);
                }
            };

            let percent = ((progress as f32 / 255.) * 100.) as u8;
            if percent != last_percent {
                last_percent = percent;
                self.send_stage_percent(percent);
            }
        }

        self.send_stage_percent(100);
        Ok(())
    }

    fn upload_firmware(
        &mut self,
        firmware: Vec<u8>,
        device: &mut MutexGuard<Box<dyn FullGoXLRDevice>>,
    ) -> Result<(), String> {
        self.send_stage_update("Uploading Firmware to Device");
        let mut last_percent = 0_u8;

        let chunk_size = 1012;
        let mut sent = 0;

        for chunk in firmware.chunks(chunk_size) {
            if let Err(error) = device.send_firmware_packet(sent, chunk) {
                let message = format!("Error uploading Firmware Chunk: {}", error);
                self.send_finish_error(message.as_str());
                return Err(message);
            }

            sent += chunk.len() as u64;
            let percent = ((sent as f32 / firmware.len() as f32) * 100.) as u8;
            if percent != last_percent {
                last_percent = percent;
                self.send_stage_percent(percent);
            }
        }

        Ok(())
    }

    fn validate_upload(
        &mut self,
        firmware_len: u32,
        device: &mut MutexGuard<Box<dyn FullGoXLRDevice>>,
    ) -> Result<(), String> {
        self.send_stage_update("Verifying File Upload");
        let mut last_percent = 0_u8;

        let mut processed = 0_u32;
        let mut remaining_bytes = firmware_len;
        let mut hash_in = 0_u32;

        while remaining_bytes > 0 {
            let (hash, count) =
                match device.validate_firmware_packet(processed, hash_in, remaining_bytes) {
                    Ok((hash, count)) => (hash, count),
                    Err(error) => {
                        let message = format!("Error Validating Firmware Packet: {}", error);
                        self.send_finish_error(message.as_str());
                        return Err(message);
                    }
                };

            processed += count;
            if processed > firmware_len {
                let message = "Error Validating Firmware, Length Mismatch";
                self.send_finish_error(message);
                return Err(message.to_string());
            }

            remaining_bytes -= count;
            hash_in = hash;

            let percent = ((processed as f32 / firmware_len as f32) * 100.) as u8;
            if percent != last_percent {
                last_percent = percent;
                self.send_stage_percent(percent);
            }
        }

        Ok(())
    }

    fn hardware_verify(
        &mut self,
        device: &mut MutexGuard<Box<dyn FullGoXLRDevice>>,
    ) -> Result<(), String> {
        self.send_stage_update("Device Firmware Verification");
        let mut last_percent = 0_u8;

        if let Err(error) = device.verify_firmware_status() {
            let message = format!("Unable to Start Verification: {}", error);
            self.send_finish_error(message.as_str());
            return Err(message);
        }

        let mut complete = false;
        while !complete {
            let (completed, total, done) = match device.poll_verify_firmware_status() {
                Ok((completed, total, done)) => (completed, total, done),
                Err(error) => {
                    let message = format!("Device Validation Failed: {}", error);
                    self.send_finish_error(message.as_str());
                    return Err(message);
                }
            };

            complete = completed;

            let percent = ((done as f32 / total as f32) * 100.) as u8;
            if percent != last_percent {
                last_percent = percent;
                self.send_stage_percent(percent);
            }
        }
        Ok(())
    }

    // This is mostly copypasta from above, same behaviour, different calls.
    fn device_finalise(
        &mut self,
        device: &mut MutexGuard<Box<dyn FullGoXLRDevice>>,
    ) -> Result<(), String> {
        self.send_stage_update("Writing Firmware..");
        let mut last_percent = 0_u8;

        if let Err(error) = device.finalise_firmware_upload() {
            let message = format!("Unable to Start Write: {}", error);
            self.send_finish_error(message.as_str());
            return Err(message);
        }

        let mut complete = false;
        while !complete {
            let (completed, total, done) = match device.poll_finalise_firmware_upload() {
                Ok((completed, total, done)) => (completed, total, done),
                Err(error) => {
                    let message = format!("Progress Check Failed: {}", error);
                    self.send_finish_error(message.as_str());
                    return Err(message);
                }
            };

            complete = completed;

            let percent = ((done as f32 / total as f32) * 100.) as u8;
            if percent != last_percent {
                last_percent = percent;
                self.send_stage_percent(percent);
            }
        }
        Ok(())
    }

    fn reboot_goxlr(&mut self, device: &mut MutexGuard<Box<dyn FullGoXLRDevice>>) {
        let _ = device.reboot_after_firmware_upload();
    }

    fn send_stage_percent(&self, percent: u8) {
        let percent = UpdateFirmwarePercent(percent);
        let _ = self.sender.send(Message::PageMessage(percent));
    }

    fn send_stage_update(&self, stage: &str) {
        let stage = UpdateFirmwareStage(stage.to_string());
        let _ = self.sender.send(Message::PageMessage(stage));

        self.send_stage_percent(0);
    }

    fn send_setup_error(&self, message: &str) {
        let stage = UpdateFirmwareStage("Preparing...".to_string());
        let percent = UpdateFirmwarePercent(0);

        let _ = self.sender.send(Message::PageMessage(stage));
        let _ = self.sender.send(Message::PageMessage(percent));
        self.send_finish_error(message);
    }

    fn send_finish_complete(&self) {
        let message = "Your GoXLR Has updated Successfully!";
        let message = UpdateFirmwareMessage(message.to_string());

        let percent = UpdateFirmwarePercent(100);

        let _ = self.sender.send(Message::PageMessage(message));
        let _ = self.sender.send(Message::PageMessage(percent));
        self.send_finish();
    }

    fn send_finish_error(&self, message: &str) {
        let is_error = UpdateFirmwareIsError(true);

        let message = format!("Error: {}", message);
        let message = UpdateFirmwareMessage(message);

        let _ = self.sender.send(Message::PageMessage(message));
        let _ = self.sender.send(Message::PageMessage(is_error));

        self.send_finish();
    }

    fn send_finish(&self) {
        let complete = UpdateFirmwareComplete(true);
        let _ = self.sender.send(Message::PageMessage(complete));
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct DeviceLocal {
    pub(crate) bus_number: u8,
    pub(crate) address: u8,
    pub(crate) identifier: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Device {
    pub device_type: DeviceType,
    pub device_serial: String,
    pub version: VersionNumber,
    pub goxlr_device: DeviceLocal,
}
