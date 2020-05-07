use async_trait::async_trait;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use log::{debug, info};
use mumble_protocol::control::{msgs, ClientControlCodec, ControlPacket};
use mumble_protocol::voice::Serverbound;
use mumble_protocol::Clientbound;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{delay_for, Duration};
use tokio_tls::{TlsConnector, TlsStream};
use tokio_util::codec::{Decoder, Framed};

pub mod prelude;
pub mod util;

#[derive(Debug)]
/// The central client struct, responsible for connecting, preprocessing packets
/// for the `handler` and keeping user information up to date.
pub struct Client<T> {
    pub sender: Arc<Mutex<Sender>>,
    receiver: SplitStream<Framed<TlsStream<TcpStream>, ClientControlCodec>>,
    handler: T,
    info: ClientInfo,
}

#[derive(Debug, Clone)]
/// Contains all information about the client and the server we could collect.
pub struct ClientInfo {
    pub channel_info: HashMap<u32, Channel>,
    pub server_info: Option<msgs::Version>,
    pub username: String,
    pub actor_id: u32,
    pub session_id: u32,
}

#[derive(Debug)]
/// The crate's error enum.
pub enum Error {
    /// Might be thrown whenever interacting with the network somehow.
    Network(tokio::io::Error),
    /// Might be thrown during the initial TLS connection or if something
    /// TLS related goes wrong while sending a packet.
    Tls(native_tls::Error),
}

/// A convenience type for the `SplitSink` we use in order to communicate iwth the
/// server.
pub type Sender =
    SplitSink<Framed<TlsStream<TcpStream>, ClientControlCodec>, ControlPacket<Serverbound>>;
/// A convenience type for all packets we receive from the server.
pub type Packet = ControlPacket<Clientbound>;

/// A convenience type for error Handling, the Err variant will always contain an
/// `Error` enum
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
/// Contains infomration about the channel as well as a Vec of all users known
/// to be connected to the channel.
pub struct Channel {
    pub users: Vec<msgs::UserState>,
    pub info: msgs::ChannelState,
}

