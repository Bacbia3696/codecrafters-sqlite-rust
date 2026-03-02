use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs::File;
use std::io::{Read, Seek};

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
            result = (result << 7) | ((byte & 0x7F) as u64);
            if byte & 0x80 == 0 {
                break;
            }
        } else {
            result = (result << 8) | (byte as u64);
        }
    }

    (result, bytes_read)
}

fn read_page_size(header: &[u8]) -> usize {
    u16::from_be_bytes([header[16], header[17]]) as usize
}

fn read_first_page(file: &mut File) -> Result<Vec<u8>> {
    let mut header = [0; 100];
    file.read_exact(&mut header)?;
    let page_size = read_page_size(&header);

    let mut page = vec![0u8; page_size];
    page[..100].copy_from_slice(&header);
    file.read_exact(&mut page[100..])?;
    Ok(page)
}

fn get_cell_offsets(page: &[u8]) -> Vec<usize> {
    let cell_count = u16::from_be_bytes([page[103], page[104]]) as usize;
    (0..cell_count)
        .map(|i| {
            let offset = 108 + i * 2;
            u16::from_be_bytes([page[offset], page[offset + 1]]) as usize
        })
        .collect()
}

/// Returns the size in bytes for a given SQLite serial type.
///
/// SQLite uses a dynamic type system where each value carries its own type information.
/// The serial type tells us both the data type and how many bytes it occupies.
///
/// ## Serial Type Reference
///
/// | Serial Type | Size        | Description                    |
/// |-------------|-------------|--------------------------------|
/// | 0           | 0 bytes     | NULL                           |
/// | 1           | 1 byte      | 8-bit signed integer           |
/// | 2           | 2 bytes     | 16-bit signed integer (BE)     |
/// | 3           | 3 bytes     | 24-bit signed integer (BE)     |
/// | 4           | 4 bytes     | 32-bit signed integer (BE)     |
/// | 5           | 6 bytes     | 48-bit signed integer (BE)     |
/// | 6           | 8 bytes     | 64-bit signed integer (BE)     |
/// | 7           | 8 bytes     | IEEE 754 float (BE)            |
/// | 8           | 0 bytes     | Integer constant 0             |
/// | 9           | 0 bytes     | Integer constant 1             |
/// | ≥12, even   | (n-12)/2    | BLOB of that many bytes        |
/// | ≥13, odd    | (n-13)/2    | TEXT of that many bytes        |
///
/// ## Examples
///
/// ```text
/// Serial type 13 → (13-13)/2 = 0 bytes  → empty string
/// Serial type 15 → (15-13)/2 = 1 byte   → 1-char string
/// Serial type 25 → (25-13)/2 = 6 bytes  → "apples"
/// Serial type 12 → (12-12)/2 = 0 bytes  → empty blob
/// Serial type 14 → (14-12)/2 = 1 byte   → 1-byte blob
/// ```
fn serial_type_size(st: u64) -> usize {
    match st {
        // NULL, or special integers 0 and 1 (stored as constants, no bytes needed)
        0 | 8 | 9 => 0,
        // Fixed-size integers: 1, 2, 3, 4, or 6 bytes
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        // 8-byte values: signed int or float
        6 | 7 => 8,
        // TEXT: odd serial types ≥ 13, length = (st - 13) / 2
        st if st >= 13 && st % 2 == 1 => ((st - 13) / 2) as usize,
        // BLOB: even serial types ≥ 12, length = (st - 12) / 2
        st if st >= 12 && st % 2 == 0 => ((st - 12) / 2) as usize,
        // Reserved or unknown serial types treated as 0 bytes
        _ => 0,
    }
}

#[derive(Parser)]
#[command(name = "sqlite")]
#[command(about = "A SQLite database CLI tool")]
struct Cli {
    database: String,
    #[command(subcommand)]
    command: Option<Commands>,
    query: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(name = ".dbinfo")]
    Dbinfo,
    #[command(name = ".tables")]
    Tables,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Dbinfo) => {
            let mut file = File::open(&cli.database)?;
            let mut header = [0; 105];
            file.read_exact(&mut header)?;

            println!("database page size: {}", read_page_size(&header));
            println!(
                "number of tables: {}",
                u16::from_be_bytes([header[103], header[104]])
            );
        }
        Some(Commands::Tables) => {
            let mut file = File::open(&cli.database)?;
            let page1 = read_first_page(&mut file)?;

            for cell_offset in get_cell_offsets(&page1) {
                let mut offset = cell_offset;

                // Skip payload size and rowid
                let (_, bytes) = read_varint(&page1, offset);
                offset += bytes;
                let (_, bytes) = read_varint(&page1, offset);
                offset += bytes;

                // Read serial types
                let (header_size, bytes) = read_varint(&page1, offset);
                let header_start = offset;
                offset += bytes;

                let mut serial_types = Vec::new();
                while offset < header_start + header_size as usize {
                    let (serial_type, bytes) = read_varint(&page1, offset);
                    serial_types.push(serial_type);
                    offset += bytes;
                }

                // Skip first two columns, read third (table name)
                offset += serial_types[0..2]
                    .iter()
                    .map(|&st| serial_type_size(st))
                    .sum::<usize>();

                let len = serial_type_size(serial_types[2]);
                if len > 0 {
                    println!("{}", String::from_utf8_lossy(&page1[offset..offset + len]));
                }
            }
        }
        None => {
            if let Some(query) = cli.query {
                let table_name = query.split_whitespace().last().unwrap();
                let mut file = File::open(&cli.database)?;
                let page1 = read_first_page(&mut file)?;
                let page_size = page1.len();

                let rootpage = find_rootpage(&page1, table_name)?;
                file.seek(std::io::SeekFrom::Start(
                    (rootpage - 1) as u64 * page_size as u64,
                ))?;

                let mut page = vec![0u8; page_size];
                file.read_exact(&mut page)?;

                println!("{}", u16::from_be_bytes([page[3], page[4]]));
            } else {
                println!("No command or query provided. Use --help for usage information.");
            }
        }
    }

    Ok(())
}

fn find_rootpage(page1: &[u8], target_table: &str) -> Result<u32> {
    for cell_offset in get_cell_offsets(page1) {
        let mut offset = cell_offset;

        // Skip payload size and rowid
        let (_, bytes) = read_varint(page1, offset);
        offset += bytes;
        let (_, bytes) = read_varint(page1, offset);
        offset += bytes;

        // Read serial types
        let (header_size, bytes) = read_varint(page1, offset);
        let header_start = offset;
        offset += bytes;

        let mut serial_types = Vec::new();
        while offset < header_start + header_size as usize {
            let (st, bytes) = read_varint(page1, offset);
            serial_types.push(st);
            offset += bytes;
        }

        // Parse columns: type(0), name(1), tbl_name(2), rootpage(3), sql(4)
        offset += serial_type_size(serial_types[0]); // Skip type column

        let name_len = serial_type_size(serial_types[1]);
        let name = String::from_utf8_lossy(&page1[offset..offset + name_len]).to_string();
        offset += name_len;

        offset += serial_type_size(serial_types[2]); // Skip tbl_name column

        let rootpage_size = serial_type_size(serial_types[3]);
        let rootpage = page1[offset..offset + rootpage_size]
            .iter()
            .fold(0u32, |acc, &b| (acc << 8) | b as u32);

        if name == target_table {
            return Ok(rootpage);
        }
    }

    anyhow::bail!("Table '{}' not found", target_table)
}
