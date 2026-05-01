//! Direct TCP dial — used when no egress profile is selected, or
//! when the selected profile is [`super::EgressKind::None`].

use std::io;

use tokio::net::TcpStream;

use super::EgressStream;

/// Open a plain TCP connection to `host:port` and box it as an
/// [`EgressStream`].
pub(super) async fn dial_direct(host: &str, port: u16) -> io::Result<EgressStream> {
    let stream = TcpStream::connect((host, port)).await?;
    Ok(Box::new(stream))
}