impl<T> Client<T>
where
    T: Handler,
{
    /// Create a new client and attempt connect it to `host` as `username`.
    /// `verify_certificate` determines wether the server's SSL certificate
    /// gets verified or not.
    pub async fn new(
        mut handler: T,
        host: SocketAddr,
        username: String,
        verify_certificate: bool,
    ) -> Result<Self> {
        info!("Connecting");
        let stream = TcpStream::connect(&host).await.map_err(Error::Network)?;

        let mut builder = native_tls::TlsConnector::builder();
        builder.danger_accept_invalid_certs(!verify_certificate);
        let connector: TlsConnector = builder.build().unwrap().into();

        debug!("Opening TLS stream");
        let tls_stream = connector
            .connect(&host.ip().to_string(), stream)
            .await
            .map_err(Error::Tls)?;

        let (mut sender, mut receiver) = ClientControlCodec::new().framed(tls_stream).split();

        debug!("Authenticating");
        let mut msg = msgs::Authenticate::new();
        msg.set_username(username.clone());
        msg.set_opus(true);
        debug!("message: {:?}", msg);
        sender.send(msg.into()).await.map_err(Error::Network)?;

        debug!("Version exchange");
        let mut server_info = None;
        if let Some(packet) = receiver.next().await {
            match packet.unwrap() {
                ControlPacket::Version(info) => {
                    debug!("Got server version information: {:?}", info);
                    server_info = Some(*info);
                }
                unexpected => debug!(
                    "Received {:?}, this should not be sent during the version exchange squence",
                    unexpected
                ),
            }
        }

        let mut msg = msgs::Version::new();
        // Version 1.2.5 would be 1 << 16 | 2 << 8 | 5 = 66053
        msg.set_version(66053);
        let mut version = "mumble_rs".to_string();
        version.push_str(env!("CARGO_PKG_VERSION"));
        msg.set_release(version);
        let os_info = os_info::get();
        msg.set_os(os_info.os_type().to_string());
        msg.set_os_version(os_info.version().to_string());
        debug!("Message: {:?}", msg);
        sender.send(msg.into()).await.map_err(Error::Network)?;

        debug!("Gathering server information");
        let mut channel_info = HashMap::new();
        let mut actor_id = None;
        let mut session_id = None;
        while let Some(packet) = receiver.next().await {
            match packet.unwrap() {
                ControlPacket::ServerSync(_) => break,
                ControlPacket::ChannelState(channel) => {
                    let channel = Channel {
                        users: Vec::new(),
                        info: *channel,
                    };
                    channel_info.insert(channel.info.get_channel_id(), channel);
                }
                ControlPacket::UserState(user) => {
                    // Unless the server send us an invalid Channel list this unwrap
                    // should be fine
                    if user.get_name() == username {
                        actor_id = Some(user.get_actor());
                        session_id = Some(user.get_session());
                    }
                    channel_info
                        .get_mut(&user.get_channel_id())
                        .expect("User in invalid channel")
                        .users
                        .push(*user);
                }
                unexpected => debug!(
                    "Received {:?}, this should not be sent during the server sync squence",
                    unexpected
                ),
            }
        }
        debug!("Got channel info: {:?}", channel_info);
        info!("Connected");

        let info = ClientInfo {
            channel_info,
            server_info,
            username,
            actor_id: actor_id.expect(
                "We didn't get out own user back from the server, this shouldn't be happening",
            ),
            session_id: session_id.expect(
                "We didn't get out own user back from the server, this shouldn't be happening",
            ),
        };

        debug!("Running ready handler");
        handler.ready(&mut sender, &info).await?;

        let sender = Arc::new(Mutex::new(sender));
        Ok(Client {
            sender,
            receiver,
            handler,
            info,
        })
    }

    /// Disconnect from the server and call the finish function of `handler`.
    pub async fn disconnect(mut self) -> Result<()> {
        let mut lock = self.sender.lock().await;
        self.handler.finish(&mut lock, &self.info).await
    }

    /// Execute the endless mainloop as well as pinging the server ever 28s.
    /// This will run the `handle` function of `handler` on every packet,
    /// after it has preprocessed it.
    pub async fn run(&mut self) -> Result<()> {
        let (ping_result, handle_result) =
            futures::join!(Self::ping(self.sender.clone()), self.handle());
        ping_result?;
        handle_result?;
        Ok(())
    }

    #[allow(unreachable_code)]
    async fn ping(sender: Arc<Mutex<Sender>>) -> Result<()> {
        loop {
            delay_for(Duration::from_secs(28)).await;
            let ping = msgs::Ping::new();
            let mut lock = sender.lock().await;
            lock.send(ping.into()).await.map_err(Error::Network)?;
            debug!("Ping sent");
        }
        Ok(())
    }

    async fn handle(&mut self) -> Result<()> {
        info!("Starting event loop");
        while let Some(packet) = self.receiver.next().await {
            let packet = packet.unwrap();
            debug!("Handling packet: {:?}", packet);
            match packet {
                // DO our own processing
                _ => ()
            };

            let mut lock = self.sender.lock().await;
            self.handler.handle(&mut lock, &packet, &self.info).await?;
        }
        Ok(())
    }
}

#[async_trait]
/// Extension trait for `Sender` so the Handler can e.g. send messages on its
/// own
pub trait SenderExt {
    /// Send `message` to `channel_id`, HTML can be used here.
    async fn send_message(
        &mut self,
        message: String,
        channel_id: u32,
        client_info: &ClientInfo,
    ) -> Result<()>;
}

#[async_trait]
impl SenderExt for Sender {
    async fn send_message(
        &mut self,
        message: String,
        channel_id: u32,
        client_info: &ClientInfo,
    ) -> Result<()> {
        let mut msg = msgs::TextMessage::new();
        msg.set_actor(client_info.actor_id);
        msg.set_session(vec![client_info.session_id]);
        msg.set_channel_id(vec![channel_id]);
        msg.set_message(message);
        debug!("message: {:?}", msg);
        self.send(msg.into()).await.map_err(Error::Network)?;
        Ok(())
    }
}

#[async_trait]
/// Trait responsible for all custom event handling related logic.
pub trait Handler {
    /// Called for every packet received after the `ready` call.
    async fn handle(
        &mut self,
        sender: &mut Sender,
        packet: &Packet,
        client_info: &ClientInfo,
    ) -> Result<()>;
    /// Called once the connection is set up by `Client`.
    async fn ready(&mut self, sender: &mut Sender, client_info: &ClientInfo) -> Result<()>;
    /// Called right before `Client`shuts down fully.
    async fn finish(&mut self, sender: &mut Sender, client_info: &ClientInfo) -> Result<()>;
}
