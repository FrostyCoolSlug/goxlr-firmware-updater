use crate::{Message, PageMessages};
use std::thread::sleep;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const APP: &str = "GoXLR App.exe";
const BETA: &str = "GoXLR Beta App.exe";
const UTIL: &str = "goxlr-daemon.exe";

pub fn status_check(sender: UnboundedSender<Message>) {
    println!("Starting Task Checker..");
    loop {
        let mut app_running = false;
        let mut beta_running = false;
        let mut utility_running = false;

        #[cfg(target_os = "windows")] {
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
