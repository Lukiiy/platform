use anyhow::Result;
use craftping::tokio::ping;
use tokio::net::TcpStream;

pub struct ServerStatus {
    pub motd: String,
    pub version: String
}

pub async fn ping_server(host: &str, port: u16) -> Result<ServerStatus> {
    let pong = ping(&mut stream, host, port).await?;

    Ok(ServerStatus {
        motd: pong.description.text().to_string(),
        version: pong.version.name
    })
}