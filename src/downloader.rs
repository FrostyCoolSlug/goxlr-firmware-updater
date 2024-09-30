use crate::{DeviceType, Message, PageMessages, StepMessages};
use reqwest::blocking::Client;
use reqwest::header::RANGE;
use reqwest::StatusCode;
use std::cmp::min;
use std::fs;
use std::fs::File;
use tokio::sync::mpsc::UnboundedSender;

static CHUNK_SIZE: u64 = 10240;

pub fn download_firmware(sender: UnboundedSender<Message>, device_type: DeviceType) {
    let full_name = "GoXLR_Firmware.bin";
    let mini_name = "GoXLR_MINI_Firmware.bin";

    let base_url = "https://mediadl.musictribe.com/media/PLM/sftp/incoming/hybris/import/GOXLR/";
    let url = match device_type {
        DeviceType::Full => format!("{}{}", base_url, full_name),
        DeviceType::Mini => format!("{}{}", base_url, mini_name),
        DeviceType::Unknown => return,
    };

    let output_path = std::env::temp_dir().join(match device_type {
        DeviceType::Full => full_name,
        DeviceType::Mini => mini_name,
        DeviceType::Unknown => "wont_happen",
    });

    if output_path.exists() && fs::remove_file(&output_path).is_err() {
        return;
    }

    let client = Client::new();

    // First, download the Manifest, and fetch the filename of the latest version..
    if let Ok(response) = client.head(&url).send() {
        println!("{:?}", response);
        if response.headers().contains_key("content-length") {
            let length = response
                .headers()
                .get("content-length")
                .and_then(|val| val.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());

            if let Some(length) = length {
                if length == 0 {
                    println!("Firmware Length of 0!");
                    println!("{url}");
                    return;
                }

                if let Ok(mut file) = File::create(&output_path) {
                    let mut current_percentage = 0;

                    let chunks = if (length % CHUNK_SIZE) != 0 {
                        length / CHUNK_SIZE + 1
                    } else {
                        length / CHUNK_SIZE
                    };

                    for i in 0..chunks {
                        let start = CHUNK_SIZE * i;
                        let end = min(((CHUNK_SIZE * i) + CHUNK_SIZE) - 1, length);

                        if start == end {
                            break;
                        }

                        let header = format!("bytes={}-{}", start, end);
                        println!("{:?}", header);

                        if let Ok(mut response) = client.get(&url).header(RANGE, header).send() {
                            let status = response.status();
                            if !(status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT)
                            {
                                return;
                            }

                            if std::io::copy(&mut response, &mut file).is_err() {
                                return;
                            }
                            let percentage = ((end as f32 / length as f32) * 100.) as u8;
                            if percentage != current_percentage {
                                current_percentage = percentage;
                                let message = Message::PageMessage(
                                    PageMessages::DownloadFirmwarePercent(percentage),
                                );
                                let _ = sender.send(message);
                            }
                        }
                    }
                }
            }
        }
    } else {
        return;
    }

    // Ok, now we send a file..
    let message = Message::StepsMessage(StepMessages::SelectFile(Some(output_path)));
    let _ = sender.send(message);
}
