use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init_tracing(verbose: u8) -> Result<()> {
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(level))?;
    let subscriber = fmt().with_env_filter(filter).with_target(false).compact();
    let _ = subscriber.try_init();
    Ok(())
}
