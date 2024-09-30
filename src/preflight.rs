use crate::{Message, PageMessages};
use std::thread::sleep;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

#[allow(dead_code)]
const APP: &str = "GoXLR App.exe";
#[allow(dead_code)]
const BETA: &str = "GoXLR Beta App.exe";
#[allow(dead_code)]
const UTIL: &str = "goxlr-daemon.exe";

pub fn status_check(sender: UnboundedSender<Message>) {
    println!("Starting Task Checker..");
    let mut system = None;

    #[cfg(target_family = "unix")]
    {
        use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
        let refresh_kind = RefreshKind::new()
            .with_processes(ProcessRefreshKind::new().with_user(UpdateKind::Always));
        system.replace(System::new_with_specifics(refresh_kind));
    }

    loop {
        let mut app_running = false;
        let mut beta_running = false;
        let mut utility_running = false;

        #[cfg(target_os = "windows")]
        {
            unsafe {
                let tasks = tasklist::Tasklist::new();

                tasks.for_each(|task| {
                    if task.get_pname() == APP {
                        app_running = true;
                    }
                    if task.get_pname() == BETA {
                        beta_running = true;
                    }
                    if task.get_pname() == UTIL {
                        utility_running = true;
                    }
                });
            }
        }
        #[cfg(target_family = "unix")]
        {
            if let Some(system) = &mut system {
                system.refresh_processes();
                let count = system.processes_by_exact_name("goxlr-daemon").count();
                if count > 0 {
                    utility_running = true;
                }
                app_running = false;
                beta_running = false;
            }
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
