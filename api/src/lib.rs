use chrono::Utc;
use reqwest::header;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{sync::mpsc, time::sleep};

macro_rules! string_enum {
    (pub enum $name:ident {
        $($variant:ident = { name: $value:literal, url: $url:literal },)*
    }) => {
        #[derive(Clone)]
        pub enum $name {
            $($variant),*
        }

        impl TryFrom<&str> for $name {
            type Error = ();

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                let urls = [$((Self::$variant, $url),)*];
                if let Some(value) = urls.into_iter().find(|u| value.contains(u.1)) {
                    return Ok(value.0);
                }
                match value {
                    $($value => Ok(Self::$variant),)*
                    _ => Err(())
                }
            }
        }

        impl From<$name> for &str {
            fn from(value: LucidaService) -> Self {
                match value {
                    $($name::$variant => $value,)*
                }
            }
        }
    };
}

#[derive(Serialize)]
struct Account {
    id: String,
    #[serde(rename = "type")]
    account_type: String,
}

#[derive(Serialize)]
struct Upload {
    enabled: bool,
    service: String,
}

#[derive(Serialize)]
struct StreamRequest {
    account: Account,
    downscale: String,
    handoff: bool,
    metadata: bool,
    private: bool,
    upload: Upload,
    url: String,
}

#[derive(Debug, Deserialize)]
pub struct StreamResponse {
    pub success: bool,
    pub error: Option<String>,
    pub handoff: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StatusResponse {
    pub success: bool,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    pub success: bool,
    pub results: SearchResults,
}

#[derive(Debug, Deserialize)]
pub struct CountriesResponse {
    pub success: bool,
    pub countries: Vec<Country>,
}

#[derive(Debug, Deserialize)]
pub struct Country {
    pub code: String,
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchResults {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artwork {
    pub url: String,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Album {
    pub url: String,
    pub title: String,
    pub artists: Option<Vec<Artist>>,
    #[serde(alias = "coverArtwork")]
    pub cover_artwork: Option<Vec<Artwork>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    pub url: String,
    pub title: String,
    pub artists: Vec<Artist>,
    pub album: Option<Album>,
    #[serde(alias = "coverArtwork")]
    pub cover_artwork: Option<Vec<Artwork>>,
    #[serde(alias = "durationMs")]
    pub duration_ms: usize,
}

impl Track {
    pub fn artwork(&self) -> Option<String> {
        self.cover_artwork.as_ref().map_or_else(
            || {
                self.album
                    .as_ref()
                    .and_then(|a| a.cover_artwork.as_ref())
                    .map(|artwork| artwork[artwork.len() - 1].url.clone())
            },
            |artwork| Some(artwork[artwork.len() - 1].url.clone()),
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artist {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MetadataResponse {
    pub success: bool,
    pub title: String,
    pub tracks: Vec<Track>,
}

pub struct DownloadResponse {
    pub response: reqwest::Response,
    pub filename: Option<String>,
    pub content_type: Option<String>,
}

string_enum! {
    pub enum LucidaService {
        Qobuz = { name: "qobuz", url: "qobuz.com" },
        Tidal = { name: "tidal", url: "tidal.com" },
        Soundcloud = { name: "soundcloud", url: "soundcloud.com" },
        Deezer = { name: "deezer", url: "deezer.com" },
        AmazonMusic = { name: "amazon", url: "music.amazon.com" },
        YandexMusic = { name: "yandex", url: "music.yandex.ru" },
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TryDownloadAllCountriesError {
    #[error(transparent)]
    RequestError(#[from] reqwest::Error),

    #[error("unknown service")]
    UnknownService,

    #[error("no available countries")]
    NoAvailableCountries,
}

pub struct LucidaClient {
    client: reqwest::Client,
    base_url: String,
}

impl Default for LucidaClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LucidaClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .read_timeout(Duration::from_secs(60))
                .build()
                .unwrap(),
            base_url: "https://katze.lucida.to".to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    pub async fn try_download_all_countries(
        &self,
        url: &str,
        metadata: bool,
        tx: mpsc::Sender<String>,
    ) -> Result<DownloadResponse, TryDownloadAllCountriesError> {
        let Ok(service) = LucidaService::try_from(url) else {
            return Err(TryDownloadAllCountriesError::UnknownService);
        };
        let countries = self.fetch_countries(service).await?;
        for country in countries.countries {
            for _ in 0..3 {
                let stream_response = self
                    .fetch_stream(url, Some(&country.code), metadata)
                    .await?;
                if !stream_response.success {
                    continue;
                }

                let id = stream_response.handoff.unwrap();

                let mut status_response = self.fetch_status(&id).await?;
                let mut status_message = status_response.message;
                let mut status = status_response.status;
                tx.send(status_message.clone()).await.unwrap();

                let processing_start = Utc::now();
                while status != "completed" && status != "error" {
                    status_response = self.fetch_status(&id).await?;
                    status = status_response.status;
                    if status_message != status_response.message {
                        status_message = status_response.message;
                        tx.send(status_message.clone()).await.unwrap();
                    }
                    if Utc::now()
                        .signed_duration_since(processing_start)
                        .num_seconds()
                        > 60
                    {
                        tx.send("Processing timeout, retrying...".to_string())
                            .await
                            .unwrap();
                        status = "error".to_string();
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
                if status == "error" {
                    continue;
                }

                tx.send("Downloading stream locally".to_string())
                    .await
                    .unwrap();

                return Ok(self.fetch_download(&id).await?);
            }
        }
        Err(TryDownloadAllCountriesError::NoAvailableCountries)
    }

    pub async fn fetch_stream(
        &self,
        url: &str,
        country: Option<&str>,
        metadata: bool,
    ) -> Result<StreamResponse, reqwest::Error> {
        self.client
            .post(format!("{}/api/fetch/stream/v2", self.base_url))
            .json(&StreamRequest {
                account: Account {
                    id: country.unwrap_or("auto").to_string(),
                    account_type: "country".to_string(),
                },
                downscale: "original".to_string(),
                handoff: true,
                metadata,
                private: true,
                upload: Upload {
                    enabled: false,
                    service: "pixeldrain".to_string(),
                },
                url: url.to_string(),
            })
            .send()
            .await?
            .json()
            .await
    }

    pub async fn fetch_status(&self, id: &str) -> Result<StatusResponse, reqwest::Error> {
        self.client
            .get(format!("{}/api/fetch/request/{id}", self.base_url))
            .send()
            .await?
            .json()
            .await
    }

    pub async fn fetch_download(&self, id: &str) -> Result<DownloadResponse, reqwest::Error> {
        let response = self
            .client
            .get(format!("{}/api/fetch/request/{id}/download", self.base_url))
            .send()
            .await?;

        let headers = response.headers();
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());

        let mut filename = None;
        if let Some(disposition) = headers.get(header::CONTENT_DISPOSITION) {
            let mut parsed = mailparse::parse_content_disposition(disposition.to_str().unwrap());
            filename = parsed.params.remove("filename");
        }

        Ok(DownloadResponse {
            response,
            filename,
            content_type,
        })
    }

    pub async fn fetch_search(
        &self,
        service: LucidaService,
        country: &str,
        query: &str,
    ) -> Result<SearchResponse, reqwest::Error> {
        self.client
            .get(format!("{}/api/search", self.base_url))
            .query(&[
                ("query", query),
                ("service", service.into()),
                ("country", country),
            ])
            .send()
            .await?
            .json()
            .await
    }

    pub async fn fetch_countries(
        &self,
        service: LucidaService,
    ) -> Result<CountriesResponse, reqwest::Error> {
        self.client
            .get(format!("{}/api/countries", self.base_url))
            .query(&[("service", Into::<&str>::into(service))])
            .send()
            .await?
            .json()
            .await
    }

    pub async fn fetch_metadata(&self, url: &str) -> Result<MetadataResponse, reqwest::Error> {
        self.client
            .get(format!("{}/api/fetch/metadata", self.base_url))
            .query(&[("url", url)])
            .send()
            .await?
            .json()
            .await
    }
}
