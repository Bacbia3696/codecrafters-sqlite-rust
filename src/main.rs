use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs::File;
use std::io::prelude::*;

/// Read a varint from data starting at offset
/// Returns (value, bytes_read)
fn read_varint(data: &[u8], offset: usize) -> (u64, usize) {
    let mut result: u64 = 0;
    let mut bytes_read = 0;

    for i in 0..9 {
        if offset + i >= data.len() {
            break;
        }
        let byte = data[offset + i];
        bytes_read += 1;

        if i < 8 {
            // First 8 bytes: use lower 7 bits
            result = (result << 7) | ((byte & 0x7F) as u64);

            // If high bit is 0, this is the last byte
            if byte & 0x80 == 0 {
                break;
            }
        } else {
            // 9th byte: use all 8 bits
            result = (result << 8) | (byte as u64);
        }
    }

    (result, bytes_read)
}

#[derive(Parser)]
#[command(name = "sqlite")]
#[command(about = "A SQLite database CLI tool")]
struct Cli {
    /// Path to the SQLite database file
    database: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display database information
    #[command(name = ".dbinfo")]
    Dbinfo,
    #[command(name = ".tables")]
    Tables,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Dbinfo => {
            let mut file = File::open(&cli.database)?;

            // Read the file header (100 bytes) + page header (5 bytes to reach cell count)
            let mut header = [0; 105];
            file.read_exact(&mut header)?;

            // The page size is stored at the 16th byte offset, using 2 bytes in big-endian order
            let page_size = u16::from_be_bytes([header[16], header[17]]);

            // The cell count is at offset 103 (100 byte file header + 3 bytes into page header)
            // It's a 2-byte big-endian value
            let cell_count = u16::from_be_bytes([header[103], header[104]]);

            println!("database page size: {}", page_size);
            println!("number of tables: {}", cell_count);
        }
        Commands::Tables => {
            let mut file = File::open(&cli.database)?;
            let mut header = [0; 100];
            file.read_exact(&mut header)?;
            let page_size = u16::from_be_bytes([header[16], header[17]]) as usize;

            // Read entire page 1
            let mut page1 = vec![0u8; page_size];
            page1[..100].copy_from_slice(&header);
            file.read_exact(&mut page1[100..])?;

            // Get cell count and cell offsets
            let cell_count = u16::from_be_bytes([page1[103], page1[104]]) as usize;
            let mut cell_offsets = Vec::with_capacity(cell_count);
            for i in 0..cell_count {
                let offset = 108 + i * 2;
                let cell_offset = u16::from_be_bytes([page1[offset], page1[offset + 1]]) as usize;
                cell_offsets.push(cell_offset);
            }

            // Parse each cell to extract tbl_name (column 2)
            //sqlite3 test.db ".schema sqlite_schema"
            // CREATE TABLE sqlite_schema (
            //   type text,
            //   name text,
            //   tbl_name text,
            //   rootpage integer,
            //   sql text
            // );
            for cell_offset in cell_offsets {
                let mut offset = cell_offset;

                // Skip payload size and rowid (both varints)
                let (_, bytes) = read_varint(&page1, offset);
                offset += bytes;
                let (_, bytes) = read_varint(&page1, offset);
                offset += bytes;

                // Read header size
                let (header_size, bytes) = read_varint(&page1, offset);
                let header_start = offset;
                offset += bytes;

                // Read serial types for columns 0, 1, 2 (type, name, tbl_name)
                let mut serial_types = Vec::new();
                while offset < header_start + header_size as usize {
                    let (serial_type, bytes) = read_varint(&page1, offset);
                    serial_types.push(serial_type);
                    offset += bytes;
                }

                // Skip column 0 (type) and column 1 (name) - both are TEXT
                // Serial type for TEXT: n >= 13 and odd, length = (n-13)/2
                for i in 0..2 {
                    let st = serial_types[i as usize];
                    if st >= 13 && st % 2 == 1 {
                        let len = ((st - 13) / 2) as usize;
                        offset += len; // Skip this column's data
                    }
                }

                // Read column 2 (tbl_name) - also TEXT
                let st = serial_types[2];
                if st >= 13 && st % 2 == 1 {
                    let len = ((st - 13) / 2) as usize;
                    let tbl_name = String::from_utf8_lossy(&page1[offset..offset + len]);
                    println!("{}", tbl_name);
                }
            }
        }
    }

    Ok(())
}
