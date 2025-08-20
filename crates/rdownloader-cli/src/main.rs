use clap::Parser;
use rdownloader_dispatcher::dispatch;
use rdownloader_utils::filename_utils::{get_filename_from_path, get_filename_from_url};
use reqwest::Client;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 要下载的文件的 URL
    url: String,

    /// 输出路径 (可以是一个完整的文件路径，或一个目录)
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// 指定 log4rs 配置文件的路径
    #[arg(short = 'c', long, value_name = "FILE")]
    log_conf: Option<PathBuf>,
}

// 修正 setup_logger 的错误处理
fn setup_logger(config_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.unwrap_or_else(|| PathBuf::from("log4rs.yaml"));
    log4rs::init_file(path, Default::default())?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // 初始化日志记录器
    if let Err(e) = setup_logger(args.log_conf.clone()) {
        eprintln!("错误：无法初始化日志记录器: {}. 日志功能将不可用。", e);
    }

    let client = Client::new();

    // --- 路径和文件名处理 ---
    let mut final_path: PathBuf;
    let output_path = args.output.clone();

    if let Some(path) = output_path {
        if path.is_dir() || path.to_string_lossy().ends_with('/') {
            final_path = path;
            if !final_path.exists() {
                std::fs::create_dir_all(&final_path)?;
            }
            let filename = get_filename_from_url(&client, &args.url)
                .await
                .or_else(|| get_filename_from_path(&args.url))
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
        let filename = get_filename_from_url(&client, &args.url)
            .await
            .or_else(|| get_filename_from_path(&args.url))
            .ok_or("无法从 URL 确定文件名，请使用 -o 指定完整路径")?;
        final_path.push(filename);
    }

    // --- 调用下载 ---
    log::info!("准备下载: {}", &args.url);
    log::info!("保存路径: {}", final_path.display());

    match dispatch(&client, &args.url, &final_path).await {
        Ok(_) => log::info!("\n下载任务成功完成!"),
        Err(e) => log::error!("\n下载任务失败: {:?}", e),
    }

    Ok(())
}
