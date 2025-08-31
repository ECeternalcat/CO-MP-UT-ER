// src/tts_engine.rs

use crate::config::Config;
use tts::Tts;
use log::{info, warn, error};
use std::error::Error;

pub struct TtsEngine {
    // Tts 实例现在是唯一的字段
    tts: Tts,
}

impl TtsEngine {
    /// 创建一个新的 TtsEngine 实例。
    /// 构造函数现在接收一个对已加载配置的引用，而不是自己加载它。
    /// 这样可以更好地分离关注点。
    pub fn new(config: &Config) -> Result<Self, Box<dyn Error>> {
        // 1. 初始化 tts 库
        let mut tts = Tts::default()?;
        
        // 2. 检查配置中是否指定了自定义语音
        if let Some(voice_name) = &config.custom_voice {
            info!("配置文件中指定了语音: '{}'。正在尝试设置...", voice_name);
            
            // 尝试在系统中找到该名称的语音
            let voice_found = tts.voices()?.into_iter().find(|v| v.name() == voice_name.as_str());
            
            if let Some(voice) = voice_found {
                // 如果找到了，就设置它
                if tts.set_voice(&voice).is_ok() {
                    info!("成功将语音设置为: {}", voice.name());
                } else {
                    // 这种情况很少见，但为了健壮性还是处理一下
                    error!("尝试设置语音 '{}' 失败，将使用默认语音。", voice_name);
                }
            } else {
                // 如果在系统中找不到配置的语音，发出警告
                warn!("未在系统中找到名为 '{}' 的语音，将使用默认语音。", voice_name);
            }
        } else {
            // 如果配置中没有指定语音，则直接使用系统默认语音
            info!("未使用自定义语音，将使用系统默认语音。");
        }

        Ok(TtsEngine { tts })
    }

    /// 播报指定的文本。
    /// 这个函数保持不变。
    pub fn speak(&mut self, text: &str) -> Result<(), Box<dyn Error>> {
        self.tts.speak(text, false)?;
        Ok(())
    }
    
    /// --- 新增 ---
    /// 获取系统中所有可用语音的名称列表。
    /// 这个方法是为设置窗口的下拉列表准备数据的。
    pub fn list_available_voices(&self) -> Result<Vec<String>, Box<dyn Error>> {
        let voices = self.tts.voices()?;
        Ok(voices.iter().map(|v| v.name().to_string()).collect())
    }

    /// --- 新增 ---
    /// 在运行时动态设置要使用的语音。
    /// 当用户在设置窗口中选择一个新语音并点击“OK”时，会调用此方法。
    pub fn set_voice(&mut self, voice_name: &str) -> Result<(), Box<dyn Error>> {
        // 在所有可用语音中查找与给定名称匹配的 Voice 对象
        let voice_to_set = self.tts.voices()?
            .into_iter()
            .find(|v| v.name() == voice_name);
            
        if let Some(voice) = voice_to_set {
            // 如果找到，就应用它
            self.tts.set_voice(&voice)?;
            info!("语音已动态切换为: {}", voice.name());
            Ok(())
        } else {
            // 如果没找到，返回一个错误，这样调用者（设置窗口）就可以知道操作失败了
            error!("尝试动态切换语音失败，未找到名为 '{}' 的语音", voice_name);
            Err(format!("未找到名为 '{}' 的语音", voice_name).into())
        }
    }
}