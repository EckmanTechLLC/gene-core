use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "gene-ctl", about = "Control interface for the gene agent")]
struct Args {
    #[arg(long, default_value = "/tmp/gene.sock")]
    socket: PathBuf,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Overall status snapshot
    Status,
    /// Full signal bus dump
    Signals,
    /// Active symbol activations
    Symbols,
    /// Current identity trace
    Identity,
    /// Inject a delta into a signal (by numeric ID)
    Inject { signal_id: u32, delta: f64 },
    /// Pause the tick loop
    Pause,
    /// Resume the tick loop
    Resume,
    /// Request an immediate checkpoint
    Checkpoint,
    /// Show recent expression records
    Expressions {
        /// Number of recent records to return (default 20)
        #[arg(default_value = "20")]
        n: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let payload = match &args.command {
        Cmd::Status     => r#"{"cmd":"Status"}"#.to_string(),
        Cmd::Signals    => r#"{"cmd":"Signals"}"#.to_string(),
        Cmd::Symbols    => r#"{"cmd":"Symbols"}"#.to_string(),
        Cmd::Identity   => r#"{"cmd":"Identity"}"#.to_string(),
        Cmd::Pause      => r#"{"cmd":"Pause"}"#.to_string(),
        Cmd::Resume     => r#"{"cmd":"Resume"}"#.to_string(),
        Cmd::Checkpoint => r#"{"cmd":"Checkpoint"}"#.to_string(),
        Cmd::Expressions { n } => format!(r#"{{"cmd":"Expressions","n":{}}}"#, n),
        Cmd::Inject { signal_id, delta } => {
            format!(r#"{{"cmd":"Inject","signal_id":{},"delta":{}}}"#, signal_id, delta)
        }
    };

    let mut stream = UnixStream::connect(&args.socket).await
        .map_err(|e| anyhow::anyhow!("Could not connect to gene at {:?}: {}", args.socket, e))?;

    let msg = payload + "\n";
    stream.write_all(msg.as_bytes()).await?;

    let (reader, _) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    // Pretty print
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("{}", response.trim());
    }

    Ok(())
}
