use clap::Parser;
use rdownloader::download;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 要下载的文件的 URL
    url: String,

    /// 输出路径 (可以是一个完整的文件路径，或一个目录)
    #[arg(short, long, value_name = "PATH")]
    output: Option<String>,

    /// 指定 log4rs 配置文件的路径
    #[arg(short = 'c', long, value_name = "FILE")]
    log_conf: Option<PathBuf>,
}

fn setup_logger(config_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.unwrap_or_else(|| PathBuf::from("log4rs.yaml"));
    log4rs::init_file(path, Default::default())?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // 初始化日志记录器
    if let Err(e) = setup_logger(args.log_conf) {
        eprintln!("错误：无法初始化日志记录器: {}. 日志功能将不可用。", e);
    }

    // --- 调用高级 API ---
    // 所有复杂的逻辑都被封装在 rdownloader::download 函数中
    match download(&args.url, args.output).await {
        Ok(_) => log::info!("\n下载任务成功完成!"),
        Err(e) => log::error!("\n下载任务失败: {:?}", e),
    }

    Ok(())
}
