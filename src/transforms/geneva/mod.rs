use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::BTreeMap, convert::TryFrom};
use std::{pin::Pin, time::Duration};
use vector_core::event::EventStatus;

use crate::{
    config::{
        DataType, GenerateConfig, Input, Output, TransformConfig, TransformContext,
        TransformDescription,
    },
    event::Event,
    template::Template,
    transforms::{
        remap::{Remap, RemapConfig},
        TaskTransform, Transform, TransformOutputsBuf,
    },
};
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::mpsc;
use vector_common::TimeZone;
pub mod pipe_client;
use crate::vector_core::transform::SyncTransform;
use async_stream::stream;
use futures::{Stream, StreamExt};
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;

use governor::state::direct::StreamRateLimitExt;
use pipe_client::{Client, RequestData};

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct GenevaConfig {
    environment: String,
    extension: String,
    endpoint: String,
    operation: String,
    target: String,
    parameters: Option<IndexMap<String, String>>,
    threshold: Option<u32>,
    window_secs: Option<f64>,
    dry_run: bool,
    dry_run_output: String,
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Geneva {
    pub config: GenevaConfig,
    pub transform: Option<Box<Remap>>,
}

static PIPE_CLIENT: Lazy<Arc<Client>> = Lazy::new(|| Arc::new(Client::new()));

impl Clone for Geneva {
    fn clone(&self) -> Self {
        Self {
            transform: self.transform.clone(),
            config: self.config.clone(),
        }
    }
}

inventory::submit! {
    TransformDescription::new::<GenevaConfig>("geneva")
}

impl GenerateConfig for GenevaConfig {
    fn generate_config() -> toml::Value {
        toml::Value::try_from(Self {
            environment: "Test".to_string(),
            extension: "CanaryExtension".to_string(),
            endpoint: "Endpoint1".to_string(),
            operation: "JsonOutput".to_string(),
            parameters: None,
            window_secs: None,
            target: "Response".to_string(),
            threshold: None,
            dry_run: false,
            dry_run_output: ".bar = parse_json!(string!(.foo))".to_owned(),
        })
        .unwrap()
    }
}

// enum TransformError {
//     TemplateParseError(TemplateParseError),
//     TemplateRenderingError(TemplateRenderingError),
// }

fn render_template(s: &str, event: &Event) -> crate::Result<String> {
    let template = Template::try_from(s)?;
    Ok(template.render_string(event)?)
}

fn render_tags(
    tags: &Option<IndexMap<String, String>>,
    event: &Event,
) -> crate::Result<Option<BTreeMap<String, String>>> {
    Ok(match tags {
        None => None,
        Some(tags) => {
            let mut map = BTreeMap::new();
            for (name, value) in tags {
                let tag = render_template(value, event)?;
                map.insert(name.to_string(), tag);
            }
            if !map.is_empty() {
                Some(map)
            } else {
                None
            }
        }
    })
}

#[async_trait::async_trait]
#[typetag::serde(name = "geneva")]
impl TransformConfig for GenevaConfig {
    async fn build(&self, _context: &TransformContext) -> crate::Result<Transform> {
        Ok(Transform::event_task(Geneva::new(self.clone())?))
    }

    fn input(&self) -> Input {
        Input::log()
    }

    fn outputs(&self) -> Vec<Output> {
        vec![Output::default(DataType::Log)]
    }

    fn transform_type(&self) -> &'static str {
        "geneva"
    }
}

