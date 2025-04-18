use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{Select, theme::ColorfulTheme};
use lucida_api::{LucidaClient, LucidaHost, LucidaServer, LucidaService, SearchResponse};
use tokio::{fs, sync::mpsc};

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
    SearchAlbum {
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

async fn search(
    client: &LucidaClient,
    service: &str,
    query: &str,
) -> Result<SearchResponse, &'static str> {
    let Some(service) = LucidaService::from_str(service) else {
        return Err("Unknown service");
    };
    let countries = client.fetch_countries(service.clone()).await.unwrap();
    if countries.countries.len() == 0 {
        return Err("Service unavailable");
    }
    let country = countries.countries[0].code.clone();
    Ok(client.fetch_search(service, &country, query).await.unwrap())
}

async fn download_and_save(
    client: &LucidaClient,
    url: &str,
    title: &str,
    metadata: bool,
    current: usize,
    total: usize,
    append_current: bool,
) {
    let log = move |m: &str| {
        println!(
            "{} {} {} {}",
            style("[").dim(),
            style(format!("{current}")).bold().cyan(),
            style(format!("/ {total} ]")).dim(),
            style(m).bold()
        )
    };
    loop {
        let title = title.to_string();
        let (tx, mut rx) = mpsc::channel::<String>(16);
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                log(&message.replace("{item}", &title));
            }
        });
        let response = match client.try_download_all_countries(url, metadata, tx).await {
            Ok(response) => response,
            Err(e) => {
                log(&format!("Download failed, retrying... ({e})"));
                continue;
            }
        };

        let mut filename = response.filename.unwrap().replace("/", "&");
        if append_current {
            filename = format!("{current}. {filename}");
        }

        fs::write(filename, response.response.bytes().await.unwrap())
            .await
            .unwrap();
        break;
    }
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

    match &cli.command {
        Commands::Rip { url } => {
            download_and_save(&client, &url, &url, cli.metadata, 1, 1, false).await
        }
        Commands::Search { query, service } => {
            let response = search(&client, &service, &query).await.unwrap();
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
            download_and_save(
                &client,
                &response.results.tracks[selection].url,
                &selector[selection],
                cli.metadata,
                1,
                1,
                false,
            )
            .await;
        }
        Commands::SearchAlbum { query, service } => {
            let response = search(&client, &service, &query).await.unwrap();
            let selector: Vec<String> = response
                .results
                .albums
                .iter()
                .map(|v| {
                    format!(
                        "{} {} {}",
                        style(&v.title).bold(),
                        style("-").dim(),
                        style(&v.artists.as_ref().unwrap()[0].name).dim()
                    )
                })
                .collect();
            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Choose the album")
                .default(0)
                .max_length(5)
                .items(&selector)
                .interact()
                .unwrap();
            let album = client
                .fetch_metadata(&response.results.albums[selection].url)
                .await
                .unwrap();
            for i in 0..album.tracks.len() {
                download_and_save(
                    &client,
                    &album.tracks[i].url,
                    &album.tracks[i].title,
                    cli.metadata,
                    i + 1,
                    album.tracks.len(),
                    true,
                )
                .await;
            }
        }
    }
}
