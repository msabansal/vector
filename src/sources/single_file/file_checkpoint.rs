use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use futures_util::{future::Shared, stream::FuturesUnordered, Future, FutureExt, StreamExt};
use tokio::sync::{
    mpsc::{self, Sender, UnboundedSender},
    Mutex,
};
use vector_core::event::{BatchStatus, BatchStatusReceiver};

use crate::{shutdown::ShutdownSignal, Result};
use bytes::Bytes;
use tracing::error;

use super::file_reader::FileReader;

pub struct FileCheckpoint {
    pub checkpoints: HashSet<usize>,
    sender: UnboundedSender<(BatchStatusReceiver, FinalizerEntry)>,
}

impl FileCheckpoint {
    fn read_restore_points(file_path: &PathBuf) -> Result<HashSet<usize>> {
        let mut file = FileReader::new(file_path, 100, Bytes::from("\n"))?;

        let mut set = HashSet::<usize>::new();
        while let Some(line) = file.read_line()? {
            set.insert(std::str::from_utf8(&line.data)?.parse::<usize>()?);
        }
        Ok(set)
    }

    pub fn new(
        shutdown: Shared<ShutdownSignal>,
        completion_tx: Sender<()>,
        path: &PathBuf,
    ) -> crate::Result<Self> {
        let mut restore_path = path.clone();
        let mut file_name = path.file_name().unwrap().to_os_string();
        file_name.push(".checkpoint");
        restore_path.set_file_name(file_name);

        let checkpoints = match FileCheckpoint::read_restore_points(&restore_path) {
            Ok(set) => set,
            Err(e) => {
                error!("Failed to read restore points: {}", e);
                HashSet::new()
            }
        };
        let (sender, mut new_entries) = mpsc::unbounded_channel();

        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(restore_path)?;
        let writer = io::BufWriter::new(file);
        let writer = Mutex::new(writer);
        let writer = Arc::new(Inner {
            writer,
            completion_tx,
        });


        tokio::spawn(async move {
            let mut status_receivers = FuturesUnordered::default();
            loop {
                tokio::select! {
                    _ = shutdown.clone() => {
                        break;
                    },
                    new_entry = new_entries.recv() => match new_entry {
                        Some((receiver, entry)) => {
                            status_receivers.push(FinalizerFuture {
                                receiver,
                                entry: Some(entry),
                            });
                        }
                        None => break,
                    },
                    finished = status_receivers.next(), if !status_receivers.is_empty() => match finished {
                        Some((status, entry)) => writer.entry_done(status, entry).await,
                        // The is_empty guard above prevents this from being reachable.
                        None => unreachable!(),
                    }
                }
            }

            while let Some((status, entry)) = status_receivers.next().await {
                info!("Waiting for final set of entries");
                writer.entry_done(status, entry).await;
            }
            // drop(t_shutdown);
        });

        Ok(Self {
            checkpoints,
            sender,
        })
    }

    pub(crate) fn sender(&self) -> UnboundedSender<(BatchStatusReceiver, FinalizerEntry)> {
        self.sender.clone()
    }
}

struct Inner {
    writer: Mutex<io::BufWriter<fs::File>>,
    completion_tx: Sender<()>,
}

impl Inner {
    pub async fn entry_done(&self, status: BatchStatus, entry: FinalizerEntry) {
        // Post completion only in case of delieverable status.
        if status == BatchStatus::Delivered {
            let _ = self.write_pos(entry.offset).await;
        }
        let _ = self.completion_tx.send(()).await;
    }

    pub async fn write_pos(&self, line_number: usize) -> crate::Result<()> {
        let mut guard = self.writer.lock().await;
        let line_number = line_number.to_string();
        guard.write(line_number.as_bytes())?;
        guard.write("\n".as_bytes())?;
        guard.flush()?;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct FinalizerEntry {
    pub(crate) offset: usize,
}

#[pin_project::pin_project]
struct FinalizerFuture<T> {
    receiver: BatchStatusReceiver,
    entry: Option<T>,
}

impl<T> Future for FinalizerFuture<T> {
    type Output = (<BatchStatusReceiver as Future>::Output, T);
    fn poll(mut self: Pin<&mut Self>, ctx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let status = futures::ready!(self.receiver.poll_unpin(ctx));
        // The use of this above in a `FuturesOrdered` will only take
        // this once before dropping the future.
        Poll::Ready((status, self.entry.take().unwrap_or_else(|| unreachable!())))
    }
}
