use rdownloader_dispatcher::{dispatch, DispatchError};
use rdownloader_utils::resolve_final_path;
use reqwest::Client;
use std::path::PathBuf;

// 定义一个公开的、更简洁的错误类型，对用户隐藏内部复杂的错误细节
#[derive(Debug)]
pub enum DownloadError {
    Dispatch(DispatchError),
    Path(Box<dyn std::error::Error>),
}

impl From<DispatchError> for DownloadError {
    fn from(err: DispatchError) -> Self {
        DownloadError::Dispatch(err)
    }
}

impl From<Box<dyn std::error::Error>> for DownloadError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        DownloadError::Path(err)
    }
}

/// rDownloader 的高级公共 API。
/// 
/// 封装了所有内部逻辑，提供一个简单的函数来启动下载。
/// 
/// # 参数
/// * `url`: 要下载的文件的 URL。
/// * `output`: 一个可选的输出路径。可以是目录，也可以是完整的文件路径。
///           如果为 `None`，则下载到当前工作目录。
pub async fn download(url: &str, output: Option<String>) -> Result<(), DownloadError> {
    let client = Client::new();
    
    // 将 Option<String> 转换为 Option<PathBuf>
    let output_path_buf = output.map(PathBuf::from);

    // 解析最终的保存路径
    let final_path = resolve_final_path(&client, url, output_path_buf).await?;

    log::info!("准备下载: {}", url);
    log::info!("保存路径: {}", final_path.display());

    // 调用调度器执行下载
    dispatch(&client, url, &final_path).await?;

    Ok(())
}