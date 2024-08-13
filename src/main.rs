mod downloader;
mod firmware;
mod goxlr;
mod preflight;

use crate::downloader::download_firmware;
use crate::firmware::VersionNumber;
use crate::goxlr::{Device, GoXLR};
use crate::preflight::status_check;
use iced::widget::{
    button, checkbox, column, container, horizontal_space, progress_bar, radio, row, scrollable,
    text, Rule, Space,
};
use iced::{
    executor, window, Application, Command, Element, Length, Padding, Renderer, Settings,
    Subscription, Theme,
};
use rfd::FileDialog;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

const LICENSE: &str = include_str!("../LICENSE");
const LICENSE_3RD_PARTY: &str = include_str!("../LICENSE-3RD-PARTY");

fn main() -> iced::Result {
    Pages::run(Settings {
        window: window::Settings {
            size: (500, 370),
            visible: true,
            resizable: false,
            ..Default::default()
        },
        ..Default::default()
    })
}

pub struct Pages {
    receiver: RefCell<Option<UnboundedReceiver<Message>>>,
    steps: Steps,
}

impl Pages {}

impl Application for Pages {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Self::Message>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let goxlr = Arc::new(Mutex::new(GoXLR::new(sender.clone())));
        (
            Pages {
                receiver: RefCell::new(Some(receiver)),
                steps: Steps::new(sender, goxlr),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "GoXLR Firmware Updater".to_string()
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::NextPressed => {
                self.steps.advance();
            }
            Message::StepsMessage(msg) => {
                self.steps.update_steps(msg);
            }
            Message::PageMessage(step_msg) => {
                self.steps.update(step_msg);
            }
        }
        Command::none()
    }

    fn view(self: &Pages) -> Element<'_, Self::Message, Renderer<Self::Theme>> {
        let Pages { steps, .. } = self;

        let mut controls = row![];
        controls = controls.push(horizontal_space(Length::Fill));
        controls = controls.push(button("Next").on_press(Message::NextPressed));

        let header = steps.header_text().map(Message::PageMessage);
        let ruler = Rule::horizontal(5);
        let body = steps.view().map(Message::PageMessage);
        let blank = Space::new(Length::Fill, 50);
        let ruler2 = Rule::horizontal(5);
        let controls = container(controls)
            .padding(Padding {
                top: 5.0,
                right: 10.0,
                bottom: 5.0,
                left: 0.0,
            })
            .height(45);

        let content: Element<_> = if self.steps.can_continue() {
            column![header, ruler, body, ruler2, controls].into()
        } else {
            column![header, ruler, body, blank].into()
        };

        container(content).width(Length::Fill).height(370).into()
    }

    fn theme(&self) -> Self::Theme {
        Theme::Dark
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        iced::subscription::unfold(
            "External Message",
            self.receiver.take(),
            move |mut receiver| async move {
                let message = receiver.as_mut().unwrap().recv().await.unwrap();
                (message, receiver)
            },
        )
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    NextPressed,
    StepsMessage(StepMessages),
    PageMessage(PageMessages),
}

#[derive(Debug, Clone)]
pub enum StepMessages {
    SetDevice(Device),
    SelectFile(Option<PathBuf>),
    SetUpdateType(SelectUpdateOption),
    SetFirmware(FirmwareDetails),
    ClearFirmware(),
}

struct Steps {
    steps: Vec<Step>,
    current: usize,
}

impl Steps {
    fn new(sender: UnboundedSender<Message>, goxlr: Arc<Mutex<GoXLR>>) -> Steps {
        Steps {
            steps: vec![
                Step::Welcome,
                Step::LicenseOne { agreed: false },
                Step::LicenseTwo { agreed: false },
                Step::Status {
                    sender: sender.clone(),
                    app: false,
                    beta: false,
                    util: false,
                },
                Step::LocateGoXLR {
                    goxlr: goxlr.clone(),
                    sender: sender.clone(),
                    devices: None,
                    selected: None,
                },
                Step::UpdateMethod {
                    sender: sender.clone(),
                    selected: None,
                },
                Step::SelectFile {
                    sender,
                    file_valid: false,
                    file: None,
                    progress: 0,
                    downgrade: false,
                    device: None,
                    fetch_method: None,
                    details: None,
                },
                Step::RunUpdate {
                    goxlr,

                    device: None,
                    firmware: None,

                    stage: "Starting".to_string(),
                    percentage: 0,
                    message: None,

                    // Final States..
                    complete: false,
                    is_error: false,
                },
                Step::Finish,
            ],
            current: 0,
        }
    }

