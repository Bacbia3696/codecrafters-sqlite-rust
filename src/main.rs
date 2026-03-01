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
            let mut header = [0; 100];
            file.read_exact(&mut header)?;

            // The page size is stored at the 16th byte offset, using 2 bytes in big-endian order
            #[allow(unused_variables)]
            let page_size = u16::from_be_bytes([header[16], header[17]]);

            println!("database page size: {}", page_size);
        }
    }

    Ok(())
}
