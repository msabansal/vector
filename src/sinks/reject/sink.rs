use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use tokio::{io, io::AsyncWriteExt};
use vector_core::{
    buffers::Acker,
    internal_event::{BytesSent, EventsSent},
    ByteSizeOf,
};

use crate::{
    event::Event,
    internal_events::ConsoleEventProcessed,
    sinks::util::{
        encoding::{Encoder, EncodingConfig, StandardEncodings},
        StreamSink,
    },
};

pub struct WriterSink<T> {
}

#[async_trait]
impl<T> StreamSink<Event> for WriterSink<T>
where
    T: io::AsyncWrite + Send + Sync + Unpin,
{
    async fn run(mut self: Box<Self>, mut input: BoxStream<'_, Event>) -> Result<(), ()> {
        while let Some(mut event) = input.next().await {
        }
        Ok(())
    }
}

fn encode_event(event: Event, encoding: &EncodingConfig<StandardEncodings>) -> Option<String> {
    encoding.encode_input_to_string(event).ok()
}