    fn update_steps(&mut self, msg: StepMessages) {
        match msg {
            StepMessages::SetDevice(selected_device) => {
                // Locate the SelectFile step in the vec..
                for step in &mut self.steps {
                    if let Step::SelectFile { device, .. } = step {
                        device.replace(selected_device.clone());
                    }
                    if let Step::RunUpdate { device, .. } = step {
                        device.replace(selected_device.clone());
                    }
                }
            }
            StepMessages::SetUpdateType(update_type) => {
                for step in &mut self.steps {
                    if let Step::SelectFile { fetch_method, .. } = step {
                        fetch_method.replace(update_type);
                    }
                }
            }
            StepMessages::SelectFile(selected) => {
                for step in &mut self.steps {
                    if let Step::SelectFile {
                        file,
                        downgrade,
                        details,
                        ..
                    } = step
                    {
                        if let Some(path) = &selected {
                            file.replace(path.clone());

                            // Untick the box.
                            *downgrade = false;
                            if let Ok(firmware) = firmware::check_firmware(path.clone()) {
                                details.replace(FirmwareDetails {
                                    path: path.clone(),
                                    device_type: firmware.device,
                                    version: firmware.version,
                                });
                            } else {
                                *details = None;
                            }

                            file.replace(path.clone());
                        }
                    }
                }
            }
            StepMessages::SetFirmware(details) => {
                for step in &mut self.steps {
                    if let Step::RunUpdate { firmware, .. } = step {
                        firmware.replace(details.clone());
                    }
                }
            }
            StepMessages::ClearFirmware() => {
                for step in &mut self.steps {
                    if let Step::RunUpdate { firmware, .. } = step {
                        *firmware = None;
                    }
                }
            }
        }
    }

    fn update(&mut self, msg: PageMessages) {
        self.steps[self.current].update(msg);
    }

    fn view(&self) -> Element<PageMessages> {
        container(self.steps[self.current].view())
            .width(500)
            .height(Length::Fill)
            .padding(Padding {
                top: 10.0,
                right: 10.0,
                bottom: 10.0,
                left: 10.0,
            })
            .into()
    }

    fn header_text(&self) -> Element<PageMessages> {
        container(self.steps[self.current].header_text())
            .width(500)
            .height(60)
            .padding(Padding {
                top: 10.0,
                right: 0.0,
                bottom: 0.0,
                left: 15.0,
            })
            .into()
    }

    fn advance(&mut self) {
        if self.can_continue() {
            self.current += 1;
            self.steps[self.current].pre_display();
        }
    }

    fn can_continue(&self) -> bool {
        self.current + 1 < self.steps.len() && self.steps[self.current].can_continue()
    }
}