impl Geneva {
    pub fn new(config: GenevaConfig) -> crate::Result<Self> {
        let transform = if config.dry_run {
            Some(Box::new(
                Remap::new(
                    RemapConfig {
                        source: Some(config.dry_run_output.clone()),
                        file: None,
                        timezone: TimeZone::default(),
                        drop_on_error: true,
                        drop_on_abort: true,
                        ..Default::default()
                    },
                    &Default::default(),
                )
                .unwrap(),
            ))
        } else {
            None
        };

        Ok(Geneva { transform, config })
    }
}
impl TaskTransform<Event> for Geneva {
    fn transform(
        self: Box<Self>,
        input_rx: Pin<Box<dyn Stream<Item = Event> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Event> + Send>>
    where
        Self: 'static,
    {
        let inner = self;
        let (tx, mut rx) = mpsc::channel::<Event>(1);

        let quota = Quota::with_period(Duration::from_secs(1))
            .unwrap()
            .allow_burst(NonZeroU32::new(1).unwrap());
        let target_field = inner.config.target.clone();

        Box::pin(
            stream! {

                let limiter = RateLimiter::direct(quota);
                let mut input_rx = input_rx.ratelimit_stream(&limiter);
                // Not required as we are on a single thread but it is good practice
                let requests = AtomicUsize::new(0);

                loop {
                // let mut output = Vec::new();
                let done = tokio::select! {
                    biased;

                    maybe_event = input_rx.next() => {
                        match maybe_event {
                            None => {
                                true
                            },
                            Some(mut event) => {
                                if inner.config.dry_run {
                                    let mut outputs =
                                    TransformOutputsBuf::new_with_capacity(vec![Output::default(DataType::Any)], 1);
                                    let transform = inner.transform.clone();
                                    transform.unwrap().transform(event, &mut outputs);
                                    let result = outputs.take_primary().pop();
                                    // tracing::info!("Result: {:?}", result);
                                    // output.push(result);
                                    yield result.unwrap();
                                } else {
                                    let environment = render_template(&inner.config.environment, &event);
                                    let endpoint = render_template(&inner.config.endpoint, &event);
                                    let extension = render_template(&inner.config.extension, &event);
                                    let operation = render_template(&inner.config.operation, &event);
                                    let parameters = render_tags(&inner.config.parameters, &event);

                                    if environment.is_err() || endpoint.is_err() || extension.is_err() || operation.is_err() || parameters.is_err() {
                                        event.metadata_mut().update_status(EventStatus::Errored);
                                        if let Err(_) = tx.send(event).await {
                                            tracing::info!("Event dropped");
                                        }
                                        continue;
                                    }
                                    let target_field = target_field.clone();
                                    let tx = tx.clone();
                                    requests.fetch_add(1, Ordering::SeqCst);
                                    tokio::spawn(async move {
                                        let data = RequestData{
                                            environment: environment.unwrap(),
                                            endpoint: endpoint.unwrap(),
                                            extension: extension.unwrap(),
                                            id: operation.unwrap(),
                                            parameters: parameters.unwrap(),
                                        };

                                        tracing::trace!("Data {:?}", data);
                                        let client = Arc::clone(&*PIPE_CLIENT);
                                        let result = client.request(data).await;
                                        tracing::trace!("Result {:?}", result);

                                        if let Ok(result) = result {
                                            let json_value = serde_json::to_value(result).unwrap();
                                            event.as_mut_log().insert(&target_field, json_value);
                                        } else {
                                            event.metadata_mut().update_status(EventStatus::Errored);
                                        }
                                        if let Err(_) = tx.send(event).await {
                                            tracing::info!("Event dropped");
                                        }
                                    });
                                }
                                false
                            }
                        }
                    }
                    event = rx.recv() => {
                        requests.fetch_sub(1, Ordering::SeqCst);
                        // output.push(event);
                        yield event.unwrap();
                        false
                    }
                };
                // yield stream::iter(output.into_iter());
                if done {
                    while requests.load(Ordering::Relaxed) > 0 {
                        tracing::info!("Waiting for requests to complete {}", requests.load(Ordering::Relaxed));
                        let event = rx.recv().await;
                        requests.fetch_sub(1, Ordering::SeqCst);
                        // let mut output = Vec::new();
                        // output.push(event.unwrap());
                        // yield stream::iter(output.into_iter());
                        yield event.unwrap();
                    }

                    let client = Arc::clone(&*PIPE_CLIENT);
                    client.drop_inner().await;
                    break;
                }
              }
            }
            // .flatten(),
        )
    }
}
