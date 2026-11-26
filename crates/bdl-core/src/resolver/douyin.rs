//! Douyin single-work source.
//!
//! Douyin protects its web API with a runtime signature (`a_bogus`/`x-tt-argus`)
//! and its share pages no longer embed the item for anonymous requests, so the
//! play address comes from a parse service instead of a hand-signed request.
//! BDL validates that the answer belongs to the requested work and never trusts
//! a different ID: a mismatched response is an error, not a download.
//!
//! The endpoint is configurable through `BDL_DOUYIN_API` and may point at a
//! self-hosted service. The service receives the canonical work URL and must
//! answer with the same `code`/`msg`/`data` envelope used by the default API.

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use url::Url;

use super::{ResolveOptions, Resolver, bilibili_publish_date};
use crate::error::{BdlError, BdlResult};
use crate::ids::{GroupId, ItemId, PartId, SourceId};
use crate::input::ClassifiedInput;
use crate::model::{
    HeaderPair, MediaKind, MediaStream, NormalizedGroup, NormalizedItem, NormalizedPart,
    NormalizedSourceTree, SourceKind, SourceSummary, StreamCodec, StreamQuality,
};

/// Public parse service used when `BDL_DOUYIN_API` is not set.
pub const DEFAULT_DOUYIN_API_ENDPOINT: &str = "https://api.xingzhige.com/API/douyin/";
pub const DOUYIN_API_ENV: &str = "BDL_DOUYIN_API";
pub const DOUYIN_MEDIA_REFERER: &str = "https://www.douyin.com/";
const DOUYIN_REQUEST_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
const DOUYIN_MEDIA_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub fn canonical_work_url(aweme_id: &str) -> String {
    format!("https://www.douyin.com/video/{aweme_id}")
}

#[derive(Debug, Clone)]
pub struct DouyinResolver {
    client: reqwest::Client,
    endpoint: String,
}

impl DouyinResolver {
    pub fn new() -> BdlResult<Self> {
        Self::with_endpoint(douyin_api_endpoint())
    }

    pub fn with_endpoint(endpoint: impl Into<String>) -> BdlResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .no_proxy()
            .build()
            .map_err(|error| BdlError::SourceService {
                message: format!("抖音解析服务连接失败：{error}"),
            })?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
        })
    }

    async fn fetch_work(&self, aweme_id: &str) -> BdlResult<DouyinApiData> {
        let mut endpoint = Url::parse(&self.endpoint).map_err(|_| BdlError::Planning {
            message: "抖音解析服务地址无效，请检查 BDL_DOUYIN_API 配置。".into(),
        })?;
        endpoint
            .query_pairs_mut()
            .append_pair("url", &canonical_work_url(aweme_id));

        let response = self
            .client
            .get(endpoint)
            .header(reqwest::header::USER_AGENT, DOUYIN_REQUEST_USER_AGENT)
            .header(reqwest::header::REFERER, DOUYIN_MEDIA_REFERER)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| BdlError::SourceService {
                message: format!(
                    "{}，请检查网络后重试；也可通过 BDL_DOUYIN_API 更换解析服务。原因：{}",
                    if error.is_timeout() {
                        "抖音解析服务响应超时"
                    } else if error.is_connect() {
                        "抖音解析服务连接失败"
                    } else {
                        "抖音解析服务请求失败"
                    },
                    crate::error::error_causes(&error)
                ),
            })?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| BdlError::SourceService {
                message: format!("抖音解析服务响应读取失败：{error}"),
            })?;
        parse_api_body(status.as_u16(), &body)
    }
}

/// Read `BDL_DOUYIN_API` once per parse so a self-hosted service can be used
/// without changing the stored settings shape.
pub fn douyin_api_endpoint() -> String {
    std::env::var(DOUYIN_API_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DOUYIN_API_ENDPOINT.to_owned())
}

#[async_trait]
impl Resolver for DouyinResolver {
    async fn resolve(
        &self,
        input: ClassifiedInput,
        options: ResolveOptions,
    ) -> BdlResult<NormalizedSourceTree> {
        let kind = input.source_kind();
        let ClassifiedInput::Douyin { aweme_id } = input else {
            return Err(BdlError::UnsupportedSource {
                kind: source_kind_name(kind).to_owned(),
            });
        };
        let data = self.fetch_work(&aweme_id).await?;
        normalize_work(&aweme_id, data, options.fetch_streams)
    }
}

