use chrono::Utc;
use reqwest::header;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

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
    pub albums: Vec<MediaEntity>,
    pub tracks: Vec<MediaEntity>,
}

#[derive(Debug, Deserialize)]
pub struct MediaEntity {
    pub url: String,
    pub title: String,
    pub artists: Vec<Artist>,
}

#[derive(Debug, Deserialize)]
pub struct Artist {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MetadataResponse {
    pub success: bool,
    pub title: String,
    pub tracks: Vec<MediaEntity>,
}

pub struct DownloadResponse {
    pub response: reqwest::Response,
    pub filename: Option<String>,
    pub content_type: Option<String>,
}

pub enum LucidaHost {
    LucidaSu,
    LucidaTo,
}

impl LucidaHost {
    pub fn as_str(&self) -> &'static str {
        match self {
            LucidaHost::LucidaTo => "lucida.to",
            LucidaHost::LucidaSu => "lucida.su",
        }
    }
}

#[derive(PartialEq)]
pub enum LucidaServer {
    Hund,
    Katze,
}

#[derive(Clone)]
pub enum LucidaService {
    Qobuz,
    Tidal,
    Soundcloud,
    Deezer,
    AmazonMusic,
    YandexMusic,
}

impl LucidaService {
    pub fn as_str(&self) -> &'static str {
        match self {
            LucidaService::Qobuz => "qobuz",
            LucidaService::Tidal => "tidal",
            LucidaService::Soundcloud => "soundcloud",
            LucidaService::Deezer => "deezer",
            LucidaService::AmazonMusic => "amazon",
            LucidaService::YandexMusic => "yandex",
        }
    }

    pub fn from_str(s: &str) -> Option<LucidaService> {
        match s {
            "qobuz" => Some(LucidaService::Qobuz),
            "tidal" => Some(LucidaService::Tidal),
            "soundcloud" => Some(LucidaService::Soundcloud),
            "deezer" => Some(LucidaService::Deezer),
            "amazon" => Some(LucidaService::AmazonMusic),
            "yandex" => Some(LucidaService::YandexMusic),
            _ => None,
        }
    }

    pub fn from_url(url: &str) -> Option<LucidaService> {
        if url.contains("qobuz.com") {
            Some(LucidaService::Qobuz)
        } else if url.contains("tidal.com") {
            Some(LucidaService::Tidal)
        } else if url.contains("soundcloud.com") {
            Some(LucidaService::Soundcloud)
        } else if url.contains("deezer.com") {
            Some(LucidaService::Deezer)
        } else if url.contains("music.amazon.com") {
            Some(LucidaService::AmazonMusic)
        } else if url.contains("music.yandex.ru") {
            Some(LucidaService::YandexMusic)
        } else {
            None
        }
    }
}

impl LucidaServer {
    pub fn as_str(&self) -> &'static str {
        match self {
            LucidaServer::Hund => "hund",
            LucidaServer::Katze => "katze",
        }
    }
}

#[derive(Debug)]
pub enum LucidaError {
    UnknownService,
    NotAvailable,
    RequestFailed(String),
    ServerError(String),
}

impl std::fmt::Display for LucidaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LucidaError::UnknownService => write!(f, "unknown service"),
            LucidaError::NotAvailable => write!(f, "not available"),
            LucidaError::RequestFailed(_) => write!(f, "request failed"),
            LucidaError::ServerError(_) => write!(f, "server error"),
        }
    }
}

impl std::error::Error for LucidaError {}

impl From<reqwest::Error> for LucidaError {
    fn from(value: reqwest::Error) -> Self {
        Self::RequestFailed(value.to_string())
    }
}

pub struct LucidaClient {
    client: reqwest::Client,
    host: LucidaHost,
    server: LucidaServer,
}

impl LucidaClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .read_timeout(Duration::from_secs(60))
                .build()
                .unwrap(),
            host: LucidaHost::LucidaTo,
            server: LucidaServer::Hund,
        }
    }

    pub fn with_options(host: LucidaHost, server: LucidaServer) -> Self {
        Self {
            client: reqwest::Client::builder()
                .read_timeout(Duration::from_secs(60))
                .build()
                .unwrap(),
            host,
            server,
        }
    }

    pub async fn try_download_all_countries(
        &self,
        url: &str,
        metadata: bool,
        mut event_callback: impl FnMut(&str),
    ) -> Result<DownloadResponse, LucidaError> {
        let Some(service) = LucidaService::from_url(url) else {
            return Err(LucidaError::UnknownService);
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
                event_callback(&status_message);

                let processing_start = Utc::now();
                while status != "completed" && status != "error" {
                    status_response = self.fetch_status(&id).await?;
                    status = status_response.status;
                    if status_message != status_response.message {
                        status_message = status_response.message;
                        event_callback(&status_message);
                    }
                    if Utc::now()
                        .signed_duration_since(processing_start)
                        .num_seconds()
                        > 60
                    {
                        event_callback("Processing timeout, retrying...");
                        status = "error".to_string();
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
                if status == "error" {
                    continue;
                }

                event_callback("Downloading stream locally");

                return self.fetch_download(&id).await;
            }
        }
        Err(LucidaError::NotAvailable)
    }

    pub async fn fetch_stream(
        &self,
        url: &str,
        country: Option<&str>,
        metadata: bool,
    ) -> Result<StreamResponse, LucidaError> {
        let response: StreamResponse = self
            .client
            .post(format!(
                "https://{}.{}/api/fetch/stream/v2",
                self.server.as_str(),
                self.host.as_str()
            ))
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
            .await?;
        Ok(response)
    }

    pub async fn fetch_status(&self, id: &str) -> Result<StatusResponse, LucidaError> {
        let response: StatusResponse = self
            .client
            .get(format!(
                "https://{}.{}/api/fetch/request/{id}",
                self.server.as_str(),
                self.host.as_str()
            ))
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    pub async fn fetch_download(&self, id: &str) -> Result<DownloadResponse, LucidaError> {
        let response = self
            .client
            .get(format!(
                "https://{}.{}/api/fetch/request/{id}/download",
                self.server.as_str(),
                self.host.as_str()
            ))
            .send()
            .await?;

        let headers = response.headers();
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Some(v.to_string()));

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
    ) -> Result<SearchResponse, LucidaError> {
        let response: SearchResponse = self
            .client
            .get(format!(
                "https://{}.{}/api/search",
                self.server.as_str(),
                self.host.as_str()
            ))
            .query(&[
                ("query", query),
                ("service", service.as_str()),
                ("country", country),
            ])
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    pub async fn fetch_countries(
        &self,
        service: LucidaService,
    ) -> Result<CountriesResponse, LucidaError> {
        let response: CountriesResponse = self
            .client
            .get(format!(
                "https://{}.{}/api/countries",
                self.server.as_str(),
                self.host.as_str()
            ))
            .query(&[("service", service.as_str())])
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    pub async fn fetch_metadata(&self, url: &str) -> Result<MetadataResponse, LucidaError> {
        let response: MetadataResponse = self
            .client
            .get(format!(
                "https://{}.{}/api/fetch/metadata",
                self.server.as_str(),
                self.host.as_str()
            ))
            .query(&[("url", url)])
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }
}
