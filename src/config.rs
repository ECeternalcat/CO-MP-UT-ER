// src/config.rs

use serde::{Deserialize, Serialize}; // --- 修改: 增加 Serialize ---
use std::fs;
use std::path::PathBuf;
use log::warn;

// --- 新增: 帮助函数，用于定位配置文件 ---
// 将配置文件放在 AppData 目录是更好的实践，但为了简单起见，我们暂时保留在程序目录
fn get_config_path() -> PathBuf {
    PathBuf::from("config.json")
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Config {
    pub custom_voice: Option<String>,
    pub auto_start: bool,
    pub language: Option<String>, // --- 新增: 用于存储语言选择，例如 "en", "zh", "ja" ---
}

impl Default for Config {
    fn default() -> Self {
        Self {
            custom_voice: None,
            auto_start: false,
            language: None, // --- 新增: 默认值为 None，表示“自动检测” ---
        }
    }
}




impl Config {
    pub fn load() -> Self {
        match fs::read_to_string(get_config_path()) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_else(|e| {
                    warn!("警告: 解析 config.json 失败: {}. 将使用默认配置。", e);
                    Config::default()
                })
            },
            Err(_) => {
                // 文件不存在是正常情况，直接返回默认值
                Config::default()
            }
        }
    }

    // --- 新增: 保存配置到文件的函数 ---
    pub fn save(&self) -> Result<(), std::io::Error> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(get_config_path(), content)
    }
}