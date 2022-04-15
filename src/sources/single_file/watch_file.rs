use tracing::{error};

use file_source::{file_watcher::FileWatcher, FileFingerprint, Line};

use futures::{
    stream, Sink, SinkExt,
};

pub fn run<C>(
    mut watcher: FileWatcher,
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
