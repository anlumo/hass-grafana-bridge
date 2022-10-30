use clap::Parser;
use client::Client;
use futures::{channel::oneshot, future::Either, stream::select};
use futures_util::{SinkExt, StreamExt};
use pharos::{Filter, ObserveConfig};
use std::{collections::HashSet, sync::Arc};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};

mod client;

#[derive(Debug, Parser)]
struct Cli {
    /// The address and port to listen on for WebSocket connections.
    #[clap(short, long, env = "LISTEN", default_value = "[::1]:8080")]
    listen: String,
    /// The Home Assistant IP to connect to.
    #[clap(short = 's', long, env = "HASS_SERVER", default_value = "localhost")]
    hass_server: String,
    /// The Home Assistant port to connec to.
    #[clap(short = 'p', long, env = "HASS_PORT", default_value = "8123")]
    hass_port: u16,
    /// The long-lived access token provided by Home Assistant.
    #[clap(short = 't', long, env = "HASS_TOKEN")]
    hass_token: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::try_init().unwrap();
    let args = Cli::parse();

    let client = Arc::new(
        Client::new(&args.hass_server, args.hass_port, &args.hass_token)
            .await
            .unwrap(),
    );

    let try_socket = TcpListener::bind(&args.listen).await;
    let listener = try_socket.expect("Failed to bind to socket");
    log::info!("Listening on: {}", args.listen);

    while let Ok((stream, _)) = listener.accept().await {
        let client = client.clone();
        tokio::spawn(async move {
            accept_connection(stream, client).await;
            log::debug!("spawn done");
        });
    }

    log::info!("Terminating.");
}

#[derive(Debug)]
struct HeaderParser {
    entities: oneshot::Sender<Option<Vec<String>>>,
}

impl Callback for HeaderParser {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        if let Some(entities) = request
            .headers()
            .get("hass-listen-entities")
            .and_then(|value| value.to_str().ok())
        {
            self.entities
                .send(Some(
                    entities
                        .split(',')
                        .map(std::borrow::ToOwned::to_owned)
                        .collect(),
                ))
                .map_err(|_| {
                    Response::builder()
                        .status(500)
                        .body(Some("Internal server error".to_owned()))
                        .unwrap()
                })?;
        } else {
            self.entities.send(None).map_err(|_| {
                Response::builder()
                    .status(500)
                    .body(Some("Internal server error".to_owned()))
                    .unwrap()
            })?;
        }

        Ok(response)
    }
}

async fn accept_connection(stream: TcpStream, client: Arc<Client>) {
    let addr = stream
        .peer_addr()
        .expect("Connected streams should have a peer address");
    log::info!("Incoming connection from {addr}");

    let (entities_sender, entities_receiver) = oneshot::channel();
    let parser = HeaderParser {
        entities: entities_sender,
    };

    let incoming_stream = match tokio_tungstenite::accept_hdr_async(stream, parser).await {
        Err(err) => {
            log::error!("Error during the websocket handshake: {err:?}");
            return;
        }
        Ok(stream) => stream,
    };
    let entities: Option<HashSet<String>> = entities_receiver
        .await
        .expect("Failed to receive HTTP headers")
        .map(|entities| entities.into_iter().collect());

    let (mut incoming_write, incoming_read) = incoming_stream.split();

    let observer = if let Some(entities) = &entities {
        let inner_entities = entities.clone();
        let observer = client
            .observe(Filter::Closure(Box::new(move |(id, _)| inner_entities.contains(id))).into())
            .await
            .unwrap();
        for (id, entity) in client.current_state().await {
            if !entities.contains(&id) {
                continue;
            }
            incoming_write.send(entity).await.unwrap();
        }
        observer
    } else {
        let observer = client.observe(ObserveConfig::default()).await.unwrap();
        for (_, entity) in client.current_state().await {
            incoming_write.send(entity).await.unwrap();
        }
        observer
    };

    // wait until incoming client closes connection
    let mut stream = select(incoming_read.map(Either::Left), observer.map(Either::Right));
    while let Some(Either::Right((_, message))) = stream.next().await {
        incoming_write.send(message).await.ok();
    }

    log::info!("Client connection {addr} closed.");
}
