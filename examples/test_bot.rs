use mumble_rs::prelude::*;
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::env;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let mut client = Client::new(
        HandleStruct {},
        // CHange this to your server
        "IP:port".parse().unwrap(),
        "BettinaBot".to_string(),
        false,
    )
    .await
    .unwrap();
    client.run().await.unwrap();
}

struct HandleStruct {}

#[async_trait]
impl Handler for HandleStruct {
    async fn handle(
        &mut self,
        sender: &mut Sender,
        packet: &Packet,
        client_info: &ClientInfo,
    ) -> Result<()> {
        Ok(())
    }

    async fn ready(&mut self, sender: &mut Sender, client_info: &ClientInfo) -> Result<()> {
        let args: Vec<String> = env::args().collect();
        let channel_name = &args[1];
        let channel = get_channel_by_name(client_info, channel_name.to_string()).unwrap();
        let mut users = channel.users.clone();
        users.shuffle(&mut thread_rng());
        let users: Vec<&str> = users.iter().map(|user| user.get_name()).collect();
        let result = format!("Order: <p>{}</p>", users.join("</p><p>"));
        sender
            .send_message(result, channel.info.get_channel_id(), client_info)
            .await?;
        Ok(())
    }

    async fn finish(&mut self, sender: &mut Sender, client_info: &ClientInfo) -> Result<()> {
        Ok(())
    }
}