#[derive(Debug, Clone)]
pub struct FirmwareDetails {
    path: PathBuf,
    device_type: DeviceType,
    version: VersionNumber,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DeviceType {
    Full,
    Mini,
    Unknown,
}

enum Step {
    Welcome,
    LicenseOne {
        agreed: bool,
    },
    LicenseTwo {
        agreed: bool,
    },
    Status {
        sender: UnboundedSender<Message>,
        app: bool,
        beta: bool,
        util: bool,
    },
    LocateGoXLR {
        sender: UnboundedSender<Message>,
        goxlr: Arc<Mutex<GoXLR>>,
        devices: Option<Vec<Device>>,
        selected: Option<usize>,
    },
    UpdateMethod {
        sender: UnboundedSender<Message>,
        selected: Option<SelectUpdateOption>,
    },
    SelectFile {
        sender: UnboundedSender<Message>,
        file_valid: bool,
        device: Option<Device>,
        fetch_method: Option<SelectUpdateOption>,
        progress: u8,
        file: Option<PathBuf>,
        details: Option<FirmwareDetails>,
        downgrade: bool,
    },
    RunUpdate {
        goxlr: Arc<Mutex<GoXLR>>,

        // Ok, we need the device and firmware details..
        device: Option<Device>,
        firmware: Option<FirmwareDetails>,

        // State Tracking..
        stage: String,
        percentage: u8,
        message: Option<String>,

        // We're done.
        complete: bool,
        is_error: bool,
    },
    Finish,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SelectUpdateOption {
    Download,
    File,
}

#[derive(Debug, Clone)]
pub enum PageMessages {
    NoneBool(bool),

    ToggleAcceptLicenseOne(bool),
    ToggleAcceptLicenseTwo(bool),
    UpdateStatusCheck(bool, bool, bool),
    UpdateDeviceList(Vec<Device>),
    SelectFirmwareOption(SelectUpdateOption),
    SelectDevice(usize),

    SelectFirmware,
    DownloadFirmwarePercent(u8),

    SetAcceptDowngrade(bool),
    SetFirmwareValid(bool),

    // Actual Firmware Details
    UpdateFirmwareStage(String),
    UpdateFirmwarePercent(u8),
    UpdateFirmwareMessage(String),
    UpdateFirmwareComplete(bool),
    UpdateFirmwareIsError(bool),
}

impl<'a> Step {
    fn pre_display(&mut self) {
        if let Step::Status { sender, .. } = self {
            // Spawn the thread that monitors to make sure everything is shut down..
            let sender = sender.clone();
            thread::spawn(move || status_check(sender));
        }

        if let Step::LocateGoXLR { goxlr, .. } = self {
            let clone = goxlr.clone();
            thread::spawn(move || clone.lock().unwrap().find_devices());
        }

        if let Step::SelectFile {
            sender,
            device,
            fetch_method: Some(method),
            ..
        } = self
        {
            if method == &SelectUpdateOption::Download {
                if let Some(device) = device {
                    let sender = sender.clone();
                    let device_type = device.device_type;
                    thread::spawn(move || download_firmware(sender, device_type));
                }
            }
        }

        if let Step::RunUpdate {
            goxlr,
            device,
            firmware,
            ..
        } = self
        {
            println!(
                "Starting Firmware Update for: {:?}, with {:?}",
                device, firmware
            );

            // Grab a useful reference to our GoXLR object..
            let g = goxlr.clone();

            // Ok, we know at this point that these values have been definitively set, it's not
            // possible to progress out of the previous steps without triggering the code that sets
            // them, so we're safe to flat .unwrap here.
            let d = device.as_ref().unwrap().clone();
            let f = firmware.as_ref().unwrap().clone();

            // Spawn the update thread, and hope for the best :D
            thread::spawn(move || g.lock().unwrap().do_update(d, f));
        }
    }

