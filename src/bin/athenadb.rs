//! `athenadb`: the dashboard and HTTP API for a store.
//!
//!     cargo run --release --features server --bin athenadb -- --data ./data

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use athenadb::server::{app, Config};

const USAGE: &str = "\
athenadb: a dashboard and HTTP API for an AthenaDB store

USAGE:
    athenadb [--data DIR] [--host HOST] [--port PORT] [--writable] [--token TOKEN]

OPTIONS:
    --data DIR      The store's directory [default: ./data]
    --host HOST     Address to listen on [default: 127.0.0.1]
    --port PORT     Port to listen on [default: 8001]
    --writable      Allow adding, deleting and snapshots, and create DIR if it's missing.
                    Without this the store is opened read-only, which is safe even while
                    another process writes to it. Never point two writers at one store.
    --token TOKEN   Require this bearer token on the API. Also read from ATHENADB_TOKEN.
                    Needed to listen on anything other than loopback.
    --help          Show this message
";

struct Args {
    data: PathBuf,
    host: String,
    port: u16,
    writable: bool,
    token: Option<String>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        data: PathBuf::from("./data"),
        host: "127.0.0.1".to_string(),
        port: 8001,
        writable: false,
        token: std::env::var("ATHENADB_TOKEN").ok().filter(|t| !t.is_empty()),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--data" => args.data = PathBuf::from(value()?),
            "--host" => args.host = value()?,
            "--port" => {
                let port = value()?;
                args.port = port.parse().map_err(|_| format!("{port} isn't a valid port"))?;
            }
            "--token" => args.token = Some(value()?),
            "--writable" => args.writable = true,
            "--help" | "-h" => return Ok(None),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Some(args))
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("athenadb: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let loopback_only = is_loopback(&args.host);
    if !loopback_only && args.token.is_none() {
        eprintln!(
            "athenadb: refusing to listen on {} without a token. Anyone who can reach this \
             address could read the store{}. Pass --token or set ATHENADB_TOKEN.",
            args.host,
            if args.writable { " and change it" } else { "" },
        );
        return ExitCode::from(2);
    }
    if !args.writable && !args.data.is_dir() {
        eprintln!(
            "athenadb: there's no store at {}. Point --data at one, or add --writable to create it.",
            args.data.display()
        );
        return ExitCode::from(2);
    }

    let mode = if args.writable { "writable" } else { "read-only" };
    let data = args.data.clone();
    let router = match app(Config { data: args.data, writable: args.writable, token: args.token, loopback_only }) {
        Ok(router) => router,
        Err(e) => {
            eprintln!("athenadb: couldn't open {}: {e}", data.display());
            return ExitCode::FAILURE;
        }
    };
    let listener = match tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("athenadb: couldn't listen on {}:{}: {e}", args.host, args.port);
            return ExitCode::FAILURE;
        }
    };
    println!("AthenaDB dashboard: http://{}:{}", args.host, args.port);
    println!("Store: {} ({mode})", data.display());
    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("athenadb: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
