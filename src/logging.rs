use std::sync::RwLock;

use std::fs::{OpenOptions, File};
use std::io::Write;

use log::{self, LogRecord, LogLevel, LogLevelFilter, LogMetadata, SetLoggerError};

struct ScreenLogger;

impl log::Log for ScreenLogger {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        metadata.level() <= LogLevel::Info
    }

    fn log(&self, record: &LogRecord) {
        if self.enabled(record.metadata()) {
            let line = format!("{} - {}", record.level(), record.args());
            let mut logs = LOGS.write().unwrap();
            logs.insert(0, line);
            logs.truncate(5);
        }
        if record.metadata().level() == LogLevel::Debug {
            let line = format!("{} - {}\n", record.level(), record.args());
            // TODO configure this
            let mut f = OpenOptions::new()
                .append(true)
                .create(true)
                .open("/home/t/src/climate/debug.log")
                .unwrap();
            f.write_all(line.as_bytes()).unwrap();
            f.sync_all().unwrap();
        }
    }
}

pub fn init_screen_log() -> Result<(), SetLoggerError> {
    log::set_logger(|max_log_level| {
        max_log_level.set(LogLevelFilter::Debug);
        Box::new(ScreenLogger)
    })
}

pub fn read_logs() -> Vec<String> {
    LOGS.read().unwrap().clone()
}

lazy_static! {
    static ref LOGS: RwLock<Vec<String>> = RwLock::new(vec![]);
}
