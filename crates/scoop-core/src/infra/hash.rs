use std::{fs::File, io::Read};

use anyhow::Context;
use camino::Utf8Path;
use sha2::{Digest, Sha256};

pub fn sha256_file(path: &Utf8Path) -> anyhow::Result<String> {
    let mut file = File::open(path).with_context(|| format!("failed to open {}", path))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path))?;
        if read == 0 {
            break;
        }

        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }

    Ok(output)
}
