use tracing::{error};

use crate::sources::single_file::Line;

use futures::{
    stream, Sink, SinkExt,
};

use super::file_reader::FileReader;

pub fn run<C>(
    mut watcher: FileReader,
    mut chans: C,
) -> Result<(), <C as Sink<Vec<Line>>>::Error>
where
    C: Sink<Vec<Line>> + Unpin,
    <C as Sink<Vec<Line>>>::Error: std::error::Error,
{
    let max_read_bytes = 2048;
    let mut file_done = false;
    while !file_done {
        let mut lines = Vec::new();
        let mut bytes_read = 0;
        loop {
            match watcher.read_line() {
                Ok(Some(line)) => {
                    bytes_read += line.data.len();

                    lines.push(line);

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

        let to_send = std::mem::take(&mut lines);
        let mut stream = stream::once(futures::future::ok(to_send));
        let result = tokio::runtime::Handle::current().block_on(chans.send_all(&mut stream));
        match result {
            Ok(()) => {}
            Err(error) => {
                error!(message = "Output channel closed.", %error);
                return Err(error);
            }
        };
    }
    Ok(())
}
