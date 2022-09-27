mod fixtures;

use fixtures::{fixtures, sats};
use std::time::Duration;

#[tokio::main]
async fn main() {
    let (fed, _, _, _, _) = fixtures(1, &[sats(100)]).await;
    let reqs = 1000; // requests == threads
    let poll = 10; // times
    let rest = 1; // seconds

    for _ in 0..poll {
        fed.open_sock_connection_on_peers(reqs).await;
        tokio::time::sleep(Duration::from_secs(rest)).await;
    }
}
