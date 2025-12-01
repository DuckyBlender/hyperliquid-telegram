use colored::Colorize;
use fern::Dispatch;
use log::LevelFilter;
use std::fs::OpenOptions;

pub fn setup_logging() -> anyhow::Result<()> {
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("bot.log")?;

    Dispatch::new()
        .format(|out, message, record| {
            let level = record.level();
            let level_str = match level {
                log::Level::Error => "ERROR".red().bold(),
                log::Level::Warn => "WARN".yellow().bold(),
                log::Level::Info => "INFO".green().bold(),
                log::Level::Debug => "DEBUG".blue().bold(),
                log::Level::Trace => "TRACE".magenta().bold(),
            };

            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                level_str,
                record.target(),
                message
            ))
        })
        .level(LevelFilter::Info)
        .level_for("sqlx", LevelFilter::Warn)
        .level_for("hyper", LevelFilter::Warn)
        .level_for("reqwest", LevelFilter::Warn)
        .chain(std::io::stdout())
        .chain(log_file)
        .apply()?;

    Ok(())
}
