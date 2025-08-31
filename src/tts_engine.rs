// src/tts_engine.rs

use crate::config::Config;
use tts::{Tts, Voice};

pub struct TtsEngine {
    tts: Tts,
}

impl TtsEngine {
    pub fn new(locale: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut tts = Tts::default()?;
        let available_voices = tts.voices()?;

        let config = Config::load();
        let mut voice_set = false;

        if let Some(custom_voice_name) = config.custom_voice {
            // FIX: 在 v.name() 前面加上 `&`，将比较变为 `&String == &String`
            if let Some(voice) = available_voices.iter().find(|v| &v.name() == &custom_voice_name) {
                println!("检测到自定义配置，使用语音: {}", voice.name());
                tts.set_voice(voice)?;
                voice_set = true;
            } else {
                eprintln!("警告: 在 config.json 中指定的语音 '{}' 未在系统中找到。", custom_voice_name);
            }
        }

        if !voice_set {
            if let Some(voice) = available_voices.iter().find(|v| v.language().starts_with(locale)) {
                println!("自动匹配本地化语言 '{}'，使用语音: {}", locale, voice.name());
                tts.set_voice(voice)?;
                voice_set = true;
            }
        }

        if !voice_set {
            if let Some(voice) = available_voices.iter().find(|v| v.language().starts_with("en")) {
                println!("未找到匹配的语音，回退到英文语音: {}", voice.name());
                tts.set_voice(voice)?;
                voice_set = true;
            }
        }

        if !voice_set {
            println!("警告: 未找到任何合适的语音，将使用系统默认语音。");
        }

        Ok(TtsEngine { tts })
    }

    pub fn speak(&mut self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.tts.speak(text, false)?;
        Ok(())
    }
}