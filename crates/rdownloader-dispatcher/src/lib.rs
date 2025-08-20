use rdownloader_http::{download_multipart, download_sequential};
use reqwest::Client;
use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG};
// 修正导入路径，直接从 rdownloader_utils 导入
use rdownloader_utils::parse_content_range;
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub enum DispatchError {
    Http(rdownloader_http::DownloadError),
    Network(reqwest::Error),
    HttpError(reqwest::StatusCode),
    UnsupportedProtocol(String),
    BuildError(reqwest::Error),
    DownloadFailed(String),
}

impl From<rdownloader_http::DownloadError> for DispatchError {
    fn from(err: rdownloader_http::DownloadError) -> Self {
        DispatchError::Http(err)
    }
}
impl From<reqwest::Error> for DispatchError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_builder() {
            DispatchError::BuildError(err)
        } else {
            DispatchError::Network(err)
        }
    }
}

// --- 可配置参数 ---
const MIN_SIZE_FOR_MULTIPART: u64 = 1 * 1024 * 1024; // 1MB
const PROBE_MAX_RETRIES: u32 = 3;
const PROBE_INITIAL_BACKOFF_SECS: u64 = 1;

pub async fn dispatch(client: &Client, url: &str, path: &Path) -> Result<(), DispatchError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(DispatchError::UnsupportedProtocol(url.to_string()));
    }

    let mut last_error: Option<DispatchError> = None;

    // --- 探测重试循环 (实现了指数退避) ---
    // 考虑到 CDN 等网络环境可能返回临时性错误，我们在此处加入重试逻辑以提高稳定性。
    for attempt in 1..=PROBE_MAX_RETRIES {
        println!("发送探测请求 (尝试 {}/{}) ...", attempt, PROBE_MAX_RETRIES);
        let probe_res = client.get(url).header("Range", "bytes=0-1").send().await?;

        // 如果请求成功 (2xx) 或作为部分内容响应 (206)，则认为探测成功
        if probe_res.status().is_success() || probe_res.status() == 206 {
            let headers = probe_res.headers();
            // 提取 ETag 用于后续的文件一致性校验
            let etag = headers
                .get(ETAG)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            // 提取 Content-Type 用于后续数据块的内容校验，防止静默的 HTML 错误页面
            let content_type = headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            // 优先通过 Content-Range 判断，这是最可靠的方式
            if let Some(range_str) = headers.get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
                if let Some(size) = parse_content_range(range_str) {
                    if size > MIN_SIZE_FOR_MULTIPART {
                        println!("探测成功 (Content-Range): 文件较大，启动多线程模式。");
                        return Ok(
                            download_multipart(client, url, path, size, etag, content_type).await?,
                        );
                    } else {
                        println!("将使用单线程模式 (文件较小)。");
                        return Ok(download_sequential(
                            client,
                            url,
                            path,
                            Some(size),
                            etag,
                            content_type,
                        )
                        .await?);
                    }
                }
            }

            // 如果 Content-Range 不可用，则回退到 Content-Length + Accept-Ranges 的组合
            if let Some(size_str) = headers.get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()) {
                if let Ok(size) = size_str.parse::<u64>() {
                    if headers.get(ACCEPT_RANGES).map_or(false, |v| v == "bytes")
                        && size > MIN_SIZE_FOR_MULTIPART
                    {
                        println!(
                            "探测成功 (Content-Length): 文件较大且服务器支持并发，启动多线程模式。"
                        );
                        return Ok(
                            download_multipart(client, url, path, size, etag, content_type).await?,
                        );
                    } else {
                        println!("将使用单线程模式 (服务器不支持并发或文件较小)。");
                        return Ok(download_sequential(
                            client,
                            url,
                            path,
                            Some(size),
                            etag,
                            content_type,
                        )
                        .await?);
                    }
                }
            }

            // --- 降级处理 ---
            // 如果以上所有方法都无法确定文件大小，则降级到不支持断点续传的单线程流式下载。
            println!("警告: 无法从服务器响应头中确定文件总大小。");
            return Ok(download_sequential(client, url, path, None, etag, content_type).await?);
        } else {
            // 如果服务器返回明确的错误，记录下来
            last_error = Some(DispatchError::HttpError(probe_res.status()));
        }

        // 如果还未到最大重试次数，则等待一段时间后重试
        if attempt < PROBE_MAX_RETRIES {
            // 指数退避： 1s, 2s, 4s, ...
            let backoff_secs = PROBE_INITIAL_BACKOFF_SECS * 2_u64.pow(attempt - 1);
            println!("探测失败，将在 {} 秒后重试...", backoff_secs);
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        }
    }

    // 如果所有重试都失败了，返回最后一次遇到的错误
    Err(last_error.unwrap_or_else(|| DispatchError::DownloadFailed("所有探测尝试均失败。".into())))
}
