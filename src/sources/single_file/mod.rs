use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::trace::current_span;
use tracing::{error, info};

use crate::{
    config::{log_schema, DataType, Output, SourceConfig, SourceContext, SourceDescription},
    encoding_transcode::{Decoder, Encoder},
    event::{BatchNotifier, Event, LogEvent},
    line_agg::{self, LineAgg},
    shutdown::ShutdownSignal,
    sources::util::{EncodingConfig, MultilineConfig},
    SourceSender,
};
mod file_checkpoint;
use regex::bytes::Regex;
mod watch_file;
use file_reader::Line;
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
mod file_reader;
use futures_util::{FutureExt, TryFutureExt};

use self::file_checkpoint::FinalizerEntry;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct SingleFileConfig {
    pub include: PathBuf,
    pub save_path: Option<PathBuf>,
    pub line_delimiter: String,
    pub encoding: Option<EncodingConfig>,
    pub message_start_indicator: Option<String>,
    pub multiline: Option<MultilineConfig>,
    pub max_inflight: usize,
}

impl Default for SingleFileConfig {
    fn default() -> Self {
        Self {
            include: PathBuf::new(),
            save_path: None,
            line_delimiter: "\n".to_string(),
            encoding: None,
            message_start_indicator: None,
            multiline: None,
            max_inflight: 50,
        }
    }
}

impl SingleFileConfig {}

inventory::submit! {
    SourceDescription::new::<SingleFileConfig>("geneva_file")
}

impl_generate_config_from_default!(SingleFileConfig);

fn wrap_with_line_agg(
    rx: impl Stream<Item = Line> + Send + std::marker::Unpin + 'static,
    config: line_agg::Config,
) -> Box<dyn Stream<Item = Line> + Send + std::marker::Unpin + 'static> {
    let logic = line_agg::Logic::new(config);
    Box::new(
        LineAgg::new(rx.map(|line| ("", line.data, line.number)), logic)
            .map(|(_file, data, number)| Line { data, number }),
    )
}

fn create_event(line: Bytes) -> Event {
    let mut event = LogEvent::from(line);

    // Add source type
    event.insert(log_schema().source_type_key(), Bytes::from("file"));

    event.into()
}

pub fn file_source(
    config: &SingleFileConfig,
    shutdown: ShutdownSignal,
    mut out: SourceSender,
) -> super::Source {
    let encoding_charset = config.encoding.clone().map(|e| e.charset);
    let shutdown = shutdown.shared();
    let (completion_tx, mut completion_rx) = mpsc::channel::<()>(config.max_inflight);
    let file_checkpoint =
        file_checkpoint::FileCheckpoint::new(shutdown.clone(), completion_tx,&config.include).unwrap();
    // let (sender, mut new_entries) = mpsc::unbounded_channel();

    // let finalizer = if acknowledgements {
    //     Some(OrderedFinalizer::new(
    //         shutdown.clone(),
    //         move |entry: FinalizerEntry| {
    //             {
    //                 let mut guard = file_writer.lock().unwrap();
    //                 guard.write_pos(entry.offset).unwrap();
    //             }

    //             let completion_tx = completion_tx.clone();
    //             tokio::spawn(async move {
    //                 let _t = completion_tx.send(()).await;
    //             });
    //         },
    //     ))
    // } else {
    //     None
    // };

    let multiline_config = config.multiline.clone();
    let message_start_indicator = config.message_start_indicator.clone();

    // if file encoding is specified, need to convert the line delimiter (present as utf8)
    // to the specified encoding, so that delimiter-based line splitting can work properly
    let line_delimiter_as_bytes = match encoding_charset {
        Some(e) => Encoder::new(e).encode_from_utf8(&config.line_delimiter),
        None => Bytes::from(config.line_delimiter.clone()),
    };

    let max_inflight = config.max_inflight;
    let path = config.include.clone();
    Box::pin(async move {
        let mut encoding_decoder = encoding_charset.map(Decoder::new);
        let (tx, rx) = futures::channel::mpsc::channel::<Vec<Line>>(2);
        let rx = rx
            .map(futures::stream::iter)
            .flatten()
            .map(move |mut line| {
                // transcode each line from the file's encoding charset to utf8
                line.data = match encoding_decoder.as_mut() {
                    Some(d) => d.decode_to_utf8(line.data),
                    None => line.data,
                };
                line
            });

        let messages: Box<dyn Stream<Item = Line> + Send + std::marker::Unpin> =
            if let Some(ref multiline_config) = multiline_config {
                wrap_with_line_agg(
                    rx,
                    multiline_config.try_into().unwrap(), // validated in build
                )
            } else if let Some(msi) = message_start_indicator {
                wrap_with_line_agg(
                    rx,
                    line_agg::Config::for_legacy(
                        Regex::new(&msi).unwrap(), // validated in build
                        100,
                    ),
                )
            } else {
                Box::new(rx)
            };

        // Once file server ends this will run until it has finished processing remaining
        // logs in the queue.
        let span = current_span();
        let span2 = span.clone();
        let shutdown = shutdown.clone();
        let sender_event = file_checkpoint.sender();
        let mut messages = messages
            .filter(move |line| {
                futures::future::ready(!file_checkpoint.checkpoints.contains(&line.number))
            })
            .map(move |line| {
                let _enter = span2.enter();
                let mut event = create_event(line.data);
                let (batch, receiver) = BatchNotifier::new_with_receiver();
                event = event.with_batch_notifier(&batch);
                let entry = FinalizerEntry {
                    offset: line.number,
                };
                let _ = sender_event.send((receiver, entry));
                event
            });

        tokio::spawn(async move {
            let mut remaining = max_inflight;
            loop {
                tokio::select! {
                    biased;

                    result = messages.next(), if remaining > 0 => {
                        if let Some(result) = result {
                            let _ = out.send(result).await;
                        } else {
                            info!("File source finished sending messages");
                            break;
                        }
                        remaining -= 1;
                    }
                    result = completion_rx.recv()  => {
                        if result.is_some() {
                            remaining += 1;
                        }
                    }
                    _shutdown = shutdown.clone() => {
                        info!("File source shutdown");
                        break;
                    }
                }
            }
        });
        let span = info_span!("file_server");
        spawn_blocking(move || {
            let _enter = span.enter();
            let watcher = file_reader::FileReader::new(
                &path,
                bytesize::kib(100u64) as usize,
                line_delimiter_as_bytes,
            );
            let result = watch_file::run(watcher.unwrap(), tx);
            // Panic if we encounter any error originating from the file server.
            // We're at the `spawn_blocking` call, the panic will be caught and
            // passed to the `JoinHandle` error, similar to the usual threads.
            result.unwrap();
        })
        .map_err(|error| error!(message="File server unexpectedly stopped.", %error))
        .await
    })
}

#[async_trait::async_trait]
#[typetag::serde(name = "geneva_file")]
impl SourceConfig for SingleFileConfig {
    async fn build(&self, cx: SourceContext) -> crate::Result<super::Source> {
        Ok(file_source(self, cx.shutdown, cx.out))
    }

    fn outputs(&self) -> Vec<Output> {
        vec![Output::default(DataType::Log)]
    }

    fn source_type(&self) -> &'static str {
        "geneva_file"
    }
}
