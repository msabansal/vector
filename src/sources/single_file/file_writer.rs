use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

pub struct FileWriter {
    writer: io::BufWriter<fs::File>,
}

impl FileWriter {
    pub fn new(path: &PathBuf) -> crate::Result<Self> {
        let file = OpenOptions::new().append(true).create(true).open(path)?;
        let writer = io::BufWriter::new(file);

        Ok(Self {
            writer,
        })
    }

    pub fn write_pos(&mut self, line_number: usize) -> crate::Result<()> {
        let line_number = line_number.to_string();
        self.writer.write(line_number.as_bytes())?;
        self.writer.write("\n".as_bytes())?;
        self.writer.flush();
        Ok(())
    }
}
