use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct RecordWriter {
    pub path: PathBuf,
    writer: BufWriter<File>,
}

impl RecordWriter {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
        })
    }

    pub fn record_frame(&mut self, t: f64, bytes: &[u8]) -> std::io::Result<()> {
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(hex, "{:02x}", b);
        }
        writeln!(self.writer, r#"{{"t": {:.6}, "hex": "{}"}}"#, t, hex)?;
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_record_writer() {
        let dir = std::env::temp_dir().join(format!("test_record_{}", std::process::id()));
        let file_path = dir.join("test_capture.jsonl");
        {
            let mut writer = RecordWriter::open(&file_path).unwrap();
            writer.record_frame(1700000000.123456, &[0x02, 0x10, 0x00]).unwrap();
            writer.record_frame(1700000001.0, &[0x06, 0x10]).unwrap();
        }

        let mut content = String::new();
        File::open(&file_path).unwrap().read_to_string(&mut content).unwrap();
        assert!(content.contains(r#"{"t": 1700000000.123456, "hex": "021000"}"#));
        assert!(content.contains(r#"{"t": 1700000001.000000, "hex": "0610"}"#));

        let _ = std::fs::remove_dir_all(dir);
    }
}
