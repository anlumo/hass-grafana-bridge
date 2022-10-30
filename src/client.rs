use std::{collections::HashMap, sync::Arc};

use futures::lock::Mutex;
use hass_rs::{HassClient, HassEntity, WSEvent};
use pharos::{Events, ObserveConfig, PharErr, Pharos, SharedPharos};
use tokio_tungstenite::tungstenite::Message;

pub struct Client {
    client: HassClient,
    pharos: SharedPharos<(String, Message)>,
    entity_state: Arc<Mutex<HashMap<String, Message>>>,
}

impl Client {
    pub async fn new(server: &str, port: u16, token: &str) -> Result<Self, ()> {
        log::info!("Connecting to Home Assistant...");
        let mut client = match hass_rs::connect(server, port).await {
            Err(err) => {
                log::error!("Failed to connect to Home Assistant: {err:?}");
                return Err(());
            }
            Ok(client) => client,
        };

        client
            .auth_with_longlivedtoken(token)
            .await
            .expect("Failed authenticating to Home Assistant");

        log::debug!("Client connected and authenticated to Home Assistant.");

        let entity_state: Arc<Mutex<HashMap<_, _>>> = Arc::new(Mutex::new(
            client
                .get_states()
                .await
                .expect("Can't fetch current state")
                .into_iter()
                .map(|entity| (entity.entity_id.clone(), Self::entity_to_message(entity)))
                .collect(),
        ));

        let pharos = SharedPharos::new(Pharos::new(2));

        let inner_pharos = pharos.clone();
        let inner_entity_state = entity_state.clone();
        client
            .subscribe_event("state_changed", move |WSEvent { event, .. }| {
                if let Some(entity) = event.data.new_state {
                    let entity_state = inner_entity_state.clone();
                    let pharos = inner_pharos.clone();
                    let entity_id = entity.entity_id.clone();
                    let message = Self::entity_to_message(entity);
                    tokio::spawn(async move {
                        entity_state
                            .lock()
                            .await
                            .insert(entity_id.clone(), message.clone());
                        pharos.notify((entity_id, message)).await.ok();
                    });
                }
            })
            .await
            .expect("Failed subscribing to state changes");
        Ok(Self {
            client,
            pharos,
            entity_state,
        })
    }

    fn entity_to_message(entity: HassEntity) -> Message {
        Message::Text(
            serde_json::to_string(&serde_json::json!({
                "entity_id": entity.entity_id,
                "state": entity.state,
                "last_changed": entity.last_changed,
                "last_updated": entity.last_updated,
                "attributes": entity.attributes,
            }))
            .unwrap(),
        )
    }

    pub async fn observe(
        &self,
        options: ObserveConfig<(String, Message)>,
    ) -> Result<Events<(String, Message)>, PharErr> {
        self.pharos.observe_shared(options).await
    }

    pub async fn current_state(&self) -> HashMap<String, Message> {
        self.entity_state.lock().await.clone()
    }
}
