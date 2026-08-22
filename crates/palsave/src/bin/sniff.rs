use std::env;
use std::fs;
use std::process::ExitCode;

struct Header {
    uncompressed_len: u32,
    compressed_len: u32,
    magic: [u8; 3],
    save_type: u8,
}

fn read_header(bytes: &[u8], base: usize) -> Option<Header> {
    if bytes.len() < base + 12 {
        return None;
    }
    let uncompressed_len = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap());
    let compressed_len = u32::from_le_bytes(bytes[base + 4..base + 8].try_into().unwrap());
    let magic: [u8; 3] = bytes[base + 8..base + 11].try_into().unwrap();
    let save_type = bytes[base + 11];
    Some(Header {
        uncompressed_len,
        compressed_len,
        magic,
        save_type,
    })
}

fn magic_name(magic: &[u8; 3]) -> &'static str {
    match magic {
        b"PlZ" => "PlZ (zlib)",
        b"PlM" => "PlM (Oodle Mermaid)",
        b"CNK" => "CNK (Game Pass / WGS wrapper)",
        _ => "unknown",
    }
}

fn print_header(label: &str, base: usize, h: &Header, file_len: usize) {
    println!("{label} (offset {base}):");
    println!("  uncompressed_len: {}", h.uncompressed_len);
    println!("  compressed_len:   {}", h.compressed_len);
    println!(
        "  magic:            {:?} -> {}",
        String::from_utf8_lossy(&h.magic),
        magic_name(&h.magic)
    );
    println!("  save_type:        0x{:02X}", h.save_type);

    let payload_start = base + 12;
    let consistent = h.compressed_len as usize == file_len.saturating_sub(payload_start)
        || (h.compressed_len as usize) <= file_len.saturating_sub(payload_start);
    println!(
        "  self-consistent:  {}",
        if consistent {
            "yes"
        } else {
            "NO — sizes don't fit file length"
        }
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: sniff <path-to-.sav>");
        return ExitCode::FAILURE;
    };

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    if bytes.len() < 32 {
        eprintln!("file too short ({} bytes) to sniff", bytes.len());
        return ExitCode::FAILURE;
    }

    let hex32: String = bytes[..32].iter().map(|b| format!("{b:02x} ")).collect();
    println!("first 32 bytes: {}", hex32.trim_end());
    println!();

    let Some(outer) = read_header(&bytes, 0) else {
        eprintln!("could not read outer header");
        return ExitCode::FAILURE;
    };
    print_header("outer header", 0, &outer, bytes.len());
    println!();

    let (container, save_type, uncompressed_len, compressed_len) = if &outer.magic == b"CNK" {
        match read_header(&bytes, 12) {
            Some(inner) => {
                print_header("inner header (CNK real payload)", 12, &inner, bytes.len());
                println!();
                (
                    format!("CNK-wrapped {}", magic_name(&inner.magic)),
                    inner.save_type,
                    inner.uncompressed_len,
                    inner.compressed_len,
                )
            }
            None => {
                eprintln!("CNK magic seen but file too short for inner header");
                return ExitCode::FAILURE;
            }
        }
    } else {
        (
            magic_name(&outer.magic).to_string(),
            outer.save_type,
            outer.uncompressed_len,
            outer.compressed_len,
        )
    };

    println!("verdict:");
    println!("  container:         {container}");
    println!("  save_type:         0x{save_type:02X}");
    println!("  uncompressed_len:  {uncompressed_len}");
    println!("  compressed_len:    {compressed_len}");
    println!("  file size:         {}", bytes.len());

    ExitCode::SUCCESS
}
