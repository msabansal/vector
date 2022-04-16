mod config;
mod sink;

pub use config::{RejectSinkConfig};

use crate::config::SinkDescription;

inventory::submit! {
    SinkDescription::new::<RejectSinkConfig>("reject")
}
