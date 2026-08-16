#[path = "support/harness.rs"]
mod harness;

use mpd_protocol::Command;
use nowplayd::{
    mpd::{CommandConnection, ConnectionConfig, IdleConnection, MpdAddress, Subsystem},
    state::PlaybackState,
};

use harness::{MpdHarness, SECOND_TITLE};

#[tokio::test]
async fn isolated_mpd_next_produces_one_coherent_song_change() {
    let harness = MpdHarness::start()
        .await
        .expect("start isolated MPD with null output");
    let mut test_client = harness.seed_and_play().await.expect("seed test queue");
    let config = ConnectionConfig {
        address: MpdAddress::Unix(harness.socket().into()),
        password: None,
    };
    let mut commands = CommandConnection::connect(&config)
        .await
        .expect("connect command role");
    let mut idle = IdleConnection::connect(&config)
        .await
        .expect("connect idle role");

    let before = commands.refresh().await.expect("initial coherent snapshot");
    assert_eq!(before.playback, PlaybackState::Playing);

    let (event, next_response) =
        tokio::join!(idle.next_event(), test_client.command(Command::new("next")));
    let event = event.expect("receive idle event");
    assert_eq!(event, vec![Subsystem::Player]);
    let next_response = next_response.expect("issue next from separate test client");
    assert!(next_response.is_success());

    let after = commands.refresh().await.expect("new coherent snapshot");
    let change = before.diff(&after);
    assert!(change.occurrence, "occurrence identity must change");
    assert!(change.media_key, "media key must change");
    assert_eq!(after.metadata.title.as_deref(), Some(SECOND_TITLE));

    let removed = harness.shutdown().expect("tear down isolated MPD");
    assert!(!removed.exists(), "temporary MPD state must be removed");
}
