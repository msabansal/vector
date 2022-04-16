use futures::{future, FutureExt};
use serde::{Deserialize, Serialize};
use tokio::io;

use crate::{
    config::{GenerateConfig, Input, SinkConfig, SinkContext},
    sinks::{
        reject::sink::WriterSink,
        util::encoding::{EncodingConfig, StandardEncodings},
        Healthcheck, VectorSink,
    },
};

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RejectSinkConfig {
}

impl GenerateConfig for RejectSinkConfig {
    fn generate_config() -> toml::Value {
        toml::Value::try_from(Self {
        })
        .unwrap()
    }
}

#[async_trait::async_trait]
#[typetag::serde(name = "reject")]
impl SinkConfig for RejectSinkConfig {
    async fn build(&self, cx: SinkContext) -> crate::Result<(VectorSink, Healthcheck)> {

        let sink: VectorSink = VectorSink::from_event_streamsink(WriterSink {
        });

        Ok((sink, future::ok(()).boxed()))
    }

    fn input(&self) -> Input {
        Input::any()
    }

    fn sink_type(&self) -> &'static str {
        "reject"
    }
}
