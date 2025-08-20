use regex::Regex;
use reqwest::header::CONTENT_DISPOSITION;
use reqwest::Client;
use std::path::{Path, PathBuf};

// --- 模块定义 (修正后) ---

// 将所有工具函数组织在不同的模块中

pub mod path_utils {
    use super::*;
    pub fn get_state_path(path: &Path) -> PathBuf {
        path.with_extension(format!(
            "{}.rdownload",
            path.extension().and_then(|s| s.to_str()).unwrap_or("")
        ))
    }
}

pub mod chunk_utils {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct ChunkState {
        pub start: u64,
        pub end: u64,
        pub completed: bool,
    }

    pub fn create_chunks(total_size: u64, is_multipart: bool) -> Vec<ChunkState> {
        const TARGET_CHUNK_SIZE: u64 = 10 * 1024 * 1024;
        const MAX_CHUNKS: u64 = 16;

        if !is_multipart || total_size < TARGET_CHUNK_SIZE {
            return vec![ChunkState {
                start: 0,
                end: total_size.saturating_sub(1),
                completed: false,
            }];
        }

        let mut num_chunks = (total_size / TARGET_CHUNK_SIZE).max(1);
        num_chunks = num_chunks.min(MAX_CHUNKS);

        let chunk_size = (total_size + num_chunks - 1) / num_chunks;

        (0..num_chunks)
            .map(|i| {
                let start = i * chunk_size;
                let end = (start + chunk_size - 1).min(total_size - 1);
                ChunkState {
                    start,
                    end,
                    completed: false,
                }
            })
            .collect()
    }
}

pub mod http_utils {
    use super::*;
    pub fn parse_content_range(header_value: &str) -> Option<u64> {
        let re = Regex::new(r"/(\d+)").unwrap();
        re.captures(header_value)
            .and_then(|cap| cap.get(1))
            .and_then(|m| m.as_str().parse().ok())
    }
}

pub mod filename_utils {
    use super::*;

    pub async fn get_filename_from_url(client: &Client, url: &str) -> Option<String> {
        let res = client.head(url).send().await.ok()?;
        res.headers()
            .get(CONTENT_DISPOSITION)?
            .to_str()
            .ok()
            .and_then(parse_filename_from_disposition)
    }

    fn parse_filename_from_disposition(header_value: &str) -> Option<String> {
        header_value
            .split(';')
            .find(|part| part.trim().starts_with("filename="))
            .map(|part| {
                part.trim()
                    .strip_prefix("filename=")
                    .unwrap()
                    .trim_matches('"')
                    .to_string()
            })
    }

    pub fn get_filename_from_path(url: &str) -> Option<String> {
        Path::new(url)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
    }
}
