// © 2026 Massachusetts Institute of Technology
// MIT License

use std::{
    str::FromStr,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum LogLevel {
    /// Don't print anything
    NONE = 0,
    /// Only information necessary for CI is printed
    QUIET = 1,
    /// Some information is printed
    NORMAL = 2,
    /// Everything is printed
    VERBOSE = 3,
}

impl From<usize> for LogLevel {
    fn from(value: usize) -> Self {
        match value {
            0 => LogLevel::NONE,
            1 => LogLevel::QUIET,
            2 => LogLevel::NORMAL,
            3 => LogLevel::VERBOSE,
            _ => LogLevel::NORMAL, // should never happen but let's do normal
        }
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(LogLevel::NONE),
            "quiet" => Ok(LogLevel::QUIET),
            "normal" => Ok(LogLevel::NORMAL),
            "verbose" => Ok(LogLevel::VERBOSE),
            _ => Err(String::from(
                "Invalid log level. Must be one of none, quiet, normal, or verbose",
            )),
        }
    }
}

static LOG_LEVEL: AtomicUsize = AtomicUsize::new(LogLevel::NORMAL as usize);

pub fn set_log_level(log_level: LogLevel) {
    LOG_LEVEL.store(log_level as usize, Ordering::Relaxed)
}

pub fn get_log_level() -> LogLevel {
    LogLevel::from(LOG_LEVEL.load(Ordering::Relaxed))
}

/// General macro to use for logging at the appropriate level
///
/// # Examples
///
/// ```
/// use cando2::{log, LogLevel};
/// log!(LogLevel::QUIET, "hello"); // Prints only with `LogLevel::QUIET`
/// log!(LogLevel::NORMAL, "hello"); // Prints with `LogLevel::NORMAL` and `LogLevel::VERBOSE`
/// log!(LogLevel::VERBOSE, "hello"); // Prints only with `LogLevel::VERBOSE`
/// ```
#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)*) => {
        let curr_level = $crate::log::get_log_level();
        if curr_level != LogLevel::NONE {
            match ($level, curr_level) {
                (LogLevel::QUIET, LogLevel::QUIET) => println!($($arg)*),
                (LogLevel::NORMAL, LogLevel::NORMAL) | (LogLevel::NORMAL, LogLevel::VERBOSE) => {
                    println!($($arg)*)
                },
                (LogLevel::VERBOSE, LogLevel::VERBOSE) => println!($($arg)*),
                _ => {},
            }
        }
    }
}
