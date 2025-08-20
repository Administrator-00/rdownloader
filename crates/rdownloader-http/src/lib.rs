use futures_util::{stream, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use log::debug;
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rdownloader_utils::chunk_utils::{create_chunks, ChunkState};
use rdownloader_utils::path_utils::get_state_path;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DownloadState {
    url: String,
    total_size: u64,
    etag: Option<String>,
    chunks: Vec<ChunkState>,
}

#[derive(Debug)]
pub enum DownloadError {
    NetworkError(reqwest::Error),
    FileError(std::io::Error),
    HttpError(reqwest::StatusCode),
    SpawnError(tokio::task::JoinError),
    JsonError(serde_json::Error),
    StateError(String),
    ChunkDownloadFailed,
    ContentTypeMismatch, // 当数据块的 Content-Type 与期望不符时返回
}

impl From<serde_json::Error> for DownloadError {
    fn from(err: serde_json::Error) -> Self {
        DownloadError::JsonError(err)
    }
}
impl From<reqwest::Error> for DownloadError {
    fn from(err: reqwest::Error) -> Self {
        DownloadError::NetworkError(err)
    }
}
impl From<std::io::Error> for DownloadError {
    fn from(err: std::io::Error) -> Self {
        DownloadError::FileError(err)
    }
}
impl From<tokio::task::JoinError> for DownloadError {
    fn from(err: tokio::task::JoinError) -> Self {
        DownloadError::SpawnError(err)
    }
}

pub async fn download_multipart(
    client: &Client,
    url: &str,
    path: &Path,
    total_size: u64,
    etag: Option<String>,
    content_type: Option<String>,
) -> Result<(), DownloadError> {
    run_download(client, url, path, total_size, etag, content_type, true).await
}

pub async fn download_sequential(
    client: &Client,
    url: &str,
    path: &Path,
    total_size: Option<u64>,
    etag: Option<String>,
    content_type: Option<String>,
) -> Result<(), DownloadError> {
    if let Some(size) = total_size {
        // 如果文件大小已知，则使用支持断点续传的 run_download
        run_download(client, url, path, size, etag, content_type, false).await
    } else {
        // --- 文件大小未知：执行简单的流式下载 ---
        // 这种模式下不支持断点续传
        println!("文件大小未知，将执行简单的流式下载 (不支持断点续传)。");
        let mut res = client.get(url).send().await?;
        if !res.status().is_success() {
            return Err(DownloadError::HttpError(res.status()));
        }

        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template(
                    "{spinner:.green} [{elapsed_precise}] {bytes_per_sec} - {bytes} downloaded",
                )
                .unwrap(),
        );
        pb.enable_steady_tick(Duration::from_millis(100));

        let mut file = File::create(path)?;
        let mut downloaded: u64 = 0;

        while let Some(chunk) = res.chunk().await? {
            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }

        pb.finish_with_message("下载完成");
        Ok(())
    }
}

async fn run_download(
    client: &Client,
    url: &str,
    path: &Path,
    total_size: u64,
    current_etag: Option<String>,
    expected_content_type: Option<String>,
    is_multipart: bool,
) -> Result<(), DownloadError> {
    let state_path = get_state_path(path);
    let mut state: DownloadState;
    let mut completed_bytes = 0;

    if state_path.exists() {
        let mut file = File::open(&state_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        state = serde_json::from_str(&contents)?;
        // 核心校验：如果文件大小、URL或ETag任意一个不匹配，则判定为无效状态，从头开始。
        if state.total_size != total_size || &state.url != url || state.etag != current_etag {
            if state_path.exists() {
                std::fs::remove_file(&state_path)?;
            }
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            let chunks = create_chunks(total_size, is_multipart);
            state = DownloadState {
                total_size,
                chunks,
                url: url.to_string(),
                etag: current_etag,
            };
            let file = File::create(&path)?;
            file.set_len(total_size)?;
        } else {
            for chunk in &state.chunks {
                if chunk.completed {
                    completed_bytes += chunk.end - chunk.start + 1;
                }
            }
        }
    } else {
        let chunks = create_chunks(total_size, is_multipart);
        state = DownloadState {
            total_size,
            chunks,
            url: url.to_string(),
            etag: current_etag,
        };
        let file = File::create(&path)?;
        // 预分配文件大小，避免后续多线程写入时频繁调整文件大小
        file.set_len(total_size)?;
    }

    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar().template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})").unwrap().progress_chars("->-"));
    pb.inc(completed_bytes);
    pb.enable_steady_tick(Duration::from_millis(100));

    let state = Arc::new(Mutex::new(state));

    let tasks = stream::iter(state.lock().unwrap().chunks.clone().into_iter().enumerate())
        .filter(|(_, chunk)| futures_util::future::ready(!chunk.completed))
        .map(|(i, chunk)| {
            let client = client.clone();
            let url = url.to_string();
            let path = path.to_path_buf();
            let state_path = state_path.clone();
            let state_arc = Arc::clone(&state);
            let pb = pb.clone();
            let expected_content_type = expected_content_type.clone();

            tokio::spawn(async move {
                let range_header = format!("bytes={}-{}", chunk.start, chunk.end);
                let res = client
                    .get(&url)
                    .header("Range", range_header)
                    .send()
                    .await?;

                // 必须是 206 Partial Content (多线程) 或 200 OK (单线程) 才是有效响应
                if res.status() != 206 && res.status() != 200 {
                    return Err(DownloadError::HttpError(res.status()));
                }

                // --- 内容校验 ---
                // 检查每个块的 Content-Type 是否与探测时获得的一致。
                // 这是为了防止服务器返回 206 状态码但响应体是 HTML 错误页面的情况。
                let chunk_content_type = res
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                if chunk_content_type != expected_content_type {
                    return Err(DownloadError::ContentTypeMismatch);
                }

                let data = res.bytes().await?;

                // 将文件写入操作移入 spawn_blocking，因为它是一个同步阻塞操作
                tokio::task::spawn_blocking(move || {
                    let mut file = OpenOptions::new().write(true).open(&path)?;
                    file.seek(std::io::SeekFrom::Start(chunk.start))?;
                    file.write_all(&data)?;

                    // 更新状态文件，这是一个原子操作
                    let mut state_lock = state_arc.lock().unwrap();
                    state_lock.chunks[i].completed = true;
                    let state_json = serde_json::to_string_pretty(&*state_lock)?;
                    let mut state_file = File::create(&state_path)?;
                    state_file.write_all(state_json.as_bytes())?;

                    pb.inc(data.len() as u64);
                    Ok::<(), DownloadError>(())
                })
                .await??;

                Ok::<(), DownloadError>(())
            })
        })
        .buffer_unordered(if is_multipart { 8 } else { 1 });

    // --- 结果处理 ---
    // 等待所有下载任务完成，并检查是否有任何一个任务失败。
    // 这是为了防止静默的数据损坏：即使只有一个块失败，整个下载也必须被视为失败。
    let results: Vec<_> = tasks.collect().await;
    let mut has_error = false;
    for result in results {
        if let Err(e) = result {
            debug!("一个下载任务失败: {:?}", e);
            has_error = true;
        }
    }

    if has_error {
        eprintln!("\n由于部分数据块下载失败，下载未完成。请重新运行命令以续传。");
        return Err(DownloadError::ChunkDownloadFailed);
    }

    // 只有当所有块都成功下载后，才删除状态文件，标志着整个任务的成功完成
    pb.finish_with_message("下载完成");
    std::fs::remove_file(&state_path)?;
    Ok(())
}