    fn update(&mut self, msg: PageMessages) {
        match msg {
            PageMessages::NoneBool(_) => {}
            PageMessages::ToggleAcceptLicenseOne(value) => {
                if let Step::LicenseOne { agreed } = self {
                    *agreed = value;
                }
            }
            PageMessages::ToggleAcceptLicenseTwo(value) => {
                if let Step::LicenseTwo { agreed } = self {
                    *agreed = value
                }
            }
            PageMessages::UpdateStatusCheck(app_running, beta_running, util_running) => {
                if let Step::Status {
                    app, beta, util, ..
                } = self
                {
                    *app = !app_running;
                    *beta = !beta_running;
                    *util = !util_running;
                }
            }
            PageMessages::UpdateDeviceList(list) => {
                if let Step::LocateGoXLR { devices, .. } = self {
                    *devices = Some(list);
                }
            }
            PageMessages::SelectDevice(device) => {
                if let Step::LocateGoXLR {
                    sender,
                    selected,
                    devices,
                    ..
                } = self
                {
                    *selected = Some(device);

                    // Send the selection upstream for the next page.
                    if let Some(devices) = devices {
                        let _ = sender.send(Message::StepsMessage(StepMessages::SetDevice(
                            devices[device].clone(),
                        )));
                    }
                }
            }
            PageMessages::SelectFirmwareOption(method) => {
                if let Step::UpdateMethod { sender, selected } = self {
                    *selected = Some(method);
                    let _ = sender.send(Message::StepsMessage(StepMessages::SetUpdateType(method)));
                }
            }

            PageMessages::DownloadFirmwarePercent(percent) => {
                if let Step::SelectFile { progress, .. } = self {
                    *progress = percent
                }
            }

            PageMessages::SelectFirmware => {
                if let Step::SelectFile { sender, .. } = self {
                    if let Some(file_selected) = FileDialog::new()
                        .add_filter("GoXLR Firmware", &["bin"])
                        .set_directory("/")
                        .pick_file()
                    {
                        let _ = sender.send(Message::StepsMessage(StepMessages::SelectFile(Some(
                            file_selected,
                        ))));
                    }
                }
            }

            PageMessages::SetAcceptDowngrade(value) => {
                if let Step::SelectFile { downgrade, .. } = self {
                    *downgrade = value
                }
            }
            PageMessages::SetFirmwareValid(value) => {
                if let Step::SelectFile { file_valid, .. } = self {
                    *file_valid = value;
                }
            }
            PageMessages::UpdateFirmwareStage(value) => {
                if let Step::RunUpdate { stage, .. } = self {
                    *stage = value;
                }
            }
            PageMessages::UpdateFirmwarePercent(value) => {
                if let Step::RunUpdate { percentage, .. } = self {
                    *percentage = value;
                }
            }
            PageMessages::UpdateFirmwareMessage(value) => {
                if let Step::RunUpdate { message, .. } = self {
                    *message = Some(value);
                }
            }
            PageMessages::UpdateFirmwareComplete(value) => {
                if let Step::RunUpdate { complete, .. } = self {
                    *complete = value;
                }
            }
            PageMessages::UpdateFirmwareIsError(value) => {
                if let Step::RunUpdate { is_error, .. } = self {
                    *is_error = value;
                }
            }
        }
    }

    fn title(&self) -> &str {
        match self {
            Step::Welcome => "Welcome",
            Step::LicenseOne { .. } => "MIT License Agreement",
            Step::LicenseTwo { .. } => "TC-Helicon License Agreement",
            Step::Status { .. } => "Checking Environment",
            Step::LocateGoXLR { .. } => "Locating GoXLRs",
            Step::UpdateMethod { .. } => "Select Update Method",
            Step::SelectFile { fetch_method: Some(method), file, .. } => match method {
                SelectUpdateOption::Download => match file {
                    None => "Downloading Firmware",
                    Some(_) => "Download Complete",
                },
                SelectUpdateOption::File => "Select Firmware File",
            },
            Step::SelectFile { .. } => "Select Firmware File",
            Step::RunUpdate { .. } => "Updating..",
            Step::Finish => "Finished.",
        }
    }

    fn description(&self) -> &str {
        match self {
            Step::Welcome => "Welcome to the GoXLR Firmware Updater",
            Step::LicenseOne { .. } => {
                "Please review the license terms before updating your firmware"
            }
            Step::LicenseTwo { .. } => {
                "Please review the license terms before updating your firmware"
            }
            Step::Status { .. } => "Please ensure all GoXLR apps are closed before continuing",
            Step::LocateGoXLR { .. } => "Please select a GoXLR from the list below",
            Step::UpdateMethod { .. } => "Please Select the update method",
            Step::SelectFile { fetch_method: Some(method), file, .. } => match method {
                SelectUpdateOption::Download => match file {
                    None => "Please wait while the firmware downloads from TC-Helicon's servers",
                    Some(_) => "Please continue when ready"
                }
                SelectUpdateOption::File => "Please select the correct firmware file for your GoXLR"
            }
            Step::SelectFile { .. } => "Please select the correct firmware file for your GoXLR",
            Step::RunUpdate { .. } => "Firmware updating, do not power off your GoXLR or computer",
            Step::Finish => "Update has been completed",
        }
    }

