use fern::Dispatch;
use fern::colors::{Color, ColoredLevelConfig};
use log::LevelFilter;
use std::fs::OpenOptions;

pub fn setup_logging() -> anyhow::Result<()> {
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("history.log")?;

    Dispatch::new()
        .format(|out, message, record| {
            let level = record.level();
            let colors = ColoredLevelConfig::new()
                .error(Color::Red)
                .warn(Color::Yellow)
                .info(Color::Green)
                .debug(Color::Blue)
                .trace(Color::Magenta);

            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                colors.color(level),
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
