use crate::{
    api::LocalServer,
    beaconer, gateway,
    router::{dispatcher, Dispatcher},
    settings::{self, Settings},
    Result,
};
use slog::{info, Logger};

pub async fn run(shutdown: &triggered::Listener, settings: &Settings, logger: &Logger) -> Result {
    let gateway_messages = gateway::message_channel();
    let dispatcher_messages = dispatcher::message_channel();
    let dispatch_sender = dispatcher_messages.sender();
    let beaconer_messages = beaconer::message_channel();
    let beaconer_sender = beaconer_messages.sender();
    let mut beaconer =
        beaconer::Beaconer::new(settings, gateway_messages.sender(), beaconer_messages);
    let mut dispatcher = Dispatcher::new(dispatcher_messages, gateway_messages.sender(), settings)?;
    let mut gateway = gateway::Gateway::new(
        dispatch_sender.clone(),
        gateway_messages,
        beaconer_sender,
        settings,
    )
    .await?;
    let api = LocalServer::new(dispatch_sender.clone(), settings)?;
    info!(logger,
        "starting server";
        "version" => settings::version().to_string(),
        "key" => settings.keypair.public_key().to_string(),
    );
    tokio::try_join!(
        beaconer.run(shutdown, logger),
        gateway.run(shutdown, logger),
        dispatcher.run(shutdown, logger),
        api.run(shutdown, logger),
    )
    .map(|_| ())
}
