use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use std::path::PathBuf;

use futures_util::FutureExt;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{error, trace};

use super::util::{finalizer::OrderedFinalizer, EncodingConfig};

use file_source::{file_watcher::FileWatcher, FileFingerprint, Line, ReadFrom};

use crate::internal_events::FileEventsReceived;
use crate::{
    config::{log_schema, DataType, Output, SourceConfig, SourceContext, SourceDescription},
    encoding_transcode::Encoder,
    event::{BatchNotifier, Event, LogEvent},
    shutdown::ShutdownSignal,
    SourceSender,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct SingleFileConfig {
    pub include: PathBuf,
    pub save_path: Option<PathBuf>,
    pub line_delimiter: String,
    pub encoding: Option<EncodingConfig>,
}

impl Default for SingleFileConfig {
    fn default() -> Self {
        Self {
            include: PathBuf::new(),
            save_path: None,
            line_delimiter: "\n".to_string(),
            encoding: None,
        }
    }
}
#[derive(Debug, PartialEq, Snafu)]
pub enum SingleFileConfigError {
    #[snafu(display("A non-empty list of lines is required for the shuffle format"))]
    ShuffleSingleFileItemsEmpty,
}

impl SingleFileConfig {}

inventory::submit! {
    SourceDescription::new::<SingleFileConfig>("single_file")
}

impl_generate_config_from_default!(SingleFileConfig);

pub(crate) struct FinalizerEntry {
    pub(crate) offset: u64,
}

fn create_event(line: Bytes, file: String) -> Event {
    emit!(&FileEventsReceived {
        count: 1,
        file: &file,
        byte_size: line.len(),
    });

    let mut event = LogEvent::from(line);

    // Add source type
    event.insert(log_schema().source_type_key(), Bytes::from("file"));

    event.into()
}
pub fn file_source(
    config: &SingleFileConfig,
    shutdown: ShutdownSignal,
    out: SourceSender,
) -> super::Source {
    let max_read_bytes = 2048;
    let out = Arc::new(Mutex::new(out));

    let encoding_charset = config.encoding.clone().map(|e| e.charset);

    // if file encoding is specified, need to convert the line delimiter (present as utf8)
    // to the specified encoding, so that delimiter-based line splitting can work properly
    let line_delimiter_as_bytes = match encoding_charset {
        Some(e) => Encoder::new(e).encode_from_utf8(&config.line_delimiter),
        None => Bytes::from(config.line_delimiter.clone()),
    };

    let path = config.include.clone();
   Box::pin(async move {
        let watcher = FileWatcher::new(
            path,
            ReadFrom::Beginning,
            None,
            bytesize::kib(100u64) as usize,
            line_delimiter_as_bytes,
        );


        if watcher.is_err() {
            error!(
                "Failed to create file watcher for file source: {:?}",
                watcher.err()
            );
            return Ok(());
        }

        let mut watcher = watcher.unwrap();
        let mut file_done = false;
        while !file_done {
            let mut lines = Vec::new();
            let mut bytes_read = 0;
            loop {
                match watcher.read_line() {
                    Ok(Some(line)) => {
                        bytes_read += line.len();

                        lines.push(Line {
                            text: line,
                            filename: watcher.path.to_str().expect("not a valid path").to_owned(),
                            file_id: FileFingerprint::Unknown(0),
                            offset: watcher.get_file_position(),
                        });

                        if bytes_read >= max_read_bytes {
                            break;
                        }
                    }
                    Ok(None) => {
                        file_done = true;
                        break;
                    }
                    Err(e) => {
                        error!(%e, "Error reading file");
                        file_done = true;
                        break;
                    }
                }
            }
            let cloned = shutdown.clone();
            let finalizer = OrderedFinalizer::new(cloned.shared(), move |entry: FinalizerEntry| {
                tracing::info!("Done entry {}", entry.offset);
            });

            let mut messages = futures::stream::iter(lines).map(move |line| {
                let mut event = create_event(line.text, line.filename);
                let (batch, receiver) = BatchNotifier::new_with_receiver();
                event = event.with_batch_notifier(&batch);
                let entry = FinalizerEntry {
                    offset: line.offset,
                };
                finalizer.add(entry, receiver);
                event
            });
            trace!("Test");

            let out = out.clone();
            tokio::spawn(async move {
                let mut guard = out.lock().await;
                guard.send_all(&mut messages).await
            });
        }

        Ok(())
    })
}

#[async_trait::async_trait]
#[typetag::serde(name = "single_file")]
impl SourceConfig for SingleFileConfig {
    async fn build(&self, cx: SourceContext) -> crate::Result<super::Source> {
        Ok(file_source(self, cx.shutdown, cx.out))
        // let start = time::Instant::now();
        // let to_send = std::mem::take(&mut lines);
        // let mut stream = stream::once(futures::future::ok(to_send));
        // let result = self.handle.block_on(chans.send_all(&mut stream));
        // match result {
        //     Ok(()) => {}
        //     Err(error) => {
        //         error!(message = "Output channel closed.", %error);
        //         return Err(error);
        //     }
        // }
    }

    fn outputs(&self) -> Vec<Output> {
        vec![Output::default(DataType::Log)]
    }

    fn source_type(&self) -> &'static str {
        "single_file"
    }
}