    fn can_continue(&self) -> bool {
        match self {
            Step::Welcome => true,
            Step::LicenseOne { agreed } => *agreed,
            Step::LicenseTwo { agreed } => *agreed,
            Step::Status {
                app, beta, util, ..
            } => *app && *beta && *util,
            Step::LocateGoXLR { selected, .. } => selected.is_some(),
            Step::UpdateMethod { .. } => true,
            Step::SelectFile { file_valid, .. } => *file_valid,
            Step::RunUpdate { complete, .. } => *complete,
            Step::Finish => false,
        }
    }

    fn view(&self) -> Element<PageMessages> {
        match self {
            Step::Welcome => self.welcome(),
            Step::LicenseOne { agreed } => self.license(*agreed, true),
            Step::LicenseTwo { agreed } => self.license(*agreed, false),
            Step::Status {
                app,
                beta,
                util,
                sender,
            } => self.status(*app, *beta, *util, sender.clone()),
            Step::LocateGoXLR {
                devices,
                selected,
                sender,
                ..
            } => self.find_goxlr(*selected, devices, sender.clone()),
            Step::UpdateMethod { selected, .. } => self.select_choice(*selected),
            Step::SelectFile {
                sender,
                fetch_method,
                details,
                device,
                progress,
                file,
                downgrade,
                ..
            } => self.select_file(
                sender.clone(),
                fetch_method,
                details,
                device,
                file,
                *progress,
                downgrade,
            ),
            Step::RunUpdate {
                stage,
                percentage,
                message,
                complete,
                is_error,
                ..
            } => self.run_update(stage, *percentage, message.clone(), *complete, *is_error),
            Step::Finish => self.welcome(),
        }
    }

    fn header_text(&self) -> Element<PageMessages> {
        column![
            text(self.header_first()).size(18),
            text(format!("    {}", self.header_second())).size(14)
        ]
        .into()
    }

    fn header_first(&self) -> &str {
        self.title()
    }

    fn header_second(&self) -> &str {
        self.description()
    }

    fn welcome(&self) -> Element<'a, PageMessages> {
        let message = r#"
Welcome to the GoXLR Firmware Update Wizard, this tool will guide you through updating (or downgrading) your GoXLRs firmware.

It's recommended that you close other programs, and make sure that both the official application and the utility are closed before proceeding.

Click Next to continue.
        "#;
        container(message).into()
    }

    fn license(&self, checked: bool, is_license_one: bool) -> Element<'a, PageMessages> {
        let message = if is_license_one {
            PageMessages::ToggleAcceptLicenseOne
        } else {
            PageMessages::ToggleAcceptLicenseTwo
        };

        let license_text = if is_license_one {
            LICENSE
        } else {
            LICENSE_3RD_PARTY
        };

        let license = scrollable(text(license_text).size(14)).height(Length::Fill);

        // We don't have a monospace font yet, so we'll have to do this the old fashioned way.
        let check = container(checkbox(
            "I accept the terms of the License Agreement",
            checked,
            message,
        ))
        .padding(Padding {
            top: 10.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        });

