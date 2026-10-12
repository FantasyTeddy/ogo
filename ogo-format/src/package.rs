//! `.ogo` game packages: an uncompressed ustar archive with flat names, holding `game.toml`,
//! `game.bin` and optionally `icon.rgb565`.
//!
//! Own reader and writer instead of the `tar` crate: on the ESP32 it pulls in `filetime`, which
//! needs Unix calls ESP-IDF lacks. They handle exactly what packages need: regular files with
//! names up to 100 bytes.

use std::io::{self, Read, Write};

/// Files a package may contain; anything else is skipped when reading.
pub const FILES: [&str; 3] = ["game.toml", "game.bin", "icon.rgb565"];
/// Files a package must contain.
pub const REQUIRED: [&str; 2] = ["game.toml", "game.bin"];

const BLOCK: usize = 512;

/// Writes a package with `files` (name, content), in this order. `mtime` is in Unix seconds.
pub fn write(mut out: impl Write, files: &[(&str, &[u8])], mtime: u64) -> io::Result<()> {
    for (name, content) in files {
        out.write_all(&header(name, content.len() as u64, mtime)?)?;
        out.write_all(content)?;
        out.write_all(&[0; BLOCK][..padding(content.len() as u64)])?;
    }
    // The end of the archive: two empty blocks.
    out.write_all(&[0; 2 * BLOCK])
}

/// Reads a package, calling `file` with the name and contents of each file in [`FILES`]. Folders
/// in names are ignored. Returns the names found; fails if one of [`REQUIRED`] is missing.
pub fn read<R: Read>(
    mut input: R,
    mut file: impl FnMut(&str, &mut dyn Read) -> io::Result<()>,
) -> io::Result<Vec<String>> {
    let mut header = [0u8; BLOCK];
    let mut found = Vec::new();
    loop {
        if !read_block(&mut input, &mut header)? || header.iter().all(|&b| b == 0) {
            break;
        }
        let name = field_str(&header[0..100]);
        let size = octal(&header[124..136]).ok_or_else(|| invalid("bad tar header"))?;
        let regular = header[156] == b'0' || header[156] == 0;
        // Name without leading "./" or folders.
        let name = name.rsplit('/').next().unwrap_or_default().to_owned();

        let mut data = (&mut input).take(size);
        if regular && FILES.contains(&name.as_str()) {
            file(&name, &mut data)?;
            found.push(name);
        }
        // Whatever `file` didn't read, and the padding to a whole block.
        io::copy(&mut data, &mut io::sink())?;
        if data.limit() > 0 {
            return Err(invalid("truncated"));
        }
        let padding = padding(size) as u64;
        if io::copy(&mut (&mut input).take(padding), &mut io::sink())? < padding {
            return Err(invalid("truncated"));
        }
    }
    for required in REQUIRED {
        if !found.iter().any(|name| name == required) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the package has no {required}"),
            ));
        }
    }
    Ok(found)
}

fn header(name: &str, size: u64, mtime: u64) -> io::Result<[u8; BLOCK]> {
    if name.len() >= 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("file name too long: {name}"),
        ));
    }
    let mut header = [0u8; BLOCK];
    header[..name.len()].copy_from_slice(name.as_bytes());
    put_octal(&mut header[100..108], 0o644); // mode
    put_octal(&mut header[108..116], 0); // uid
    put_octal(&mut header[116..124], 0); // gid
    put_octal(&mut header[124..136], size);
    put_octal(&mut header[136..148], mtime);
    header[156] = b'0'; // regular file
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    // The checksum is computed with its own field set to spaces.
    header[148..156].fill(b' ');
    let checksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    Ok(header)
}

/// Writes `value` as zero-padded octal digits, ending in NUL.
fn put_octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    field[..digits].copy_from_slice(format!("{value:0digits$o}").as_bytes());
    field[digits] = 0;
}

