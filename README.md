# Working with this
In examples/test_bot.rs is a small test bot that will try and connect to a channel on a server you provide it and then
print the names in that channel in random order into the chat, afterwards it should simply stay in the server and do
nothing. You can pass an IP + port as well as a channel for the bot to connect to like this: `cargo run --example=test_bot -- 192.168.178.1:1234 my_channel_name`

## The opus bug
In src/lib.rs line 100 you will find: `msg.set_opus(true);` this tells the bot to communicate to the server that it is
able to use the opus voice encoding, if this flag is set to false instead of true the server should downgrade the
communication codec for all clients to CELT since the mumble protocol assumes this encoding is supported by every mumble
client there is. In order to recompile with the new version simply rerun the cargo command from above.
