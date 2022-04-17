use std::{
    collections::{BTreeMap, HashMap},
};

use serde::{Deserialize, Serialize};
use tokio::{
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    sync::{Mutex, oneshot::Receiver},
};
use tokio::{
    process::{Child, Command},
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot::{self, Sender},
    },
};
use tokio_serde::formats::*;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{debug, error, info, trace};
use uuid::Uuid;

use crate::Result;

use futures::prelude::*;
use futures::stream::StreamExt;

#[derive(Serialize, Deserialize, Debug)]
pub struct Request {
    id: i32,
    data: RequestData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestData {
    pub environment: String,
    pub endpoint: String,
    pub extension: String,
    pub id: String,
    pub parameters: Option<BTreeMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    id: i32,
    pub data: String,
    pub progress_messages: Option<Vec<String>>,
    pub html_output: String,
    pub status: String,
}

pub struct Client {
    inner: Mutex<Option<Inner>>,
}

impl Client {
    pub fn new() -> Client {
        Client {
            inner: Mutex::new(None),
        }
    }

    pub async fn request(&self, request: RequestData) -> Result<Response> {
        let mut guard = self.inner.lock().await;
        let inner = if guard.is_none() {
            guard.insert(Inner::new()?)
        } else {
            guard.as_mut().unwrap()
        };

        trace!("Request sent to runner");
        let rx = inner.queue_request(request)?;
        drop(guard);
        Ok(rx.await?)
    }

    pub async fn drop_inner(&self) {
        let mut guard = self.inner.lock().await;
        let _ = guard.take();
    }
}

struct Inner {
    shutdown: Option<oneshot::Sender<()>>,
    message_tx: UnboundedSender<(Sender<Response>, RequestData)>,
    _process: Child,
}

impl Inner {
    pub fn new() -> Result<Inner> {
        let pipe_name = Uuid::new_v4().to_string();
        let pipe_name_client = format!("\\\\.\\pipe\\{}", pipe_name);
        let process = Command::new("acisrunner.exe")
            .arg(pipe_name)
            .kill_on_drop(true)
            .spawn()?;

        let mut count = 0;
        let client = loop {
            match ClientOptions::new().open(&pipe_name_client) {
                Ok(client) => break client,
                Err(e) => {
                    if count > 10 {
                        let _ = Err(e)?;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    count += 1;
                }
            }
        };

        let (tx, rx) = oneshot::channel::<()>();
        let (message_tx, message_rx) =
            mpsc::unbounded_channel();


        tokio::spawn(async move {
            let result = Inner::handle_requests(client, rx, message_rx).await;
            debug!("Named pipe listener stopped {:?}", result);
        });

        Ok(Inner {
            shutdown: Some(tx),
            message_tx,
            _process: process,
        })
    }


    pub fn queue_request(
        &mut self,
        request: RequestData,
    ) -> Result<Receiver<Response>> {
        let (tx, rx) = oneshot::channel::<Response>();
        self.message_tx.send((tx, request))?;
        Ok(rx)
    }

    async fn handle_requests(
        client: NamedPipeClient,
        mut rx: oneshot::Receiver<()>,
        mut message_rx: UnboundedReceiver<(Sender<Response>, RequestData)>,
    ) -> Result<()> {
        let length_delimited = Framed::new(
            client,
            LengthDelimitedCodec::builder().little_endian().new_codec(),
        );
        let mut serialized =
            tokio_serde::Framed::new(length_delimited, Json::<Response, Request>::default());
        let mut id = 0;
        let mut request_map = HashMap::new();

        loop {
            tokio::select! {
                biased;

                Some((tx, message)) = message_rx.recv() => {
                    id = id + 1;
                    let request = Request {
                        id,
                        data: message,
                    };

                    let _ = serialized.send(request).await?;
                    request_map.insert(id, tx);
                }
                data = serialized.next() => {
                    if let Some(data) = data {
                        if let Ok(data) = data {
                            match request_map.remove(&data.id) {
                                Some(tx) => {
                                    let _ = tx.send(data);
                                }
                                None => {
                                    error!("Received response for unknown request");
                                }
                            }
                        } else {
                            error!("Error in response {:?}", data);
                        }
                    } else {
                        info!("Named pipe stream closed");
                        break;
                    }
                },
                _ = &mut rx => {
                    break;
                }
            }
        }

        // let mut client = client.lock().await;
        // client.listener_active = false;

        Ok(())
    }
}

unsafe impl Send for Client {}
unsafe impl Sync for Client {}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(data) = self.shutdown.take() {
            data.send(()).unwrap();
        }
    }
}
