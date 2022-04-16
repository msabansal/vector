use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use tracing::{trace, debug, info, error};
use serde::{Deserialize, Serialize};
use tokio::{sync::oneshot, process::{Child, Command}};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    sync::Mutex,
};
use tokio_serde::formats::*;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
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
    pub status: String,
}

pub struct Client {
    inner: Arc<Mutex<Inner>>,
    _process: Child,
}

impl Client {
    pub fn new() -> Result<Client> {
        let pipe_name = Uuid::new_v4().to_string();
        let pipe_name_client = format!("\\\\.\\pipe\\{}", pipe_name);
        let process= Command::new("acisrunner.exe").arg(pipe_name).kill_on_drop(true).spawn()?;
        Ok(Client {
            _process: process,
            inner: Inner::new(&pipe_name_client.as_str())?,
        })
    }

    pub async fn request(&self, request: RequestData) -> Result<Response> {
        let client = Arc::clone(&self.inner);
        trace!("Request sent to runner");
        Inner::queue_request(client, request).await
    }
}

struct Inner {
    pipe: tokio_serde::Framed<
        FramedWrite<WriteHalf<NamedPipeClient>, LengthDelimitedCodec>,
        Request,
        Request,
        Json<Request, Request>,
    >,
    shutdown: Option<oneshot::Sender<()>>,
    listener_active: bool,
    request_map: HashMap<i32, oneshot::Sender<Response>>,
    request_id: i32,
}

impl Inner {
    pub fn new(pipe_name: &str) -> Result<Arc<Mutex<Inner>>> {
        let mut count = 0;
        let client = loop {
            match ClientOptions::new().open(pipe_name) {
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

        let (reader, writer) = tokio::io::split(client);
        let (tx, rx) = oneshot::channel::<()>();

        let length_delimited = FramedWrite::new(
            writer,
            LengthDelimitedCodec::builder().little_endian().new_codec(),
        );

        let serialized =
            tokio_serde::SymmetricallyFramed::new(length_delimited, SymmetricalJson::default());

        let piped = Arc::new(Mutex::new(Inner {
            pipe: serialized,
            shutdown: Some(tx),
            listener_active: true,
            request_map: HashMap::new(),
            request_id: 0,
        }));

        let cloned = Arc::clone(&piped);

        tokio::spawn(async move {
            let result = Inner::handle_requests(reader, rx, &cloned).await;
            debug!("Named pipe listener stopped {:?}", result);
        });

        Ok(piped)
    }

    async fn notify_listeners(self: &mut Self, response: Response) {
        let waiter = self.request_map.remove(&response.id);

        if let Some(waiter) = waiter {
            waiter.send(response).unwrap();
        } else {
            debug!("No waiter for response: {:?}", response.id);
        }
    }

    pub async fn queue_request(
        client: Arc<Mutex<Inner>>,
        request: RequestData,
    ) -> Result<Response> {
        let (tx, rx) = oneshot::channel::<Response>();
        let mut client = client.lock().await;
        client.request_id += 1;
        let id = client.request_id;
        let request = Request {
            id: client.request_id,
            data: request,
        };
        client.request_map.insert(id, tx);
        let result = client.pipe.send(request).await;
        if let Err(e) = result {
            client.request_map.remove(&id).unwrap();
            return Err(Box::new(e));
        }
        drop(client);

        Ok(rx.await?)
    }

    async fn handle_requests(
        reader: ReadHalf<NamedPipeClient>,
        mut rx: oneshot::Receiver<()>,
        client: &Arc<Mutex<Inner>>,
    ) -> Result<()> {
        let length_delimited = FramedRead::new(
            reader,
            LengthDelimitedCodec::builder().little_endian().new_codec(),
        );

        let mut serialized =
            tokio_serde::SymmetricallyFramed::new(length_delimited, SymmetricalJson::default());
        loop {
            tokio::select! {
                _ = &mut rx => {
                    break;
                }
                data = serialized.next() => {
                    if let Some(data) = data {
                        if let Ok(data) = data {
                            client.lock().await.notify_listeners(data).await;
                        } else {
                            error!("Error in response {:?}", data);
                        }
                    } else {
                        info!("Named pipe stream closed");
                        break;
                    }
                }
            }
        }

        let mut client = client.lock().await;
        client.listener_active = false;

        Ok(())
    }
}

unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(data) = self.shutdown.take() {
            data.send(()).unwrap();
        }
    }
}
