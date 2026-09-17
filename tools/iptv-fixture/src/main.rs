use nova_iptv_fixture::{FixtureServer, Scenario};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let scenario = match args.as_slice() {
        [] => Scenario::Normal,
        [name] => Scenario::from_name(name).ok_or("unknown fixture scenario")?,
        _ => return Err("usage: nova-iptv-fixture [normal|vod-unavailable|vod-malformed|vod-empty|playlist-html]".into()),
    };
    let server = FixtureServer::start(scenario).await?;
    println!("Nova fixture v1: {} ({scenario:?})", server.base_url());
    println!("Playlist: {}/playlist.m3u", server.base_url());
    println!("Synthetic Xtream credentials: fixture / fixture. No playable media is served.");
    tokio::signal::ctrl_c().await?;
    server.shutdown().await?;
    Ok(())
}