        container(column![license, check]).into()
    }

    fn status(
        &self,
        app: bool,
        beta: bool,
        util: bool,
        sender: UnboundedSender<Message>,
    ) -> Element<'a, PageMessages> {
        // Ok, despite there being 3 bools here, due to the way the apps are designed, only one
        // can be true at any given point in time. So we'll create a message to cater to the specific
        // case.

        let _ = sender.send(Message::NextPressed);

        let message = if !app {
            "Please close the GoXLR App before continuing"
        } else if !beta {
            "Please close the GoXLR Beta App before continuing"
        } else if !util {
            "Please close the GoXLR utiltiy before continuing"
        } else {
            let _ = sender.send(Message::NextPressed);
            "Good to go, click 'Next' to Continue!"
        };

        let msg = container(message).padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 30.0,
            left: 0.0,
        });

        container(column![msg]).into()
    }

    fn find_goxlr(
        &self,
        selected: Option<usize>,
        list: &Option<Vec<Device>>,
        sender: UnboundedSender<Message>,
    ) -> Element<'a, PageMessages> {
        if let Some(list) = list {
            if list.is_empty() {
                println!("No Devices..");
                return container(column![text(
                    "No GoXLRs Found, please attach one and restart."
                )])
                .into();
            } else if list.len() == 1 {
                println!("One Device..");
                // Only one device, select and skip to the next page.
                let _ = sender.send(Message::PageMessage(PageMessages::SelectDevice(0)));
                let _ = sender.send(Message::NextPressed);
            } else {
                return container(
                    column(
                        list.iter()
                            .cloned()
                            .enumerate()
                            .map(|(i, device)| {
                                let label = format!(
                                    "[{}] GoXLR {:?}",
                                    device.device_serial, device.device_type
                                );
                                radio(label, i, selected, PageMessages::SelectDevice)
                            })
                            .map(Element::from)
                            .collect(),
                    )
                    .spacing(10),
                )
                .into();
            }
        }

        container(column![text("Please Wait..")]).into()
    }

    fn select_choice(&self, selected: Option<SelectUpdateOption>) -> Element<'a, PageMessages> {
        let download = radio(
            "Download Latest (EXPERIMENTAL)",
            SelectUpdateOption::Download,
            selected,
            PageMessages::SelectFirmwareOption,
        );
        let file = radio(
            "Select File",
            SelectUpdateOption::File,
            selected,
            PageMessages::SelectFirmwareOption,
        );

        container(column![download, file].spacing(10)).into()
    }

    #[allow(clippy::too_many_arguments)]
    fn select_file(
        &self,
        sender: UnboundedSender<Message>,
        fetch_method: &Option<SelectUpdateOption>,
        details: &Option<FirmwareDetails>,
        device: &Option<Device>,
        file: &Option<PathBuf>,
        progress: u8,
        downgrade: &bool,
    ) -> Element<'a, PageMessages> {
        // For the selection, there are now two options.. The first is waiting for a download to
        // complete and providing a file, the second is allowing the user to directly select a
        // file, so we need a bit of potential sh

        let button = match fetch_method {
            None => Some(button("Select Firmware")),
            Some(option) => match option {
                SelectUpdateOption::Download => None,
                SelectUpdateOption::File => {
                    Some(button("Select Firmware").on_press(PageMessages::SelectFirmware))
                }
            },
        };

        // Firstly, create a top row for selecting the file..
        let mut header = row![];
        let file_text = if let Some(file) = file {
            format!("{}", file.file_name().unwrap().to_string_lossy())
        } else {
            "No File Selected".to_string()
        };

        // We need to define the 'File' box based on whether we're downloading a firmware, or
        // asking the user to select..
        let file_box = if fetch_method == &Some(SelectUpdateOption::Download) {
            let progress_bar = progress_bar(0.0..=100.0, progress as f32).width(Length::Fill);
            let progress_text =
                container(text(format!("{}%", progress)))
                    .width(50)
                    .padding(Padding {
                        top: 5.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: 5.0,
                    });

            if let Some(file) = file {
                let file_text = format!("{}", file.file_name().unwrap().to_string_lossy());
                container(text(file_text))
                    .padding(Padding {
                        top: 5.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                    })
                    .width(Length::Fill)
            } else {
                container(row![progress_bar, progress_text])
            }
        } else {
            container(text(file_text))
                .padding(Padding {
                    top: 5.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .width(Length::Fill)
        };

        header = header.push(file_box);
        if let Some(button) = button {
            header = header.push(button);
        }
        let header = container(header).padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 10.0,
            left: 0.0,
        });

        let mut messages = column![];
        let mut valid = true;
        let mut is_downgrade = false;
        let mut is_reinstall = false;

        if let Some(details) = details {
            // We need the current device info here..
            if let Some(device) = device {
                if device.device_type != details.device_type {
                    let expected = match details.device_type {
                        DeviceType::Full => "Full Sized GoXLR",
                        DeviceType::Mini => "GoXLR Mini",
                        DeviceType::Unknown => "Hi! You broke something badly! Contact Frosty.",
                    };

                    messages = messages.push(text(format!(
                        "This firmware is only compatible with the {}",
                        expected
                    )));
                    valid = false;
                } else if version_newer_or_equal_to(&device.version, details.version) {
                    is_downgrade = true;
                }

                if valid {
                    let current = text(format!("Current Firmware: {}", device.version));
                    let new_version = text(format!("Selected Firmware: {}", details.version));
                    messages = messages.push(new_version);
                    messages = messages.push(current);
                }
                if is_downgrade && (device.version == details.version) {
                    is_reinstall = true;
                }
            }
        } else if file.is_some() {
            messages = messages.push("Selected file is not a GoXLR Firmware");
            valid = false;
        } else {
            valid = false;
        }

        if valid && is_downgrade {
            let task = if is_reinstall {
                "Reinstall"
            } else {
                "Downgrade"
            };

            messages = messages.push(Space::new(Length::Fill, Length::Fill));
            messages = messages.push(checkbox(
                format!("Confirm Firmware {}", task),
                *downgrade,
                PageMessages::SetAcceptDowngrade,
            ));
        }

        // We'll get called a few times for any changes, so can inform the parent if we're ready
        // to go.
        let ready = !(!valid || is_downgrade && !*downgrade);
        let _ = sender.send(Message::PageMessage(PageMessages::SetFirmwareValid(ready)));

        // We need this so we can actually do the firmware update, if the user changes the file,
        // this will be updated.
        if ready {
            if let Some(details) = details {
                let _ = sender.send(Message::StepsMessage(StepMessages::SetFirmware(
                    details.clone(),
                )));
            }
        } else {
            let _ = sender.send(Message::StepsMessage(StepMessages::ClearFirmware()));
        }

        let message_container = container(messages).padding(Padding {
            top: 10.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        });

        container(column![header, Rule::horizontal(5), message_container]).into()
    }

    fn run_update(
        &self,
        stage: &String,
        percent: u8,
        message: Option<String>,
        is_complete: bool,
        is_error: bool,
    ) -> Element<'a, PageMessages> {
        let progress_bar = progress_bar(0.0..=100.0, percent as f32).width(Length::Fill);
        let progress_text = container(text(format!("{}%", percent)))
            .width(50)
            .padding(Padding {
                top: 5.0,
                right: 0.0,
                bottom: 0.0,
                left: 5.0,
            });
        let row = row![progress_bar, progress_text];

        let mut page = column![];
        page = page.push(text(stage));
        page = page.push(row);

        if let Some(message) = message {
            page = page.push(text(message));
        }

        page = page.push(Space::new(Length::Fill, 30));

        if is_complete {
            if is_error {
                let message = "An error occurred updating your GoXLR, it has been rebooted back into it's previous firmware.";
                page = page.push(text(message));
            } else {
                let message = "Your GoXLR was successfully updated and has been rebooted.";
                page = page.push(text(message));
            }
            let message = "You can now close this tool, and restart the GoXLR App of your choice!";
            page = page.push(message);
        }

        container(page).into()
    }
}

pub fn version_newer_or_equal_to(version: &VersionNumber, comparison: VersionNumber) -> bool {
    match version.0.cmp(&comparison.0) {
        Ordering::Greater => return true,
        Ordering::Less => return false,
        Ordering::Equal => {}
    }

    match version.1.cmp(&comparison.1) {
        Ordering::Greater => return true,
        Ordering::Less => return false,
        Ordering::Equal => {}
    }

    match version.2.cmp(&comparison.2) {
        Ordering::Greater => return true,
        Ordering::Less => return false,
        Ordering::Equal => {}
    }

    if version.3 >= comparison.3 {
        return true;
    }

    false
}
