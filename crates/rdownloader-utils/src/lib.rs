use regex::Regex;
use reqwest::header::CONTENT_DISPOSITION;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// --- chunk_utils ---
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkState {
    pub start: u64,
    pub end: u64,
    pub completed: bool,
}

pub fn create_chunks(total_size: u64, is_multipart: bool) -> Vec<ChunkState> {
    if !is_multipart {
        return vec![ChunkState {
            start: 0,
            end: total_size - 1,
            completed: false,
        }];
    }
    let chunk_size = 1 * 1024 * 1024; // 1MB
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < total_size {
        let end = (start + chunk_size - 1).min(total_size - 1);
        chunks.push(ChunkState {
            start,
            end,
            completed: false,
        });
        start = end + 1;
    }
    chunks
}

// --- http_utils ---
pub fn parse_content_range(range_str: &str) -> Option<u64> {
    let re = Regex::new(r"bytes \d+-\d+/(\d+)").unwrap();
    re.captures(range_str)
        .and_then(|cap| cap.get(1)?.as_str().parse().ok())
}

// --- path_utils ---
pub fn get_state_path(path: &Path) -> PathBuf {
    let mut state_path = path.as_os_str().to_owned();
    state_path.push(".rdownload");
    PathBuf::from(state_path)
}

// --- filename_utils ---
pub async fn get_filename_from_url(client: &Client, url: &str) -> Option<String> {
    let res = client.head(url).send().await.ok()?;
    if let Some(content_disposition) = res.headers().get(CONTENT_DISPOSITION) {
        let re = Regex::new(r#"filename="?([^"\s]+)"?"#).unwrap();
        if let Some(caps) = re.captures(content_disposition.to_str().ok()?) {
            return Some(caps.get(1)?.as_str().to_string());
        }
    }
    get_filename_from_path(url)
}

pub fn get_filename_from_path(url: &str) -> Option<String> {
    Path::new(url)
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
}

// --- resolve_final_path ---

/// 根据用户提供的可选输出路径和 URL，解析出最终应保存的完整文件路径。
///
/// # 逻辑:
/// 1. 如果提供了 `output_path`:
///    - 如果它指向一个已存在的目录，或以 '/' 结尾，则视为目录。
///      程序会尝试从 URL 自动推断文件名，然后与目录拼接。
///    - 否则，直接将其作为完整的文件路径。
/// 2. 如果未提供 `output_path`:
///    - 使用当前工作目录，并尝试从 URL 自动推断文件名。
///
/// 在需要创建目录的情况下，此函数会自动创建。
pub async fn resolve_final_path(
    client: &Client,
    url: &str,
    output_path: Option<PathBuf>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut final_path: PathBuf;

    if let Some(path) = output_path {
        if path.is_dir() || path.to_string_lossy().ends_with('/') {
            final_path = path;
            if !final_path.exists() {
                std::fs::create_dir_all(&final_path)?;
            }
            let filename = get_filename_from_url(client, url)
                .await
                .or_else(|| get_filename_from_path(url))
                .ok_or("无法从 URL 确定文件名，请使用 -o 指定完整路径")?;
            final_path.push(filename);
        } else {
            final_path = path;
            if let Some(parent) = final_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
    } else {
        final_path = std::env::current_dir()?;
        let filename = get_filename_from_url(client, url)
            .await
            .or_else(|| get_filename_from_path(url))
            .ok_or("无法从 URL 确定文件名，请使用 -o 指定完整路径")?;
        final_path.push(filename);
    }

    Ok(final_path)
}
