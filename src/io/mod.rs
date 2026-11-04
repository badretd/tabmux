//! Input readers, output writers, and safe filesystem helpers.

pub mod encoding;
pub mod safe;

use std::io::{self, Read, Write};

use crate::error::{Error, Result};

pub use encoding::{DecodedText, decode_text};
pub use safe::SafeWriter;

/// Read a file or stdin (`-`) as raw bytes.
pub fn read_input(path: &str) -> Result<Vec<u8>> {
    if path == "-" {
        let mut buffer = Vec::new();
        io::stdin().lock().read_to_end(&mut buffer)?;
        Ok(buffer)
    } else {
        Ok(std::fs::read(path)?)
    }
}

/// Write bytes to stdout, treating a closed pipe as a clean exit.
pub fn write_stdout(bytes: &[u8]) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match stdout.write_all(bytes) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
        Err(error) => return Err(Error::Io(error)),
    }
    match stdout.flush() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::Io(error)),
    }
}
