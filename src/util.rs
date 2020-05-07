//! Contains a few utility functions that make your life easier.
use crate::{Channel, ClientInfo};

/// Tries to find a channel named `name` inside `client_info`.
pub fn get_channel_by_name(client_info: &ClientInfo, name: String) -> Option<Channel> {
    client_info
        .channel_info
        .values()
        .find(|channel| channel.info.get_name() == name).cloned()
}
