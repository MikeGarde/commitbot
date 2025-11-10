use std::io::Write;

use colored::Colorize;
use env_logger::Builder;
use log::{Level, LevelFilter};

pub fn init_logger(verbosity: u8) {
    let level = match verbosity {
        0 => LevelFilter::Error, // default: only errors
        1 => LevelFilter::Info,  // -v: info and up
        2 => LevelFilter::Debug, // -vv: debug and up
        _ => LevelFilter::Trace, // -vvv: trace and up
    };

    let mut builder = Builder::new();
    builder.filter_level(level);

    builder.format(|buf, record| {
        let level = record.level();

        let level_label = match level {
            Level::Error => "ERROR".red().bold(),
            Level::Warn  => "WARN ".yellow().bold(),
            Level::Info  => "INFO ".white().bold(),
            Level::Debug => "DEBUG".bright_black(),
            Level::Trace => "TRACE".bright_black(),
        };

        writeln!(
            buf,
            "{} {}",
            level_label,
            record.args()
        )
    });

    builder.init();
}
