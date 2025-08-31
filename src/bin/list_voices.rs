// src/bin/list_voices.rs
use tts::Tts;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tts = Tts::default()?;
    let voices = tts.voices()?;

    println!("=============================================");
    println!("           系统中所有可用的TTS语音           ");
    println!("=============================================");

    for voice in voices {
        println!("  名称: {}", voice.name());
        println!("  语言: {}", voice.language());
        println!("---------------------------------------------");
    }

    Ok(())
}