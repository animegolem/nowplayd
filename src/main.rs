use std::error::Error;

use nowplayd::mpd::{CommandConnection, ConnectionConfig, IdleConnection};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = ConnectionConfig::default();
    let mut commands = CommandConnection::connect(&config).await?;
    let mut idle = IdleConnection::connect(&config).await?;
    let mut state = commands.refresh().await?;
    eprintln!("initial MPD state: {state:?}");

    loop {
        let subsystems = idle.next_event().await?;
        let newer = commands.refresh().await?;
        let change = state.diff(&newer);
        eprintln!("MPD event {subsystems:?}; state change {change:?}");
        state = newer;
    }
}