/// Bytes after `size` bytes of data up to the next whole block.
fn padding(size: u64) -> usize {
    ((BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64) as usize
}

/// Reads one block; `false` at the end of the input.
fn read_block(input: &mut impl Read, block: &mut [u8; BLOCK]) -> io::Result<bool> {
    let mut filled = 0;
    while filled < BLOCK {
        let count = input.read(&mut block[filled..])?;
        if count == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err(invalid("truncated"));
        }
        filled += count;
    }
    Ok(true)
}

fn field_str(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn octal(field: &[u8]) -> Option<u64> {
    u64::from_str_radix(field_str(field).trim(), 8).ok()
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("not a valid .ogo package ({what})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a package into (name, content) pairs.
    fn read_all(package: &[u8]) -> io::Result<Vec<(String, Vec<u8>)>> {
        let mut files = Vec::new();
        read(package, |name, data| {
            let mut content = Vec::new();
            data.read_to_end(&mut content)?;
            files.push((name.to_owned(), content));
            Ok(())
        })?;
        Ok(files)
    }

    fn package(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        write(&mut out, files, 1_700_000_000).unwrap();
        out
    }

    #[test]
    fn round_trip() {
        let bin: Vec<u8> = (0..2000u32).map(|i| i as u8).collect();
        let files: [(&str, &[u8]); 3] = [
            ("game.toml", b"id = \"x\"\n"),
            ("game.bin", &bin),
            ("icon.rgb565", &[7; 8192]),
        ];
        let package = package(&files);
        assert_eq!(package.len() % BLOCK, 0);
        let read = read_all(&package).unwrap();
        assert_eq!(read.len(), 3);
        for ((name, content), (expected_name, expected)) in read.iter().zip(files) {
            assert_eq!(name, expected_name);
            assert_eq!(content, expected);
        }
    }

    #[test]
    fn headers_are_valid_ustar() {
        let package = package(&[("game.toml", b"x"), ("game.bin", b"y")]);
        let header = &package[..BLOCK];
        assert_eq!(&header[257..265], b"ustar\x0000");
        let stored = octal(&header[148..156]).unwrap();
        let mut blank = header.to_vec();
        blank[148..156].fill(b' ');
        assert_eq!(stored, blank.iter().map(|&b| u64::from(b)).sum::<u64>());
    }

    #[test]
    fn skips_other_files_and_strips_folders() {
        let package = package(&[
            ("readme.txt", b"hello"),
            ("./game.toml", b"t"),
            ("dir/game.bin", b"b"),
        ]);
        let read = read_all(&package).unwrap();
        let names: Vec<&str> = read.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["game.toml", "game.bin"]);
    }

    #[test]
    fn callback_may_leave_data_unread() {
        let package = package(&[("game.bin", &[1; 1500]), ("game.toml", b"t")]);
        let found = read(&package[..], |_, _| Ok(())).unwrap();
        assert_eq!(found, ["game.bin", "game.toml"]);
    }

    #[test]
    fn missing_required_file() {
        let error = read_all(&package(&[("game.toml", b"t")])).unwrap_err();
        assert_eq!(error.to_string(), "the package has no game.bin");
    }

    #[test]
    fn truncated_package() {
        let package = package(&[("game.toml", b"t"), ("game.bin", &[1; 3000])]);
        for cut in [100, BLOCK + 10, 3 * BLOCK] {
            let error = read_all(&package[..cut]).unwrap_err();
            assert!(
                error.to_string().contains("truncated"),
                "cut at {cut}: {error}"
            );
        }
    }

    #[test]
    fn not_a_tar_file() {
        let error = read_all(&[b'x'; 2 * BLOCK]).unwrap_err();
        assert!(error.to_string().contains("bad tar header"), "{error}");
    }

    #[test]
    fn empty_input_has_no_files() {
        assert_eq!(
            read_all(&[]).unwrap_err().to_string(),
            "the package has no game.toml"
        );
    }
}