#[derive(Debug, Default, Deserialize)]
struct DouyinApiResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<DouyinApiData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DouyinApiData {
    jx: DouyinJx,
    author: DouyinAuthor,
    stat: DouyinStat,
    item: DouyinItem,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DouyinJx {
    item_id: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DouyinAuthor {
    name: String,
    uid: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DouyinStat {
    aweme_id: String,
    time: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DouyinItem {
    title: String,
    cover: String,
    url: String,
    ury: String,
    quality: String,
    codec: String,
    bitrate: String,
    duration: Option<f64>,
    size: Option<u64>,
    size_y: Option<u64>,
    #[serde(default)]
    images: Vec<serde_json::Value>,
}

fn parse_api_body(status: u16, body: &str) -> BdlResult<DouyinApiData> {
    if !(200..300).contains(&status) {
        return Err(BdlError::SourceService {
            message: format!("抖音解析服务返回 HTTP {status}，请稍后重试或更换解析服务。"),
        });
    }
    let response: DouyinApiResponse =
        serde_json::from_str(body).map_err(|_| BdlError::SourceService {
            message: "抖音解析服务返回了无法识别的内容，请稍后重试或更换解析服务。".into(),
        })?;
    let data = response.data.ok_or_else(|| BdlError::SourceService {
        message: douyin_service_message(&response.msg),
    })?;
    if response.code != 0 {
        return Err(BdlError::SourceService {
            message: douyin_service_message(&response.msg),
        });
    }
    Ok(data)
}

fn douyin_service_message(message: &str) -> String {
    match message.trim() {
        "" => "抖音解析服务没有返回作品信息，请稍后重试。".into(),
        message if message.contains("频繁") || message.contains("限速") => {
            format!("抖音解析服务暂时限速：{message}。请稍后重试。")
        }
        message => format!("抖音解析失败：{message}"),
    }
}

fn normalize_work(
    aweme_id: &str,
    data: DouyinApiData,
    fetch_streams: bool,
) -> BdlResult<NormalizedSourceTree> {
    let returned_id = [data.jx.item_id.trim(), data.stat.aweme_id.trim()]
        .into_iter()
        .find(|value| !value.is_empty());
    if let Some(returned_id) = returned_id
        && returned_id != aweme_id
    {
        return Err(BdlError::Planning {
            message: "抖音解析结果与所选作品不一致，请重新解析。".into(),
        });
    }

    if data.jx.kind.contains("图") || !data.item.images.is_empty() {
        return Err(BdlError::Planning {
            message: "这条抖音作品是图文（图集），BDL 暂不支持下载图文作品。".into(),
        });
    }

    let play_urls = ordered_play_urls(&data.item);
    if play_urls.is_empty() {
        return Err(BdlError::SourceService {
            message: "抖音解析服务没有返回视频地址，请稍后重试或更换解析服务。".into(),
        });
    }

    let title = data.item.title.trim();
    let title = if title.is_empty() {
        format!("抖音视频 {aweme_id}")
    } else {
        title.to_owned()
    };
    let owner_name = non_empty(&data.author.name);
    let cover_url = non_empty(&data.item.cover);
    let duration_seconds = data
        .item
        .duration
        .filter(|value| *value > 0.0)
        .map(|value| value.round() as u64);
    let publish_date = data.stat.time.and_then(bilibili_publish_date);
    let streams = if fetch_streams {
        vec![media_stream(aweme_id, &data.item, play_urls)]
    } else {
        Vec::new()
    };
    let key = format!("douyin:{aweme_id}");
    let source_url = canonical_work_url(aweme_id);

    let item = NormalizedItem {
        nfo_metadata: crate::nfo::NfoMetadata {
            description: title.clone(),
            uploader: owner_name.clone().unwrap_or_default(),
            publish_date: publish_date.clone().unwrap_or_default(),
            duration_seconds,
            source_url: source_url.clone(),
            ..Default::default()
        },
        id: ItemId(format!("item:{key}")),
        title: title.clone(),
        owner_name: owner_name.clone(),
        owner_mid: None,
        publish_date,
        cover_url,
        duration_seconds,
        parts: vec![NormalizedPart {
            id: PartId(format!("part:{key}")),
            title: title.clone(),
            aid: None,
            bvid: None,
            cid: None,
            duration_seconds,
            streams,
            assets: Vec::new(),
        }],
    };

    Ok(NormalizedSourceTree {
        source: SourceSummary {
            id: SourceId(key.clone()),
            kind: SourceKind::Douyin,
            input: source_url,
            title: title.clone(),
            loaded_count: 1,
            total_count: Some(1),
            has_more: false,
        },
        groups: vec![NormalizedGroup {
            id: GroupId(format!("group:{key}")),
            kind: "douyin".to_owned(),
            title,
            items: vec![item],
            page: None,
        }],
    })
}

/// The service reports the standard rendering plus, when available, a higher
/// bitrate rendering whose size is only exposed as `size_y`.
fn ordered_play_urls(item: &DouyinItem) -> Vec<String> {
    let standard = non_empty(&item.url);
    let larger = non_empty(&item.ury).filter(|_| {
        item.size_y
            .zip(item.size)
            .map(|(larger_size, standard_size)| larger_size > standard_size)
            .unwrap_or(true)
    });
    let mut urls = Vec::with_capacity(2);
    for url in larger.into_iter().chain(standard) {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

fn media_stream(aweme_id: &str, item: &DouyinItem, urls: Vec<String>) -> MediaStream {
    MediaStream {
        id: format!("stream:douyin:{aweme_id}:video"),
        kind: MediaKind::Video,
        quality: quality_height(&item.quality),
        codec: StreamCodec::from_source(&item.codec),
        bandwidth: parse_bitrate(&item.bitrate),
        urls,
        headers: vec![
            HeaderPair {
                name: "Referer".to_owned(),
                value: DOUYIN_MEDIA_REFERER.to_owned(),
            },
            HeaderPair {
                name: "User-Agent".to_owned(),
                value: DOUYIN_MEDIA_USER_AGENT.to_owned(),
            },
        ],
        acquired_at: Utc::now(),
        includes_audio: true,
    }
}

/// `"540p"` is the only quality label the service exposes; the planner ranks by
/// this numeric value, so use the vertical resolution it names.
fn quality_height(label: &str) -> StreamQuality {
    label
        .trim()
        .trim_end_matches(['p', 'P'])
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .map_or(StreamQuality::Best, StreamQuality::Quality)
}

/// `"420.69kbps"` describes bits per second; the planner expects that unit.
fn parse_bitrate(value: &str) -> Option<u64> {
    let value = value.trim();
    let (number, scale) = if let Some(number) = value.strip_suffix("kbps") {
        (number, 1_000.0)
    } else if let Some(number) = value.strip_suffix("mbps") {
        (number, 1_000_000.0)
    } else {
        (value.strip_suffix("bps").unwrap_or(value), 1.0)
    };
    let number = number.trim().parse::<f64>().ok()?;
    (number > 0.0).then(|| (number * scale).round() as u64)
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn source_kind_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Video => "video",
        SourceKind::Bangumi => "bangumi",
        SourceKind::Cheese => "cheese",
        SourceKind::Favorite => "favorite",
        SourceKind::WatchLater => "watch_later",
        SourceKind::Collection => "collection",
        SourceKind::Series => "series",
        SourceKind::Uploader => "uploader",
        SourceKind::Douyin => "douyin",
        SourceKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWEME_ID: &str = "7709208428677229898";

    fn payload() -> serde_json::Value {
        serde_json::json!({
            "code": 0,
            "msg": "解析成功",
            "data": {
                "jx": { "item_id": AWEME_ID, "type": "视频" },
                "author": { "name": "薄荷猫", "uid": "haku0201" },
                "stat": { "aweme_id": AWEME_ID, "time": 1794939962 },
                "item": {
                    "title": "请依赖19岁的咲哉吧！\n生日快乐",
                    "cover": "https://p95.example.invalid/cover.jpeg",
                    "url": "https://v95.example.invalid/standard.mp4",
                    "ury": "https://v95.example.invalid/large.mp4",
                    "quality": "540p",
                    "codec": "bytevc1",
                    "bitrate": "420.69kbps",
                    "duration": 70.543,
                    "size": 3709606,
                    "size_y": 10154973
                }
            }
        })
    }

    fn resolve(
        payload: &serde_json::Value,
        fetch_streams: bool,
    ) -> BdlResult<NormalizedSourceTree> {
        let data = parse_api_body(200, &payload.to_string())?;
        normalize_work(AWEME_ID, data, fetch_streams)
    }

    #[test]
    fn builds_a_single_muxed_video_stream_and_keeps_metadata() {
        let tree = resolve(&payload(), true).unwrap();

        assert_eq!(tree.source.kind, SourceKind::Douyin);
        assert_eq!(tree.source.id.0, format!("douyin:{AWEME_ID}"));
        assert_eq!(
            tree.source.input,
            format!("https://www.douyin.com/video/{AWEME_ID}")
        );
        assert_eq!(tree.groups.len(), 1);

        let item = &tree.groups[0].items[0];
        assert_eq!(item.owner_name.as_deref(), Some("薄荷猫"));
        assert_eq!(item.duration_seconds, Some(71));
        assert_eq!(item.publish_date.as_deref(), Some("2026-11-18"));
        assert_eq!(item.title, "请依赖19岁的咲哉吧！\n生日快乐");

        let part = &item.parts[0];
        assert!(part.cid.is_none() && part.bvid.is_none());
        let stream = &part.streams[0];
        assert_eq!(stream.kind, MediaKind::Video);
        assert!(stream.includes_audio);
        assert_eq!(
            stream.urls,
            vec![
                "https://v95.example.invalid/large.mp4",
                "https://v95.example.invalid/standard.mp4"
            ]
        );
        assert_eq!(stream.codec, StreamCodec::Hevc);
        assert_eq!(stream.bandwidth, Some(420_690));
        assert_eq!(stream.quality, StreamQuality::Quality(540));
        assert!(
            stream
                .headers
                .iter()
                .any(|header| header.name == "User-Agent")
        );
    }

    #[test]
    fn metadata_only_resolution_skips_signed_play_urls() {
        let tree = resolve(&payload(), false).unwrap();
        assert!(tree.groups[0].items[0].parts[0].streams.is_empty());
    }

    #[test]
    fn keeps_standard_url_when_the_larger_rendering_is_smaller() {
        let mut payload = payload();
        payload["data"]["item"]["size_y"] = serde_json::json!(1000);
        let tree = resolve(&payload, true).unwrap();
        assert_eq!(
            tree.groups[0].items[0].parts[0].streams[0].urls,
            vec!["https://v95.example.invalid/standard.mp4"]
        );
    }

    #[test]
    fn rejects_image_posts_instead_of_treating_them_as_video() {
        let mut payload = payload();
        payload["data"]["jx"]["type"] = serde_json::json!("图集");
        payload["data"]["item"]["images"] =
            serde_json::json!([{ "url": "https://x.invalid/1.jpg" }]);
        let error = resolve(&payload, true).unwrap_err().to_string();
        assert!(error.contains("图文"));
    }

    #[test]
    fn rejects_answers_for_a_different_work() {
        let mut payload = payload();
        payload["data"]["jx"]["item_id"] = serde_json::json!("1");
        let error = resolve(&payload, true).unwrap_err().to_string();
        assert!(error.contains("不一致"));
    }

    #[test]
    fn surfaces_service_errors_without_leaking_the_body() {
        let error = parse_api_body(
            200,
            &serde_json::json!({ "code": 1, "msg": "解析失败，请稍后重试" }).to_string(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("解析失败，请稍后重试"));
        assert!(parse_api_body(500, "internal error").is_err());
        assert!(parse_api_body(200, "<html>blocked</html>").is_err());
    }

    #[test]
    fn bitrate_and_quality_labels_are_parsed_for_planning() {
        assert_eq!(parse_bitrate("420.69kbps"), Some(420_690));
        assert_eq!(parse_bitrate("1.2mbps"), Some(1_200_000));
        assert_eq!(parse_bitrate("1000000bps"), Some(1_000_000));
        assert_eq!(parse_bitrate(""), None);
        assert_eq!(quality_height("1080p"), StreamQuality::Quality(1080));
        assert_eq!(quality_height(""), StreamQuality::Best);
    }

    #[test]
    fn canonical_urls_and_endpoint_override_are_stable() {
        assert_eq!(
            canonical_work_url(AWEME_ID),
            format!("https://www.douyin.com/video/{AWEME_ID}")
        );
        // The environment is process-wide; only assert the fallback shape here.
        let endpoint = douyin_api_endpoint();
        assert!(endpoint.starts_with("http"));
    }

    #[tokio::test]
    #[ignore = "live Douyin probe: needs network access to the parse service"]
    async fn live_douyin_work_resolves_and_downloads_through_the_planner() {
        use crate::fetcher::{FetchConfig, Fetcher, ReqwestFetcher};
        use crate::planner::{DownloadOptions, plan_selected_parts};

        let tree = DouyinResolver::new()
            .unwrap()
            .resolve(
                ClassifiedInput::Douyin {
                    aweme_id: AWEME_ID.to_owned(),
                },
                ResolveOptions {
                    fetch_streams: true,
                },
            )
            .await
            .expect("the live parse service should resolve a public work");

        assert_eq!(tree.source.kind, SourceKind::Douyin);
        let part = &tree.groups[0].items[0].parts[0];
        assert!(part.streams[0].includes_audio);

        let root = std::env::temp_dir().join(format!("bdl-douyin-live-{}", uuid::Uuid::new_v4()));
        let options = DownloadOptions::new(root.clone());
        let task = plan_selected_parts(&tree, std::slice::from_ref(&part.id), &options)
            .unwrap()
            .remove(0);
        // A single-file work plans one muxed video resource, not a second audio one.
        assert_eq!(task.resources.len(), 1);

        let fetcher = ReqwestFetcher::with_config(FetchConfig {
            max_retries: 0,
            ..Default::default()
        })
        .unwrap();
        std::fs::create_dir_all(root.join("media")).unwrap();
        fetcher
            .fetch(&task.resources[0], None)
            .await
            .expect("the CDN should accept the planned stream and headers");
        assert!(
            std::fs::metadata(&task.resources[0].target_path)
                .map(|metadata| metadata.len() > 1024)
                .unwrap_or(false)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
