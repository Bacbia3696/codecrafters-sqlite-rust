use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs::File;
use std::io::prelude::*;

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
    }

    Ok(())
}
