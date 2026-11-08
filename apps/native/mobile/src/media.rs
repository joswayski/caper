//! File-oriented bridge: native hosts upload the resulting file with their own networking.
use crate::metadata::{self, Outcome, Patched, Refusal};
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

/// 0 = cleaned output written; 1 = unsupported; 2 = malformed; 3 = I/O error.
/// The destination must not exist. Never modifies the source or replaces another file.
pub fn clean_metadata(source: &str, destination: &str, mime: &str) -> i32 {
    fn run(source: &Path, destination: &Path, mime: &str) -> io::Result<i32> {
        let mut input = File::open(source)?;
        let len = input.metadata()?.len();
        let edits = match metadata::clean(&mut input, len, mime)? {
            Outcome::Clean(edits) => edits,
            Outcome::Refused(Refusal::Unsupported) => return Ok(1),
            Outcome::Refused(Refusal::Malformed) => return Ok(2),
        };
        let mut reader = Patched::new(input, len, edits)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        if let Err(error) = io::copy(&mut reader, &mut output).and_then(|_| output.sync_all()) {
            drop(output);
            let _ = std::fs::remove_file(destination);
            return Err(error);
        }
        Ok(0)
    }
    run(Path::new(source), Path::new(destination), mime).unwrap_or(3)
}
