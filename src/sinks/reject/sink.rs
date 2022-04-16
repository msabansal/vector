use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use vector_core::{
    event::EventStatus,
};

use crate::{
    event::Event,
    sinks::util::{
        StreamSink,
    },
};

pub struct WriterSink {}

#[async_trait]
impl StreamSink<Event> for WriterSink
{
    async fn run(mut self: Box<Self>, mut input: BoxStream<'_, Event>) -> Result<(), ()> {
        while let Some(mut event) = input.next().await {
            event.metadata_mut().update_status(EventStatus::Rejected);
        }
        Ok(())
    }
}
