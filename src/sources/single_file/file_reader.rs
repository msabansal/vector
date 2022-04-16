use std::{
    fs,
    io::{self, BufRead, Seek},
    path::PathBuf,
};

use bytes::{Bytes, BytesMut};

use file_source::{
    buffer::read_until_with_max_size, FilePosition,
};

#[derive(Debug)]
pub struct Line {
    pub data: Bytes,
    pub number: usize,
}

pub struct FileReader {
    reader: Box<dyn BufRead>,
    file_position: FilePosition,
    max_line_bytes: usize,
    line_delimiter: Bytes,
    buf: BytesMut,
    line_number: usize,
}

impl FileReader {
    /// Create a new `FileReader`
    ///
    /// The input path will be used by `FileReader` to prime its state
    /// machine. A `FileReader` tracks _only one_ file. This function returns
    /// None if the path does not exist or is not readable by the current process.
    pub fn new(
        path: &PathBuf,
        max_line_bytes: usize,
        line_delimiter: Bytes,
    ) -> Result<FileReader, io::Error> {
        let f = fs::File::open(path)?;
        let mut reader = io::BufReader::new(f);

        let file_position = reader.seek(io::SeekFrom::Start(0)).unwrap();
        let reader = Box::new(reader);

        Ok(FileReader {
            reader,
            file_position,
            max_line_bytes,
            line_delimiter,
            buf: BytesMut::new(),
            line_number: 0,
        })
    }

    /// Read a single line from the underlying file
    ///
    pub fn read_line(&mut self) -> io::Result<Option<Line>> {
        let reader = &mut self.reader;
        let file_position = &mut self.file_position;
        match read_until_with_max_size(
            reader,
            file_position,
            self.line_delimiter.as_ref(),
            &mut self.buf,
            self.max_line_bytes,
        ) {
            Ok(Some(_)) => {
                self.line_number += 1;
                Ok(Some(Line {
                    data: self.buf.split().freeze(),
                    number: self.line_number,
                }))
            }
            Ok(None) => {
                Ok(None)
            }
            Err(e) => {
                Err(e)
            }
        }
    }
}
