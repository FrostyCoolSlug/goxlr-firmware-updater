use crate::{Message, PageMessages};
use std::thread::sleep;
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
use tokio::sync::mpsc::UnboundedSender;


const APP: &str = "GoXLR App.exe";
const BETA: &str = "GoXLR Beta App.exe";
const UTIL: &str = "goxlr-daemon.exe";
const UTIL_LINUX: &str = "goxlr-daemon";

pub fn status_check(sender: UnboundedSender<Message>) {
    println!("Starting Task Checker..");

    let kind = ProcessRefreshKind::new().with_user(UpdateKind::Always);
    let refresh_kind = RefreshKind::new().with_processes(kind);
    let mut system = System::new_with_specifics(refresh_kind);

    loop {
        let mut app_running = false;
        let mut beta_running = false;
        let mut utility_running = false;


        system.refresh_processes();
        if system.processes_by_exact_name(APP).count() > 0 {
            app_running = true;
        }

        if system.processes_by_exact_name(BETA).count() > 0 {
            beta_running = true;
        }

        if system.processes_by_exact_name(UTIL).count() > 0 {
            utility_running = true;
        }

        if system.processes_by_exact_name(UTIL_LINUX).count() > 0 {
            utility_running = true;
        }

        // Fire off the message..
        let _ = sender.send(Message::PageMessage(PageMessages::UpdateStatusCheck(
            app_running,
            beta_running,
            utility_running,
        )));
        if !app_running && !beta_running && !utility_running {
            // Everything's shutdown, we don't need to check anymore..
            break;
        }
        sleep(Duration::from_secs(1));
    }
    println!("Task Checker Terminated");
}
