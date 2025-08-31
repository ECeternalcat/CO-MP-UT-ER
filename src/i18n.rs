// src/i18n.rs

use serde_json::Value;
use std::collections::HashMap;
use std::fs;

pub struct I18nManager {
    translations: HashMap<String, String>,
}

impl I18nManager {
    pub fn new(locale: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let path = format!("locales/{}.json", locale);
        let data = fs::read_to_string(path)?;
        let parsed: Value = serde_json::from_str(&data)?;
        
        let mut translations = HashMap::new();
        if let Value::Object(map) = parsed {
            for (key, value) in map {
                if let Value::String(s) = value {
                    translations.insert(key, s);
                }
            }
        }

        Ok(I18nManager { translations })
    }

    pub fn get_text(&self, key: &str) -> Option<String> {
        self.translations.get(key).cloned()
    }

    pub fn get_text_with_param(&self, key: &str, param_key: &str, param_value: &str) -> Option<String> {
        self.translations.get(key).map(|s| {
            s.replace(&format!("{{{}}}", param_key), param_value)
        })
    }
}