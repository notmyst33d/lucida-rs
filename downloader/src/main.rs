use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{Select, theme::ColorfulTheme};
use lucida_api::{LucidaClient, LucidaHost, LucidaServer, LucidaService};
use tokio::fs;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(
        long,
        help = "API host: lucida-to, lucida-su (default: lucida-to)",
        global = true
    )]
    host: Option<String>,

    #[arg(
        long,
        help = "Media server: hund, katze (default: hund)",
        global = true
    )]
    server: Option<String>,

    #[arg(long, help = "Embed metadata", global = true)]
    metadata: bool,
}

#[derive(Subcommand)]
enum Commands {
    Rip {
        #[arg(short, long, help = "Streaming service track link")]
        url: String,
    },
    Search {
        #[arg(short, long, help = "Search query")]
        query: String,

        #[arg(
            short,
            long,
            help = "Streaming service: qobuz, tidal, soundcloud, deezer, amazon, yandex"
        )]
        service: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let host = if let Some(host) = cli.host {
        match host.as_str() {
            "lucida-su" => LucidaHost::LucidaSu,
            _ => LucidaHost::LucidaTo,
        }
    } else {
        LucidaHost::LucidaTo
    };
    let server = if let Some(server) = cli.server {
        match server.as_str() {
            "hund" => LucidaServer::Hund,
            "katze" => LucidaServer::Katze,
            _ => LucidaServer::Hund,
        }
    } else {
        LucidaServer::Hund
    };
    let client = LucidaClient::with_options(host, server);

    let url = match &cli.command {
        Commands::Rip { url } => url.to_string(),
        Commands::Search { query, service } => {
            let Some(service) = LucidaService::from_str(&service) else {
                println!("Unknown service");
                return;
            };
            let countries = client.fetch_countries(service.clone()).await.unwrap();
            if countries.countries.len() == 0 {
                println!("Service unavailable");
                return;
            }
            let country = countries.countries[0].code.clone();
            let response = client
                .fetch_search(service, &country, &query)
                .await
                .unwrap();
            let selector: Vec<String> = response
                .results
                .tracks
                .iter()
                .map(|v| {
                    format!(
                        "{} {} {}",
                        style(&v.title).bold(),
                        style("-").dim(),
                        style(&v.artists[0].name).dim()
                    )
                })
                .collect();
            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Choose the track")
                .default(0)
                .max_length(5)
                .items(&selector)
                .interact()
                .unwrap();
            response.results.tracks[selection].url.clone()
        }
    };

    let mut current = String::new();
    let response = client
        .try_download_all_countries(&url, cli.metadata, |e| {
            if current != e {
                current = e;
                println!("{}", current.replace("{item}", &url));
            }
        })
        .await
        .unwrap();

    fs::write(
        response.filename.unwrap(),
        response.response.bytes().await.unwrap(),
    )
    .await
    .unwrap();
}
