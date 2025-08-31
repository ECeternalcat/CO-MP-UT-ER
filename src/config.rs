// src/config.rs

use serde::Deserialize;
use std::fs;

// 定义配置文件的结构
#[derive(Deserialize, Default)]
pub struct Config {
    pub custom_voice: Option<String>,
}

impl Config {
    // 加载配置文件。如果文件不存在或解析失败，则返回默认配置。
    pub fn load() -> Self {
        match fs::read_to_string("config.json") {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_else(|e| {
                    eprintln!("警告: 解析 config.json 失败: {}. 将使用默认配置。", e);
                    Config::default()
                })
            },
            Err(_) => {
                // 文件不存在是正常情况，直接返回默认值
                Config::default()
            }
        }
    }
}